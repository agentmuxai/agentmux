// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The accessibility snapshot an agent reads a browser pane through, and the
//! element references it acts with
//! (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §4.1).
//!
//! Pure functions over CDP `Accessibility.getFullAXTree` output plus a few
//! `DOM.describeNode` facts, so the format and the secret-field rule are
//! unit-testable without a browser. The routes in `routes.rs` gather the
//! inputs and act on the references. `find` reads the same tree but returns
//! only the lines of elements named like what the agent asked for.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::{json, Value};

pub use super::secret::{is_secret, secret_reason};

/// Snapshot text is capped so one page can't flood an agent's context.
pub const MAX_SNAPSHOT_BYTES: usize = 40_000;

/// Roles an agent can act on: each gets a reference.
const INTERACTIVE_ROLES: &[&str] = &[
    "button", "link", "textbox", "searchbox", "combobox", "listbox", "option", "checkbox", "radio",
    "switch", "slider", "spinbutton", "menuitem", "menuitemcheckbox", "menuitemradio", "tab",
    "treeitem", "textarea", "PopUpButton", "ComboBoxMenuButton", "ComboBoxSelect",
    // A <details> section's <summary>: opening it is often the only way to
    // read a review page.
    "DisclosureTriangle", "DisclosureTriangleGrouped",
    // Native date, time, month, week and datetime-local inputs: `fill` sets them.
    "Date", "DateTime", "InputTime",
];

/// The roles of native date and time inputs. Their segments (day, month,
/// year spin buttons) aren't shown: the control is filled whole.
const DATE_ROLES: &[&str] = &["Date", "DateTime", "InputTime"];

/// Roles that carry no meaning of their own; their children are shown in
/// their place.
const TRANSPARENT_ROLES: &[&str] = &[
    "generic", "none", "presentation", "InlineTextBox", "LineBreak", "RootWebArea", "WebArea",
    "Section", "GenericContainer", "Ignored", "IgnoredRole", "LabelText", "Legend", "MenuListPopup",
];

/// Text inside these is already shown: a label's or legend's text is the
/// accessible name of what it labels, and a field's text is its `value=`.
const TEXT_SHOWN_ELSEWHERE: &[&str] = &[
    "LabelText", "Legend", "textbox", "searchbox", "combobox", "spinbutton", "slider", "textarea",
];

/// What `DOM.describeNode` says about an element, for the few things the
/// accessibility tree doesn't: secret fields and file inputs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomFacts {
    pub local_name: String,
    pub attrs: HashMap<String, String>,
    /// What a person reads as the field's name: its label's text, or its
    /// accessible name. Empty when unknown.
    pub label: String,
}

impl DomFacts {
    /// From a `DOM.describeNode` result's `node` object.
    pub fn from_described(node: &Value) -> Self {
        let local_name = node.get("localName").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
        let mut attrs = HashMap::new();
        if let Some(list) = node.get("attributes").and_then(|v| v.as_array()) {
            for pair in list.chunks(2) {
                if let [k, v] = pair {
                    if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                        attrs.insert(k.to_ascii_lowercase(), v.to_string());
                    }
                }
            }
        }
        // Only the focused-field script reports a label in the node it returns.
        let label = node.get("label").and_then(|v| v.as_str()).unwrap_or("").to_string();
        Self { local_name, attrs, label }
    }

    /// The same facts with the field's accessible name filled in.
    pub fn with_label(mut self, label: &str) -> Self {
        self.label = label.to_string();
        self
    }

    pub(super) fn attr(&self, name: &str) -> &str {
        self.attrs.get(name).map(String::as_str).unwrap_or("")
    }

    pub fn is_file_input(&self) -> bool {
        self.local_name == "input" && self.attr("type").eq_ignore_ascii_case("file")
    }
}

/// The limits a field puts on what can be typed, as the browser enforces
/// them: ` [maxlength=350]`, ` [minlength=2]`, ` [pattern="[0-9]{9}"]`.
/// Empty when it has none. Shown so an agent writes within them instead of
/// finding out from a cut-off value.
pub fn limits_of(f: &DomFacts) -> String {
    let mut out = String::new();
    if !takes_length_limits(f) {
        return out;
    }
    for key in ["maxlength", "minlength"] {
        if let Some(n) = f.attrs.get(key).and_then(|v| v.trim().parse::<u64>().ok()) {
            out.push_str(&format!(" [{key}={n}]"));
        }
    }
    if let Some(p) = f.attrs.get("pattern").filter(|p| !p.is_empty() && f.local_name == "input") {
        out.push_str(&format!(" [pattern={}]", quote(p)));
    }
    out
}

/// The `maxlength` of a field, if it has a valid one the browser enforces.
pub fn max_length(f: &DomFacts) -> Option<usize> {
    if !takes_length_limits(f) {
        return None;
    }
    f.attrs.get("maxlength").and_then(|v| v.trim().parse::<usize>().ok())
}

/// Whether the browser applies `maxlength`, `minlength` and `pattern` to this
/// field: a `textarea`, or an `input` of a text-like type (an unknown type is
/// `text`). Number, date, time and the other non-text inputs ignore them.
fn takes_length_limits(f: &DomFacts) -> bool {
    match f.local_name.as_str() {
        "textarea" => true,
        "input" => {
            let ty = f.attrs.get("type").map(|t| t.trim().to_ascii_lowercase()).unwrap_or_default();
            !matches!(
                ty.as_str(),
                "number" | "date" | "time" | "datetime-local" | "month" | "week" | "range" | "color" | "checkbox"
                    | "radio" | "file" | "hidden" | "submit" | "image" | "reset" | "button"
            )
        }
        _ => false,
    }
}

/// A rendered snapshot and the references it handed out (`e1` → backend node id).
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub text: String,
    pub refs: Vec<(String, i64)>,
    /// For each reference inside a frame, the frame's ordinal (`f2e5` → 2).
    /// References in the top frame are not listed.
    pub ref_frames: Vec<(String, usize)>,
    pub truncated: bool,
}

/// One iframe's accessibility tree, to be shown under the `<iframe>` element
/// that holds it (spec §4.2). Built by the route that read the tree.
#[derive(Debug, Clone, Default)]
pub struct FrameTree {
    /// `f{ordinal}` in references; 1.. in document order. The top frame is 0.
    pub ordinal: usize,
    /// The ordinal of the frame the `<iframe>` element is in (0: the top).
    pub parent: usize,
    /// The `<iframe>` element's backend node id, in the parent's tree.
    pub owner_backend: Option<i64>,
    pub origin: String,
    pub cross_origin: bool,
    pub nodes: Vec<Value>,
    pub facts: HashMap<i64, DomFacts>,
    /// Why the frame's contents aren't shown (it was over the limit, or its
    /// tree couldn't be read), shown on its line instead of a silent bare one.
    pub note: Option<String>,
}

fn str_of<'a>(node: &'a Value, key: &str) -> &'a str {
    node.get(key).and_then(|v| v.get("value")).and_then(|v| v.as_str()).unwrap_or("")
}

fn value_text(node: &Value) -> Option<String> {
    let v = node.get("value")?.get("value")?;
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn prop<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    node.get("properties")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
        .and_then(|p| p.get("value"))
        .and_then(|v| v.get("value"))
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "true",
        _ => false,
    }
}

fn quote(s: &str) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    // Hide a card, ID or bank number, a security or one-time code and a token the page prints in
    // plain text (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §5.1). The whole text is
    // masked first and cut to 2000 characters after, so nothing the cut falls inside can show its
    // first characters; the 160-character limit below then applies to the masked text.
    let one_line = agentmux_common::pii_mask::mask_bounded(&one_line, 2000);
    let clipped: String = if one_line.chars().count() > 160 {
        one_line.chars().take(157).collect::<String>() + "…"
    } else {
        one_line
    };
    serde_json::to_string(&clipped).unwrap_or_else(|_| "\"\"".to_string())
}

/// Roles that carry a typed value, which the snapshot shows only once the
/// element's DOM facts prove it isn't a secret field.
pub const VALUE_ROLES: &[&str] =
    &["textbox", "searchbox", "combobox", "spinbutton", "slider", "textarea", "Date", "DateTime", "InputTime"];

/// The nodes of the subtree rooted at the element with `backend_id`, in
/// document order (empty if it isn't in `nodes`).
pub fn subtree(nodes: &[Value], backend_id: i64) -> Vec<&Value> {
    let by_id: HashMap<&str, &Value> =
        nodes.iter().filter_map(|n| Some((n.get("nodeId")?.as_str()?, n))).collect();
    let Some(root) = nodes.iter().find(|n| n.get("backendDOMNodeId").and_then(|v| v.as_i64()) == Some(backend_id)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        out.push(n);
        if let Some(kids) = n.get("childIds").and_then(|v| v.as_array()) {
            for k in kids.iter().rev() {
                if let Some(c) = k.as_str().and_then(|id| by_id.get(id)) {
                    stack.push(c);
                }
            }
        }
    }
    out
}

/// A checkbox or radio input the page hides (`display: none`) behind a
/// `<label>` drawn as the box, a common styling pattern. The accessibility
/// tree leaves such an input out, so it would get no reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenChoice {
    pub input_backend: i64,
    pub label_backend: i64,
    pub radio: bool,
    /// The label's text, or the input's own aria-label.
    pub name: String,
    pub checked: bool,
    pub disabled: bool,
}

/// Add each hidden choice to `nodes` as a checkbox or radio node, named by its
/// label, carrying the input's backend id, first inside its label's node (or
/// last in the root when the label isn't in the tree). It then gets a line and
/// a reference like any checkbox; `check` and `click` reach the input through
/// its label (`act.rs`'s `CLICK_POINT`).
pub fn add_hidden_choices(nodes: &mut Vec<Value>, choices: &[HiddenChoice]) {
    for (i, c) in choices.iter().enumerate() {
        let id = format!("agentmux-hidden-choice-{i}");
        let holder = nodes
            .iter()
            .position(|n| n.get("backendDOMNodeId").and_then(|v| v.as_i64()) == Some(c.label_backend));
        let (holder, first) = match holder {
            Some(h) => (h, true),
            None => match nodes.iter().position(|n| n.get("parentId").is_none()) {
                Some(root) => (root, false),
                None => continue,
            },
        };
        let parent = nodes[holder].get("nodeId").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let kids = nodes[holder]
            .as_object_mut()
            .map(|o| o.entry("childIds").or_insert_with(|| json!([])));
        if let Some(Value::Array(kids)) = kids {
            if first {
                kids.insert(0, json!(id));
            } else {
                kids.push(json!(id));
            }
        }
        let mut properties = vec![json!({ "name": "checked", "value": { "type": "tristate", "value": if c.checked { "true" } else { "false" } } })];
        if c.disabled {
            properties.push(json!({ "name": "disabled", "value": { "type": "boolean", "value": true } }));
        }
        nodes.push(json!({
            "nodeId": id,
            "parentId": parent,
            "ignored": false,
            "role": { "type": "role", "value": if c.radio { "radio" } else { "checkbox" } },
            "name": { "type": "computedString", "value": c.name },
            "properties": properties,
            "backendDOMNodeId": c.input_backend,
            "childIds": [],
        }));
    }
}

/// Render `Accessibility.getFullAXTree`'s `nodes` as an indented list, one
/// line per meaningful node, with a reference on each interactive one.
/// `scope_backend_id` narrows the snapshot to the subtree of that element.
/// References are named `{ref_prefix}e1`, `{ref_prefix}e2`, …
///
/// A value-bearing field without DOM facts (`facts` has no entry: the
/// lookup was capped or failed) is treated as possibly secret: its value is
/// hidden. Fail closed.
pub fn render(
    nodes: &[Value],
    facts: &HashMap<i64, DomFacts>,
    scope_backend_id: Option<i64>,
    ref_prefix: &str,
) -> Snapshot {
    render_with_frames(nodes, facts, scope_backend_id, ref_prefix, &[])
}

/// [`render`], with each iframe's tree shown under its `<iframe>` element.
/// References inside frame `N` are named `fNe1`, `fNe2`, …
pub fn render_with_frames(
    nodes: &[Value],
    facts: &HashMap<i64, DomFacts>,
    scope_backend_id: Option<i64>,
    ref_prefix: &str,
    frames: &[FrameTree],
) -> Snapshot {
    let by_id: HashMap<&str, &Value> =
        nodes.iter().filter_map(|n| Some((n.get("nodeId")?.as_str()?, n))).collect();
    let root = match scope_backend_id {
        Some(b) => nodes.iter().find(|n| n.get("backendDOMNodeId").and_then(|v| v.as_i64()) == Some(b)),
        None => nodes
            .iter()
            .find(|n| n.get("parentId").is_none())
            .or_else(|| nodes.first()),
    };
    let mut out = Snapshot { text: String::new(), refs: Vec::new(), ref_frames: Vec::new(), truncated: false };
    let zones = super::shown_once::copy_zones(nodes);
    if let Some(root) = root {
        // A scope inside a zone is still in it: its secret stays masked.
        let in_zone = super::shown_once::under_zone(root, &by_id, &zones);
        walk(root, &by_id, facts, &zones, in_zone, ref_prefix, 0, "", false, 0, frames, &mut out);
    }
    if out.truncated {
        out.text.push_str("- … snapshot truncated at 40 KB: use find (a role and name), or pass `scope` (a reference, or a role and quoted name) to read one part of the page\n");
    }
    out
}

fn walk(
    node: &Value,
    by_id: &HashMap<&str, &Value>,
    facts: &HashMap<i64, DomFacts>,
    zones: &HashSet<&str>,
    in_zone: bool,
    ref_prefix: &str,
    depth: usize,
    parent_name: &str,
    hide_text: bool,
    frame: usize,
    frames: &[FrameTree],
    out: &mut Snapshot,
) {
    if out.truncated {
        return;
    }
    let role = str_of(node, "role");
    let name = str_of(node, "name").trim();
    let backend = node.get("backendDOMNodeId").and_then(|v| v.as_i64());
    let transparent = is_transparent(node, role, name, hide_text, parent_name);
    let in_zone = in_zone || super::shown_once::starts_zone(node, zones);

    let mut child_depth = depth;
    let mut name_for_children = parent_name;
    if !transparent {
        // The reference goes into the table only once its line is known to
        // fit: a line cut by the cap would hand out a reference the agent
        // never sees.
        let r = (takes_ref(role, backend, facts) || (backend.is_some() && masks_shown_once(node, role, name, in_zone)))
            .then(|| next_ref(out, ref_prefix, frame));
        let line = line_for(node, role, name, backend, facts, r.as_deref(), frame, frames, in_zone);
        let indent = "  ".repeat(depth);
        if out.text.len() + indent.len() + line.len() + 1 > MAX_SNAPSHOT_BYTES {
            out.truncated = true;
            return;
        }
        if let (Some(r), Some(b)) = (r, backend) {
            hand_out(out, r, b, frame);
        }
        out.text.push_str(&indent);
        out.text.push_str(&line);
        out.text.push('\n');
        child_depth = depth + 1;
        name_for_children = name;
        // An iframe's own tree goes under its element.
        if let Some(inner) = frame_of_owner(frames, frame, role, backend) {
            if inner.note.is_none() {
                render_frame(inner, frames, child_depth, out);
            }
        }
    }
    if DATE_ROLES.contains(&role) && !transparent {
        return;
    }
    if let Some(children) = node.get("childIds").and_then(|v| v.as_array()) {
        for c in children {
            if let Some(child) = c.as_str().and_then(|id| by_id.get(id)) {
                walk(child, by_id, facts, zones, in_zone, ref_prefix, child_depth, name_for_children, hide_text || TEXT_SHOWN_ELSEWHERE.contains(&role), frame, frames, out);
            }
        }
    }
}

/// The frame held by the `<iframe>` element `backend` (with accessible role
/// `role`) in frame `frame`, if one was read.
fn frame_of_owner<'a>(frames: &'a [FrameTree], frame: usize, role: &str, backend: Option<i64>) -> Option<&'a FrameTree> {
    if !is_iframe_role(role) {
        return None;
    }
    let b = backend?;
    frames.iter().find(|f| f.parent == frame && f.owner_backend == Some(b))
}

/// An `<iframe>` element's accessible role. A frame whose element says
/// `role="presentation"` (the Azure portal's blades do) is `IframePresentational`
/// instead of `Iframe`, and its content is as much the page's as any other's.
fn is_iframe_role(role: &str) -> bool {
    matches!(role, "Iframe" | "IframePresentational")
}

/// Render one frame's tree at `depth`, under its `<iframe>` line.
fn render_frame(f: &FrameTree, frames: &[FrameTree], depth: usize, out: &mut Snapshot) {
    let by_id: HashMap<&str, &Value> =
        f.nodes.iter().filter_map(|n| Some((n.get("nodeId")?.as_str()?, n))).collect();
    let root = f.nodes.iter().find(|n| n.get("parentId").is_none()).or_else(|| f.nodes.first());
    let zones = super::shown_once::copy_zones(&f.nodes);
    if let Some(root) = root {
        walk(root, &by_id, &f.facts, &zones, false, "", depth, "", false, f.ordinal, frames, out);
    }
}

/// A node the snapshot shows no line for: its children are shown in its place.
/// `hide_text` and `parent_name` drop text already shown by an ancestor.
fn is_transparent(node: &Value, role: &str, name: &str, hide_text: bool, parent_name: &str) -> bool {
    let ignored = node.get("ignored").and_then(|v| v.as_bool()).unwrap_or(false);
    ignored
        || TRANSPARENT_ROLES.contains(&role)
        || (role == "StaticText" && (hide_text || name.is_empty() || name == parent_name))
        || role.is_empty()
}

/// The role a line shows: `text` for text, `file input` for a file input.
fn shown_role<'a>(role: &'a str, fact: Option<&DomFacts>) -> &'a str {
    if role == "StaticText" {
        "text"
    } else if fact.is_some_and(DomFacts::is_file_input) {
        "file input"
    } else {
        role
    }
}

/// Does this node get a reference: something an agent can act on.
fn takes_ref(role: &str, backend: Option<i64>, facts: &HashMap<i64, DomFacts>) -> bool {
    let Some(b) = backend else { return false };
    INTERACTIVE_ROLES.contains(&role) || facts.get(&b).is_some_and(DomFacts::is_file_input)
}

/// Whether `node`'s line masks a secret the page shows once: it then gets a
/// reference, for `Browser` `capture`.
fn masks_shown_once(node: &Value, role: &str, name: &str, in_zone: bool) -> bool {
    let value = if VALUE_ROLES.contains(&role) { value_text(node) } else { None };
    super::shown_once::masks(name, value.as_deref(), in_zone, truthy(prop(node, "readonly")))
}

/// The reference the next acted-on node in `frame` would get, without
/// handing it out (`hand_out` does).
fn next_ref(out: &Snapshot, ref_prefix: &str, frame: usize) -> String {
    if frame == 0 {
        // Top-frame references count only themselves, not the frames'.
        format!("{ref_prefix}e{}", out.refs.len() - out.ref_frames.len() + 1)
    } else {
        let n = out.ref_frames.iter().filter(|(_, f)| *f == frame).count() + 1;
        format!("f{frame}e{n}")
    }
}

fn hand_out(out: &mut Snapshot, r: String, backend: i64, frame: usize) {
    if frame != 0 {
        out.ref_frames.push((r.clone(), frame));
    }
    out.refs.push((r, backend));
}

/// One node's line, with `r` as its reference if it has one.
fn line_for(
    node: &Value,
    role: &str,
    name: &str,
    backend: Option<i64>,
    facts: &HashMap<i64, DomFacts>,
    r: Option<&str>,
    frame: usize,
    frames: &[FrameTree],
    in_zone: bool,
) -> String {
    let fact = backend.and_then(|b| facts.get(&b));
    let secret = fact.and_then(secret_reason);
    let mut line = format!("- {}", shown_role(role, fact));
    if !name.is_empty() {
        line.push(' ');
        line.push_str(&quote(&super::shown_once::mask_where(name, in_zone)));
    }
    if let Some(r) = r {
        line.push_str(&format!(" [ref={r}]"));
    }
    if let Some(inner) = frame_of_owner(frames, frame, role, backend) {
        let n = inner.ordinal;
        match &inner.note {
            None => line.push_str(&format!(
                " [frame=f{n} origin={} {}]",
                if inner.origin.is_empty() { "unknown" } else { &inner.origin },
                if inner.cross_origin { "cross-origin" } else { "same-origin" },
            )),
            Some(why) => line.push_str(&format!(" [frame=f{n} {why}]")),
        }
    }
    if role == "heading" {
        if let Some(level) = prop(node, "level").and_then(|v| v.as_i64()) {
            line.push_str(&format!(" [level={level}]"));
        }
    }
    if matches!(role, "checkbox" | "radio" | "switch" | "menuitemcheckbox" | "menuitemradio") {
        match prop(node, "checked") {
            Some(Value::String(s)) if s == "mixed" => line.push_str(" [checked=mixed]"),
            v => line.push_str(if truthy(v) { " [checked]" } else { " [checked=false]" }),
        }
    }
    if let Some(v) = prop(node, "expanded") {
        line.push_str(if truthy(Some(v)) { " [expanded]" } else { " [expanded=false]" });
    }
    if truthy(prop(node, "selected")) {
        line.push_str(" [selected]");
    }
    for (p, label) in [("required", "required"), ("invalid", "invalid"), ("disabled", "disabled"), ("readonly", "readonly")] {
        // `invalid` is a token ("false", "true", "spelling", ...), not a bool.
        let on = match prop(node, p) {
            Some(Value::String(s)) => s != "false",
            other => truthy(other),
        };
        if on {
            line.push_str(&format!(" [{label}]"));
        }
    }
    if VALUE_ROLES.contains(&role) {
        if let Some(f) = fact {
            line.push_str(&limits_of(f));
        }
    }
    if let Some(why) = secret {
        line.push_str(&format!(" [secret: ask the user to fill this; {why}]"));
    } else if VALUE_ROLES.contains(&role) {
        match (fact, value_text(node)) {
            (Some(_), Some(v)) => line.push_str(&format!(" value={}", quote(&super::shown_once::mask_where(&v, in_zone || truthy(prop(node, "readonly")))))),
            // Not checked: it could be a secret field, so its value stays hidden.
            (None, Some(v)) if !v.is_empty() => line.push_str(" [value hidden: not checked; take a scoped snapshot to see it]"),
            _ => {}
        }
    }
    line
}

// ── find: the lines for elements named like this, not the whole page ──────

/// The most matches one `find` shows: enough to pick from, short enough to
/// be cheaper than a snapshot.
pub const MAX_FIND_MATCHES: usize = 20;

/// Elements by role and accessible name, for `find` and for a `scope` given
/// by name. `role` is the role a snapshot line shows (`button`, `link`,
/// `menuitem`, `text`, `file input`), in any case. `name` is a part of the
/// accessible name in any case, or with `exact` the whole name as written.
/// Empty matches every name.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ElementQuery {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub exact: bool,
}

impl ElementQuery {
    fn role(&self) -> Option<String> {
        self.role.as_deref().map(|r| r.trim().to_lowercase()).filter(|r| !r.is_empty())
    }

    /// Does a node match? Without `fact` the node's markup wasn't read, so a
    /// button counts as a possible `file input` (the route reads it, then asks again).
    fn matches(&self, role: &str, name: &str, fact: Option<&DomFacts>, checked: bool) -> bool {
        let role_ok = match self.role().as_deref() {
            // Text is shown only when asked for: otherwise a word in a
            // paragraph would crowd out the button named by it.
            None => role != "StaticText",
            Some("file input") => role == "button" && (fact.is_some_and(DomFacts::is_file_input) || (!checked && fact.is_none())),
            Some(want) => shown_role(role, fact).eq_ignore_ascii_case(want),
        };
        role_ok && self.name_matches(name)
    }

    fn name_matches(&self, name: &str) -> bool {
        let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let (want, have) = (words(&self.name), words(name));
        // No name asks for any name, exact or not: a role alone.
        if want.is_empty() {
            return true;
        }
        if self.exact {
            have == want
        } else {
            have.to_lowercase().contains(&want.to_lowercase())
        }
    }

    /// How the query reads in a message: `button "Save"`.
    pub fn describe(&self) -> String {
        match (self.role(), self.name.trim()) {
            (Some(r), "") => r,
            (Some(r), n) => format!("{r} {}", quote(n)),
            (None, n) => quote(n),
        }
    }
}

/// A shown node, its nearest shown ancestor with a name (`(role, name)`), and
/// whether it is where a page shows a secret once.
type Shown<'a> = (&'a Value, Option<(&'a str, &'a str)>, bool);

/// Every node a snapshot would show a line for, in document order, with its
/// nearest shown ancestor that has a name, for context.
fn shown_nodes(nodes: &[Value]) -> Vec<Shown<'_>> {
    let by_id: HashMap<&str, &Value> =
        nodes.iter().filter_map(|n| Some((n.get("nodeId")?.as_str()?, n))).collect();
    let zones = super::shown_once::copy_zones(nodes);
    let mut out = Vec::new();
    let root = nodes.iter().find(|n| n.get("parentId").is_none()).or_else(|| nodes.first());
    // (node, parent's name, hide text, named ancestor, in a zone): iterative,
    // children pushed in reverse so they come off in document order.
    type Pending<'a> = (&'a Value, &'a str, bool, Option<(&'a str, &'a str)>, bool);
    let mut stack: Vec<Pending> = root.map(|r| (r, "", false, None, false)).into_iter().collect();
    while let Some((node, parent_name, hide_text, context, in_zone)) = stack.pop() {
        let role = str_of(node, "role");
        let name = str_of(node, "name").trim();
        let in_zone = in_zone || super::shown_once::starts_zone(node, &zones);
        let (mut name_for_children, mut context_for_children) = (parent_name, context);
        if !is_transparent(node, role, name, hide_text, parent_name) {
            out.push((node, context, in_zone));
            name_for_children = name;
            if !name.is_empty() {
                context_for_children = Some((if role == "StaticText" { "text" } else { role }, name));
            }
        }
        let hide = hide_text || TEXT_SHOWN_ELSEWHERE.contains(&role);
        if let Some(kids) = node.get("childIds").and_then(|v| v.as_array()) {
            for k in kids.iter().rev() {
                if let Some(c) = k.as_str().and_then(|id| by_id.get(id)) {
                    stack.push((c, name_for_children, hide, context_for_children, in_zone));
                }
            }
        }
    }
    out
}

/// The name a query is matched against: the one a line shows, with a secret the
/// page shows once masked. Matching the raw name would let a query read it a
/// piece at a time (`find name=<prefix>`, counting the matches).
fn shown_name(node: &Value, in_zone: bool) -> String {
    super::shown_once::mask_where(str_of(node, "name").trim(), in_zone).into_owned()
}

/// The nodes that may match `q` before their markup is read: what the find
/// route looks up `DomFacts` for (a field's value, whether a button is a file
/// input). In document order.
pub fn find_candidates<'a>(nodes: &'a [Value], q: &ElementQuery) -> Vec<&'a Value> {
    shown_nodes(nodes)
        .into_iter()
        .filter(|(n, _, in_zone)| q.matches(str_of(n, "role"), &shown_name(n, *in_zone), None, false))
        .map(|(n, _, _)| n)
        .collect()
}

/// The element a `scope` given by role and name narrows a snapshot to: the
/// first match, as `find` would list it.
pub fn first_match(nodes: &[Value], q: &ElementQuery) -> Option<i64> {
    find_candidates(nodes, q).into_iter().find_map(|n| n.get("backendDOMNodeId").and_then(|v| v.as_i64()))
}

/// `find`'s answer: the matching lines, the references on them, and how
/// many matched in all.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub text: String,
    pub refs: Vec<(String, i64)>,
    /// Every match, shown or not: the text holds the first `MAX_FIND_MATCHES`.
    pub total: usize,
    /// The page's iframes, whose contents `find` doesn't search.
    pub frames: usize,
}

/// The lines of the elements matching `q`, each formatted as in a snapshot
/// (the same secret-field rule: a field without facts has its value hidden)
/// with `(in <role> "<name>")` naming its nearest named ancestor. References
/// are `{ref_prefix}e1`, `{ref_prefix}e2`, …; top frame only.
pub fn find(nodes: &[Value], facts: &HashMap<i64, DomFacts>, q: &ElementQuery, ref_prefix: &str) -> Found {
    let mut found = Found { text: String::new(), refs: Vec::new(), total: 0, frames: 0 };
    for (node, context, in_zone) in shown_nodes(nodes) {
        let role = str_of(node, "role");
        let name = str_of(node, "name").trim();
        if is_iframe_role(role) {
            found.frames += 1;
        }
        let backend = node.get("backendDOMNodeId").and_then(|v| v.as_i64());
        if !q.matches(role, &shown_name(node, in_zone), backend.and_then(|b| facts.get(&b)), true) {
            continue;
        }
        found.total += 1;
        if found.total > MAX_FIND_MATCHES {
            continue;
        }
        let r = (takes_ref(role, backend, facts) || (backend.is_some() && masks_shown_once(node, role, name, in_zone)))
            .then(|| format!("{ref_prefix}e{}", found.refs.len() + 1));
        let mut line = line_for(node, role, name, backend, facts, r.as_deref(), 0, &[], in_zone);
        if let Some((in_role, in_name)) = context {
            line.push_str(&format!(" (in {in_role} {})", quote(&super::shown_once::mask_where(in_name, in_zone))));
        }
        if let (Some(r), Some(b)) = (r, backend) {
            found.refs.push((r, b));
        }
        found.text.push_str(&line);
        found.text.push('\n');
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn n(id: &str, parent: Option<&str>, role: &str, name: &str, backend: Option<i64>, kids: &[&str]) -> Value {
        let mut v = json!({
            "nodeId": id,
            "role": { "type": "role", "value": role },
            "name": { "type": "computedString", "value": name },
            "childIds": kids,
            "ignored": false,
        });
        if let Some(p) = parent {
            v["parentId"] = json!(p);
        }
        if let Some(b) = backend {
            v["backendDOMNodeId"] = json!(b);
        }
        v
    }

    fn facts(pairs: &[(i64, &str, &[(&str, &str)])]) -> HashMap<i64, DomFacts> {
        pairs
            .iter()
            .map(|(b, tag, attrs)| {
                (
                    *b,
                    DomFacts {
                        local_name: tag.to_string(),
                        attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                        label: String::new(),
                    },
                )
            })
            .collect()
    }

    /// Facts for every backend id given, as plain text inputs.
    fn plain(ids: &[i64]) -> HashMap<i64, DomFacts> {
        ids.iter().map(|b| (*b, DomFacts { local_name: "input".into(), attrs: HashMap::new(), label: String::new() })).collect()
    }

    #[test]
    fn a_field_whose_type_was_not_checked_never_shows_its_value() {
        // Fail closed: no facts (the lookup was capped or failed) means it
        // could be a password field.
        let mut f = n("2", Some("1"), "textbox", "Code", Some(20), &[]);
        f["value"] = json!({ "type": "string", "value": "123456" });
        let nodes = vec![n("1", None, "RootWebArea", "", Some(1), &["2"]), f];
        let s = render(&nodes, &HashMap::new(), None, "");
        assert!(!s.text.contains("123456"), "{}", s.text);
        assert!(s.text.contains("[value hidden"), "{}", s.text);
    }

    #[test]
    fn subtree_collects_one_branch_in_document_order() {
        let nodes = vec![
            n("1", None, "RootWebArea", "", Some(1), &["2", "3"]),
            n("2", Some("1"), "form", "", Some(2), &["4", "5"]),
            n("3", Some("1"), "textbox", "Outside", Some(3), &[]),
            n("4", Some("2"), "textbox", "A", Some(4), &[]),
            n("5", Some("2"), "textbox", "B", Some(5), &[]),
        ];
        let ids: Vec<i64> = subtree(&nodes, 2).iter().filter_map(|n| n["backendDOMNodeId"].as_i64()).collect();
        assert_eq!(ids, vec![2, 4, 5]);
        assert!(subtree(&nodes, 99).is_empty());
    }

    #[test]
    fn scoped_references_carry_their_prefix_from_the_start() {
        // A page can't spoof a reference by putting "[ref=e1]" in its text:
        // references are generated with their final names, never rewritten.
        let nodes = vec![
            n("1", None, "RootWebArea", "", Some(1), &["2"]),
            n("2", Some("1"), "form", "Login [ref=e1]", Some(2), &["3"]),
            n("3", Some("2"), "button", "Go", Some(3), &[]),
        ];
        let s = render(&nodes, &HashMap::new(), Some(2), "e4.");
        assert_eq!(s.text, "- form \"Login [ref=e1]\"\n  - button \"Go\" [ref=e4.e1]\n");
        assert_eq!(s.refs, vec![("e4.e1".to_string(), 3)]);
    }

    #[test]
    fn a_form_renders_with_references_on_what_can_be_acted_on() {
        let mut email = n("4", Some("2"), "textbox", "Email", Some(40), &[]);
        email["properties"] = json!([{ "name": "required", "value": { "type": "boolean", "value": true } }]);
        email["value"] = json!({ "type": "string", "value": "a@b.c" });
        let mut agree = n("5", Some("2"), "checkbox", "I agree", Some(50), &[]);
        agree["properties"] = json!([{ "name": "checked", "value": { "type": "tristate", "value": "false" } }]);
        let mut h = n("3", Some("2"), "heading", "Submit a file", Some(30), &["7"]);
        h["properties"] = json!([{ "name": "level", "value": { "type": "integer", "value": 1 } }]);
        let nodes = vec![
            n("1", None, "RootWebArea", "Page", Some(1), &["2"]),
            n("2", Some("1"), "generic", "", Some(2), &["3", "4", "5", "6"]),
            h,
            email,
            agree,
            n("6", Some("2"), "button", "Continue", Some(60), &[]),
            n("7", Some("3"), "StaticText", "Submit a file", None, &[]),
        ];
        let s = render(&nodes, &plain(&[40]), None, "");
        assert_eq!(
            s.text,
            "- heading \"Submit a file\" [level=1]\n\
             - textbox \"Email\" [ref=e1] [required] value=\"a@b.c\"\n\
             - checkbox \"I agree\" [ref=e2] [checked=false]\n\
             - button \"Continue\" [ref=e3]\n"
        );
        assert_eq!(s.refs, vec![("e1".into(), 40), ("e2".into(), 50), ("e3".into(), 60)]);
        assert!(!s.truncated);
    }

    #[test]
    fn label_text_and_field_text_are_not_repeated() {
        let mut name = n("4", Some("3"), "textbox", "Your name", Some(40), &["5"]);
        name["value"] = json!({ "type": "string", "value": "Lark" });
        let nodes = vec![
            n("1", None, "RootWebArea", "", Some(1), &["2"]),
            n("2", Some("1"), "LabelText", "", Some(2), &["6", "3"]),
            n("3", Some("2"), "generic", "", Some(3), &["4"]),
            name,
            n("5", Some("4"), "StaticText", "Lark", None, &[]),
            n("6", Some("2"), "StaticText", "Your name", None, &[]),
        ];
        let s = render(&nodes, &plain(&[40]), None, "");
        assert_eq!(s.text, "- textbox \"Your name\" [ref=e1] value=\"Lark\"\n");
    }

    #[test]
    fn a_password_field_is_marked_secret_and_its_value_never_shown() {
        let mut pw = n("2", Some("1"), "textbox", "Password", Some(20), &[]);
        pw["value"] = json!({ "type": "string", "value": "hunter2" });
        let nodes = vec![n("1", None, "RootWebArea", "", Some(1), &["2"]), pw];
        let f = facts(&[(20, "input", &[("type", "password")])]);
        let s = render(&nodes, &f, None, "");
        assert!(s.text.contains("[secret"), "{}", s.text);
        assert!(!s.text.contains("hunter2"), "{}", s.text);
    }

    #[test]
    fn a_file_input_is_named_as_one_and_gets_a_reference() {
        let nodes = vec![
            n("1", None, "RootWebArea", "", Some(1), &["2"]),
            n("2", Some("1"), "button", "Select the file", Some(20), &[]),
        ];
        let f = facts(&[(20, "input", &[("type", "file")])]);
        let s = render(&nodes, &f, None, "");
        assert_eq!(s.text, "- file input \"Select the file\" [ref=e1]\n");
    }

    #[test]
    fn scope_narrows_to_one_subtree() {
        let nodes = vec![
            n("1", None, "RootWebArea", "", Some(1), &["2", "3"]),
            n("2", Some("1"), "form", "Login", Some(2), &["4"]),
            n("3", Some("1"), "link", "Elsewhere", Some(3), &[]),
            n("4", Some("2"), "button", "Go", Some(4), &[]),
        ];
        let s = render(&nodes, &HashMap::new(), Some(2), "");
        assert_eq!(s.text, "- form \"Login\"\n  - button \"Go\" [ref=e1]\n");
    }

    #[test]
    fn a_huge_page_is_truncated_with_a_hint() {
        let mut nodes = vec![];
        let kids: Vec<String> = (0..4000).map(|i| format!("c{i}")).collect();
        let kid_refs: Vec<&str> = kids.iter().map(String::as_str).collect();
        nodes.push(n("root", None, "RootWebArea", "", Some(1), &kid_refs));
        for (i, k) in kids.iter().enumerate() {
            nodes.push(n(k, Some("root"), "link", &format!("Link number {i} with a fairly long name"), Some(10 + i as i64), &[]));
        }
        let s = render(&nodes, &HashMap::new(), None, "");
        assert!(s.truncated);
        assert!(s.text.len() <= MAX_SNAPSHOT_BYTES + 200);
        assert!(s.text.ends_with("use find (a role and name), or pass `scope` (a reference, or a role and quoted name) to read one part of the page\n"));
    }

    #[test]
    fn a_checkbox_hidden_behind_its_label_gets_a_line_and_a_reference() {
        // <label><input type=checkbox style="display:none"><span class=cr></span><b>Short Form Good Standing</b></label>
        let mut nodes = vec![
            n("1", None, "RootWebArea", "", Some(1), &["2"]),
            n("2", Some("1"), "cell", "Short Form Good Standing (Check if Additional Language Required)", Some(2), &["3", "5"]),
            n("3", Some("2"), "LabelText", "", Some(20), &["4"]),
            n("4", Some("3"), "StaticText", "Short Form Good Standing", None, &[]),
            n("5", Some("2"), "link", "", Some(30), &[]),
        ];
        let choice = |label_backend| HiddenChoice {
            input_backend: 21,
            label_backend,
            radio: false,
            name: "Short Form Good Standing".into(),
            checked: false,
            disabled: false,
        };
        add_hidden_choices(&mut nodes, &[choice(20)]);
        let s = render(&nodes, &HashMap::new(), None, "");
        assert!(s.text.contains("  - checkbox \"Short Form Good Standing\" [ref=e1] [checked=false]\n"), "{}", s.text);
        assert_eq!(s.refs[0], ("e1".to_string(), 21), "the reference is the input's");
        // The checkbox comes before the help link beside it.
        assert!(s.text.find("checkbox").unwrap() < s.text.find("- link").unwrap(), "{}", s.text);

        // A label missing from the tree: the checkbox still gets a line, at the end.
        let mut nodes = vec![n("1", None, "RootWebArea", "", Some(1), &["2"]), n("2", Some("1"), "button", "Next", Some(2), &[])];
        add_hidden_choices(&mut nodes, &[HiddenChoice { radio: true, checked: true, ..choice(99) }]);
        let s = render(&nodes, &HashMap::new(), None, "");
        let lines: Vec<&str> = s.text.lines().collect();
        assert_eq!(lines.last().copied(), Some("- radio \"Short Form Good Standing\" [ref=e2] [checked]"), "{}", s.text);
    }

    #[test]
    fn a_secret_mark_says_why() {
        let nodes = vec![
            n("root", None, "RootWebArea", "", None, &["pw"]),
            n("pw", Some("root"), "textbox", "Your SSN", Some(7), &[]),
        ];
        let mut facts = HashMap::new();
        facts.insert(7, DomFacts { local_name: "input".into(), attrs: HashMap::new(), label: "Your SSN".into() });
        let s = render(&nodes, &facts, None, "");
        assert!(
            s.text.contains("[secret: ask the user to fill this; its name or label says Social Security or taxpayer ID number]"),
            "{}",
            s.text
        );
    }

    #[test]
    fn a_tax_id_field_is_marked_secret_in_the_snapshot_and_its_value_hidden() {
        let nodes = vec![
            n("root", None, "RootWebArea", "", Some(1), &["tin"]),
            {
                let mut v = n("tin", Some("root"), "textbox", "Taxpayer's TIN", Some(7), &[]);
                v["value"] = json!({ "type": "string", "value": "123456789" });
                v
            },
        ];
        let mut facts = HashMap::new();
        facts.insert(
            7,
            DomFacts { local_name: "input".into(), attrs: HashMap::new(), label: "Taxpayer's TIN".into() },
        );
        let s = render(&nodes, &facts, None, "");
        assert!(s.text.contains("[secret: ask the user to fill this;"), "{}", s.text);
        assert!(!s.text.contains("123456789"), "{}", s.text);
    }

    fn frame_page() -> (Vec<Value>, FrameTree) {
        let top = vec![
            n("root", None, "RootWebArea", "", Some(1), &["b", "ifr", "c"]),
            n("b", Some("root"), "button", "Top button", Some(2), &[]),
            n("ifr", Some("root"), "Iframe", "Application form", Some(3), &[]),
            n("c", Some("root"), "link", "Top link", Some(4), &[]),
        ];
        let mut inner_nodes = vec![
            n("r", None, "RootWebArea", "", Some(1), &["h", "t", "go"]),
            n("h", Some("r"), "heading", "Step 3", Some(2), &[]),
            n("t", Some("r"), "textbox", "Taxpayer's TIN", Some(10), &[]),
            n("go", Some("r"), "button", "Next", Some(11), &[]),
        ];
        inner_nodes[2]["value"] = json!({ "type": "string", "value": "123456789" });
        let mut facts = HashMap::new();
        facts.insert(10, DomFacts { local_name: "input".into(), attrs: HashMap::new(), label: "Taxpayer's TIN".into() });
        let frame = FrameTree {
            ordinal: 1,
            parent: 0,
            owner_backend: Some(3),
            origin: "https://forms.example.gov".into(),
            cross_origin: true,
            nodes: inner_nodes,
            facts,
            note: None,
        };
        (top, frame)
    }

    #[test]
    fn an_iframe_the_page_marks_presentational_still_shows_its_tree() {
        // The Azure portal's blades: `<iframe role="presentation">` is `IframePresentational`.
        let (mut top, frame) = frame_page();
        top[2]["role"] = json!({ "type": "role", "value": "IframePresentational" });
        let s = render_with_frames(&top, &HashMap::new(), None, "", &[frame]);
        let lines: Vec<&str> = s.text.lines().collect();
        assert_eq!(
            lines[1],
            "- IframePresentational \"Application form\" [frame=f1 origin=https://forms.example.gov cross-origin]"
        );
        assert!(lines[3].starts_with("  - textbox \"Taxpayer's TIN\" [ref=f1e1]"), "{}", lines[3]);
        assert!(is_iframe_role("Iframe") && is_iframe_role("IframePresentational") && !is_iframe_role("generic"));
    }

    #[test]
    fn an_iframes_tree_is_shown_under_it_with_frame_prefixed_references() {
        let (top, frame) = frame_page();
        let s = render_with_frames(&top, &HashMap::new(), None, "", &[frame]);
        let lines: Vec<&str> = s.text.lines().collect();
        assert_eq!(lines[0], "- button \"Top button\" [ref=e1]");
        assert_eq!(
            lines[1],
            "- Iframe \"Application form\" [frame=f1 origin=https://forms.example.gov cross-origin]"
        );
        assert_eq!(lines[2], "  - heading \"Step 3\"");
        assert!(lines[3].starts_with("  - textbox \"Taxpayer's TIN\" [ref=f1e1]"), "{}", lines[3]);
        assert_eq!(lines[4], "  - button \"Next\" [ref=f1e2]");
        assert_eq!(lines[5], "- link \"Top link\" [ref=e2]", "top-frame numbering carries on after the frame");
        assert_eq!(
            s.refs.iter().map(|(r, b)| (r.as_str(), *b)).collect::<Vec<_>>(),
            vec![("e1", 2), ("f1e1", 10), ("f1e2", 11), ("e2", 4)]
        );
        assert_eq!(s.ref_frames, vec![("f1e1".to_string(), 1), ("f1e2".to_string(), 1)]);
    }

    #[test]
    fn a_secret_field_inside_a_frame_is_marked_and_its_value_hidden() {
        let (top, frame) = frame_page();
        let s = render_with_frames(&top, &HashMap::new(), None, "", &[frame]);
        assert!(s.text.contains("[secret: ask the user to fill this;"), "{}", s.text);
        assert!(!s.text.contains("123456789"), "{}", s.text);
    }

    #[test]
    fn a_frame_that_isnt_shown_says_why_on_its_own_line() {
        let (top, mut frame) = frame_page();
        frame.note = Some("not shown: over the frame limit".into());
        frame.nodes.clear();
        let s = render_with_frames(&top, &HashMap::new(), None, "", &[frame]);
        assert!(s.text.contains("- Iframe \"Application form\" [frame=f1 not shown: over the frame limit]"), "{}", s.text);
        assert!(!s.text.contains("Step 3"));
    }

    #[test]
    fn a_frame_inside_a_frame_nests_and_numbers_its_own_references() {
        let (top, outer) = frame_page();
        let mut outer = outer;
        // The outer frame's tree gets an iframe of its own (backend 12) ...
        outer.nodes.push(n("deep", Some("r"), "Iframe", "Inner", Some(12), &[]));
        outer.nodes[0]["childIds"] = json!(["h", "t", "go", "deep"]);
        // ... holding a text field.
        let inner = FrameTree {
            ordinal: 2,
            parent: 1,
            owner_backend: Some(12),
            origin: "https://other.example".into(),
            cross_origin: true,
            nodes: vec![
                n("r", None, "RootWebArea", "", Some(1), &["x"]),
                n("x", Some("r"), "textbox", "City", Some(20), &[]),
            ],
            facts: HashMap::from([(20, DomFacts { local_name: "input".into(), attrs: HashMap::new(), label: String::new() })]),
            note: None,
        };
        let s = render_with_frames(&top, &HashMap::new(), None, "", &[outer, inner]);
        assert!(s.text.contains("    - textbox \"City\" [ref=f2e1]"), "{}", s.text);
        assert!(s.text.contains("  - Iframe \"Inner\" [frame=f2 origin=https://other.example cross-origin]"), "{}", s.text);
    }

    #[test]
    fn an_iframe_with_no_frame_read_stays_a_bare_line() {
        let (top, _) = frame_page();
        let s = render_with_frames(&top, &HashMap::new(), None, "", &[]);
        assert!(s.text.contains("- Iframe \"Application form\"\n"), "{}", s.text);
        assert!(s.ref_frames.is_empty());
    }

    /// Every reference handed out is on a line the agent reads, once.
    fn assert_every_ref_is_shown(s: &Snapshot) {
        for (r, _) in &s.refs {
            assert_eq!(s.text.matches(&format!("[ref={r}]")).count(), 1, "{r} is in the table but not in the text once");
        }
        assert_eq!(s.text.matches("[ref=").count(), s.refs.len(), "a line shows a reference the table doesn't have");
    }

    #[test]
    fn no_reference_is_handed_out_for_a_line_cut_by_the_cap() {
        // Many sizes, so the line that crosses the cap falls on every kind of
        // boundary: a link, a field with a value, a heading without a ref.
        for name_len in [5usize, 17, 40, 61, 99, 150] {
            let count = MAX_SNAPSHOT_BYTES / (name_len + 10) + 50;
            let kids: Vec<String> = (0..count).map(|i| format!("c{i}")).collect();
            let kid_refs: Vec<&str> = kids.iter().map(String::as_str).collect();
            let mut nodes = vec![n("root", None, "RootWebArea", "", Some(1), &kid_refs)];
            let mut ids = Vec::new();
            for (i, k) in kids.iter().enumerate() {
                let role = ["link", "textbox", "heading", "button"][i % 4];
                let name = format!("{i:05}{}", "x".repeat(name_len));
                let mut node = n(k, Some("root"), role, &name, Some(10 + i as i64), &[]);
                if role == "textbox" {
                    node["value"] = json!({ "type": "string", "value": "v" });
                    ids.push(10 + i as i64);
                }
                nodes.push(node);
            }
            let s = render(&nodes, &plain(&ids), None, "");
            assert!(s.truncated, "{name_len}: the fixture should be over the cap");
            assert_every_ref_is_shown(&s);
        }
        // And when nothing is cut.
        let (top, frame) = frame_page();
        assert_every_ref_is_shown(&render_with_frames(&top, &HashMap::new(), None, "", &[frame]));
    }

    /// A page with a dialog, a toolbar, text, a password field and an iframe.
    fn find_page() -> (Vec<Value>, HashMap<i64, DomFacts>) {
        let mut pw = n("pw", Some("form"), "textbox", "Password", Some(31), &[]);
        pw["value"] = json!({ "type": "string", "value": "hunter2" });
        let mut title = n("title", Some("dlg"), "textbox", "Title", Some(21), &[]);
        title["value"] = json!({ "type": "string", "value": "Draft one" });
        let nodes = vec![
            n("root", None, "RootWebArea", "Page", Some(1), &["bar", "p", "dlg", "form", "ifr"]),
            n("bar", Some("root"), "toolbar", "Actions", Some(10), &["sd", "si", "upl"]),
            n("sd", Some("bar"), "button", "Save draft", Some(11), &["sdt"]),
            n("sdt", Some("sd"), "StaticText", "Save draft", None, &[]),
            n("si", Some("bar"), "link", "Saved   items", Some(12), &[]),
            n("upl", Some("bar"), "button", "Attach", Some(13), &[]),
            n("p", Some("root"), "paragraph", "", Some(14), &["pt"]),
            n("pt", Some("p"), "StaticText", "Save your work often", None, &[]),
            n("dlg", Some("root"), "dialog", "Edit item", Some(20), &["g"]),
            n("g", Some("dlg"), "generic", "", Some(19), &["title", "save", "cancel"]),
            title,
            n("save", Some("g"), "button", "Save", Some(22), &[]),
            n("cancel", Some("g"), "button", "Cancel", Some(23), &[]),
            n("form", Some("root"), "form", "", Some(30), &["pw"]),
            pw,
            n("ifr", Some("root"), "Iframe", "Payment", Some(40), &[]),
        ];
        let mut f = plain(&[21]);
        f.extend(facts(&[(31, "input", &[("type", "password")]), (13, "input", &[("type", "file")]), (11, "button", &[]), (22, "button", &[])]));
        (nodes, f)
    }

    fn q(role: Option<&str>, name: &str, exact: bool) -> ElementQuery {
        ElementQuery { role: role.map(str::to_string), name: name.to_string(), exact }
    }

    #[test]
    fn find_returns_only_matching_lines_with_references_and_context() {
        let (nodes, f) = find_page();
        let found = find(&nodes, &f, &q(None, "SAVE", false), "n1");
        assert_eq!(
            found.text,
            "- button \"Save draft\" [ref=n1e1] (in toolbar \"Actions\")\n\
             - link \"Saved items\" [ref=n1e2] (in toolbar \"Actions\")\n\
             - button \"Save\" [ref=n1e3] (in dialog \"Edit item\")\n"
        );
        assert_eq!(found.refs, vec![("n1e1".into(), 11), ("n1e2".into(), 12), ("n1e3".into(), 22)]);
        assert_eq!((found.total, found.frames), (3, 1));
    }

    #[test]
    fn find_narrows_by_role_in_any_case_and_by_the_exact_name() {
        let (nodes, f) = find_page();
        let by_role = find(&nodes, &f, &q(Some("Button"), "save", false), "n2");
        assert_eq!(by_role.refs.iter().map(|(_, b)| *b).collect::<Vec<_>>(), vec![11, 22]);
        let exact = find(&nodes, &f, &q(None, "Save", true), "n3");
        assert_eq!(exact.refs, vec![("n3e1".into(), 22)]);
        // Exact is the whole name as written, spaces aside.
        assert_eq!(find(&nodes, &f, &q(None, "save", true), "n4").total, 0);
        assert_eq!(find(&nodes, &f, &q(None, " Saved items ", true), "n4").total, 1);
        // A role alone lists every element of that role.
        assert_eq!(find(&nodes, &f, &q(Some("dialog"), "", false), "n5").text, "- dialog \"Edit item\"\n");
        // Exact with no name is still a role alone, not "nameless only".
        assert_eq!(find(&nodes, &f, &q(Some("dialog"), "", true), "n6").text, "- dialog \"Edit item\"\n");
    }

    #[test]
    fn find_shows_text_only_when_asked_for_and_it_takes_no_reference() {
        let (nodes, f) = find_page();
        assert!(!find(&nodes, &f, &q(None, "your work", false), "n1").text.contains("text"));
        let text = find(&nodes, &f, &q(Some("text"), "your work", false), "n1");
        assert_eq!(text.text, "- text \"Save your work often\"\n");
        assert!(text.refs.is_empty());
    }

    #[test]
    fn find_masks_secret_fields_like_a_snapshot() {
        let (nodes, f) = find_page();
        let pw = find(&nodes, &f, &q(None, "password", false), "n1");
        assert!(pw.text.contains("[secret: ask the user to fill this;"), "{}", pw.text);
        assert!(!pw.text.contains("hunter2"), "{}", pw.text);
        // A field whose markup wasn't read keeps its value hidden.
        let unread = find(&nodes, &HashMap::new(), &q(None, "title", false), "n1");
        assert!(unread.text.contains("[value hidden"), "{}", unread.text);
        assert!(!unread.text.contains("Draft one"), "{}", unread.text);
        let read = find(&nodes, &f, &q(None, "title", false), "n1");
        assert!(read.text.contains("value=\"Draft one\""), "{}", read.text);
    }

    #[test]
    fn find_names_a_file_input_by_its_shown_role() {
        let (nodes, f) = find_page();
        // Before the markup is read every button may be one ...
        let maybe: Vec<i64> = find_candidates(&nodes, &q(Some("file input"), "", false))
            .iter()
            .filter_map(|n| n["backendDOMNodeId"].as_i64())
            .collect();
        assert_eq!(maybe, vec![11, 13, 22, 23]);
        // ... after, only the real one is listed, and as a file input.
        let found = find(&nodes, &f, &q(Some("file input"), "", false), "n1");
        assert_eq!(found.text, "- file input \"Attach\" [ref=n1e1] (in toolbar \"Actions\")\n");
        assert_eq!(find(&nodes, &f, &q(Some("button"), "attach", false), "n1").total, 0);
    }

    #[test]
    fn find_shows_at_most_twenty_and_counts_the_rest() {
        let kids: Vec<String> = (0..30).map(|i| format!("b{i}")).collect();
        let kid_refs: Vec<&str> = kids.iter().map(String::as_str).collect();
        let mut nodes = vec![n("root", None, "RootWebArea", "", Some(1), &kid_refs)];
        for (i, k) in kids.iter().enumerate() {
            nodes.push(n(k, Some("root"), "menuitem", &format!("Delete item {i}"), Some(100 + i as i64), &[]));
        }
        let found = find(&nodes, &HashMap::new(), &q(Some("menuitem"), "delete", false), "n7");
        assert_eq!(found.total, 30);
        assert_eq!(found.refs.len(), MAX_FIND_MATCHES);
        assert_eq!(found.text.lines().count(), MAX_FIND_MATCHES);
        assert_eq!(found.refs.last().unwrap().0, "n7e20");
    }

    #[test]
    fn a_name_on_the_page_cant_pass_for_a_reference() {
        let nodes = vec![
            n("root", None, "RootWebArea", "", Some(1), &["b"]),
            n("b", Some("root"), "button", "Go [ref=e1]", Some(2), &[]),
        ];
        let found = find(&nodes, &HashMap::new(), &q(None, "go", false), "n1");
        assert_eq!(found.text, "- button \"Go [ref=e1]\" [ref=n1e1]\n");
    }

    #[test]
    fn a_scope_by_name_is_the_first_match() {
        let (nodes, _) = find_page();
        assert_eq!(first_match(&nodes, &q(Some("dialog"), "edit", false)), Some(20));
        assert_eq!(first_match(&nodes, &q(None, "save", false)), Some(11));
        assert_eq!(first_match(&nodes, &q(Some("dialog"), "nothing like it", false)), None);
        assert_eq!(q(Some("Dialog"), "Edit item", false).describe(), "dialog \"Edit item\"");
    }

    #[test]
    fn a_fields_length_limits_and_pattern_show_on_its_line() {
        let nodes = vec![
            n("root", None, "RootWebArea", "", Some(1), &["a", "b", "c"]),
            n("a", Some("root"), "textbox", "Restrictions", Some(2), &[]),
            n("b", Some("root"), "textbox", "ZIP", Some(3), &[]),
            n("c", Some("root"), "textbox", "Name", Some(4), &[]),
        ];
        let f = facts(&[
            (2, "textarea", &[("maxlength", "350")]),
            (3, "input", &[("minlength", "5"), ("maxlength", "9"), ("pattern", "[0-9]{5,9}")]),
            (4, "input", &[("maxlength", "lots")]),
        ]);
        let s = render(&nodes, &f, None, "");
        assert!(s.text.contains("textbox \"Restrictions\" [ref=e1] [maxlength=350]"), "{}", s.text);
        assert!(s.text.contains("textbox \"ZIP\" [ref=e2] [maxlength=9] [minlength=5] [pattern=\"[0-9]{5,9}\"]"), "{}", s.text);
        assert!(s.text.contains("textbox \"Name\" [ref=e3]\n"), "an invalid maxlength is not shown: {}", s.text);
    }

    #[test]
    fn limits_are_shown_and_enforced_only_where_the_browser_applies_them() {
        let f = |tag: &str, attrs: &[(&str, &str)]| DomFacts {
            local_name: tag.into(),
            attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            label: String::new(),
        };
        let number = f("input", &[("type", "number"), ("maxlength", "2")]);
        assert_eq!(limits_of(&number), "");
        assert_eq!(max_length(&number), None, "a number input ignores maxlength");
        assert_eq!(max_length(&f("input", &[("type", "date"), ("maxlength", "4")])), None);
        assert_eq!(max_length(&f("div", &[("contenteditable", "true"), ("maxlength", "4")])), None);
        assert_eq!(max_length(&f("input", &[("type", "email"), ("maxlength", "40")])), Some(40));
        assert_eq!(max_length(&f("input", &[("type", "bogus"), ("maxlength", "5")])), Some(5), "an unknown type is text");
        assert_eq!(limits_of(&f("textarea", &[("maxlength", "9"), ("pattern", "x")])), " [maxlength=9]", "a textarea has no pattern");
    }

    #[test]
    fn a_summary_gets_a_reference_so_a_section_can_be_opened() {
        let nodes = vec![
            n("root", None, "RootWebArea", "", Some(1), &["d"]),
            n("d", Some("root"), "DisclosureTriangle", "Market Values", Some(2), &[]),
        ];
        let s = render(&nodes, &HashMap::new(), None, "");
        assert!(s.text.contains("- DisclosureTriangle \"Market Values\" [ref=e1]"), "{}", s.text);
    }

    #[test]
    fn a_date_input_is_one_field_with_its_value_not_three_spin_buttons() {
        let mut date = n("d", Some("root"), "Date", "Date of transfer", Some(2), &["m", "dd", "y"]);
        date["value"] = json!({ "type": "string", "value": "2026-10-15" });
        let nodes = vec![
            n("root", None, "RootWebArea", "", Some(1), &["d"]),
            date,
            n("m", Some("d"), "spinbutton", "Month", Some(3), &[]),
            n("dd", Some("d"), "spinbutton", "Day", Some(4), &[]),
            n("y", Some("d"), "spinbutton", "Year", Some(5), &[]),
        ];
        let facts = HashMap::from([(2, DomFacts::from_described(&json!({ "localName": "input", "attributes": ["type", "date"] })))]);
        let s = render(&nodes, &facts, None, "");
        assert!(s.text.contains("- Date \"Date of transfer\" [ref=e1] value=\"2026-10-15\""), "{}", s.text);
        assert!(!s.text.contains("spinbutton"), "{}", s.text);
        assert_eq!(s.refs, vec![("e1".to_string(), 2)]);
    }

    #[test]
    fn describe_node_facts_parse() {
        let d = DomFacts::from_described(&json!({ "localName": "INPUT", "attributes": ["type", "File", "Name", "upload"] }));
        assert!(d.is_file_input());
        assert_eq!(d.attrs.get("name").map(String::as_str), Some("upload"));
    }

    /// A checkout page whose fields the markup doesn't call secrets: names like `field_1`, no
    /// `autocomplete`, only what a person reads next to each (the accessible name).
    fn checkout() -> (Vec<Value>, HashMap<i64, DomFacts>) {
        let field = |id: &str, name: &str, value: &str, backend: i64| {
            let mut v = n(id, Some("2"), "textbox", name, Some(backend), &[]);
            v["value"] = json!({ "type": "string", "value": value });
            v
        };
        let nodes = vec![
            n("1", None, "RootWebArea", "Checkout", Some(1), &["2"]),
            n("2", Some("1"), "generic", "", Some(2), &["10", "11", "12", "13", "14", "15", "16", "17", "18"]),
            field("10", "Card number", "4111 1111 1111 1111", 10),
            field("11", "Expiry (MM/YY)", "12/29", 11),
            field("12", "Security code", "123", 12),
            field("13", "Name on card", "Alex Doe", 13),
            field("14", "Routing number", "021000021", 14),
            field("15", "Account number", "000123456789", 15),
            field("16", "Email", "alex@example.test", 16),
            field("17", "Passport expiry", "2030-01-02", 17),
            n("18", Some("2"), "StaticText", "Order total $40. Paid with card 4111-1111-1111-1111, CVV: 123. Your verification code is 482913.", None, &[]),
        ];
        let input = |attrs: &[(&str, &str)], label: &str| DomFacts {
            local_name: "input".into(),
            attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            label: label.into(),
        };
        let facts = HashMap::from([
            (10, input(&[("name", "field_1")], "Card number")),
            (11, input(&[("name", "field_2"), ("placeholder", "MM/YY")], "Expiry (MM/YY)")),
            (12, input(&[("name", "field_3")], "Security code")),
            (13, input(&[("name", "field_4")], "Name on card")),
            (14, input(&[("name", "field_5")], "Routing number")),
            (15, input(&[("name", "field_6")], "Account number")),
            (16, input(&[("name", "email"), ("type", "email")], "Email")),
            (17, input(&[("name", "field_8")], "Passport expiry")),
        ]);
        (nodes, facts)
    }

    #[test]
    fn a_checkout_page_never_shows_a_card_bank_or_id_value_even_when_the_markup_does_not_name_it() {
        let (nodes, f) = checkout();
        let s = render(&nodes, &f, None, "");
        for secret in ["4111", "12/29", "123\"", "021000021", "000123456789", "482913"] {
            assert!(!s.text.contains(secret), "{secret} shown in:
{}", s.text);
        }
        for named in ["Card number", "Expiry (MM/YY)", "Security code", "Routing number", "Account number"] {
            let line = s.text.lines().find(|l| l.contains(named)).unwrap_or_else(|| panic!("no line for {named}: {}", s.text));
            assert!(line.contains("[secret:") && !line.contains("value="), "{line}");
        }
        // What is not a secret still shows: a name on a card, an email, a passport's expiry.
        assert!(s.text.contains("value=\"Alex Doe\""), "{}", s.text);
        assert!(s.text.contains("value=\"alex@example.test\""), "{}", s.text);
        assert!(s.text.contains("value=\"2030-01-02\""), "{}", s.text);
    }

    #[test]
    fn a_page_that_prints_a_card_number_in_plain_text_has_it_hidden() {
        let (nodes, f) = checkout();
        let s = render(&nodes, &f, None, "");
        let echo = s.text.lines().find(|l| l.contains("Order total")).unwrap_or_else(|| panic!("{}", s.text));
        assert!(echo.contains("[card number hidden]") && echo.contains("CVV: [code hidden]") && echo.contains("verification code is [code hidden]"), "{echo}");
        assert!(echo.contains("Order total $40."), "the rest of the text stays: {echo}");
    }

    #[test]
    fn find_and_a_scoped_read_hide_them_too() {
        let (nodes, f) = checkout();
        let found = find(&nodes, &f, &q(Some("textbox"), "", false), "n1");
        assert!(!found.text.contains("4111") && !found.text.contains("021000021"), "{}", found.text);
        let echo = find(&nodes, &f, &q(Some("text"), "order total", false), "n1");
        assert!(!echo.text.contains("4111") && echo.text.contains("[card number hidden]"), "{}", echo.text);
    }

    #[test]
    fn a_number_the_line_cut_falls_inside_does_not_show_its_first_digits() {
        let padding = "x ".repeat(78);
        let line = quote(&format!("{padding}4111 1111 1111 1111 and more words after it"));
        assert!(!line.contains("4111"), "{line}");
    }
}
