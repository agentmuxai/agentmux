// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The accessibility snapshot an agent reads a browser pane through, and the
//! element references it acts with
//! (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §4.1).
//!
//! Pure functions over CDP `Accessibility.getFullAXTree` output plus a few
//! `DOM.describeNode` facts, so the format and the secret-field rule are
//! unit-testable without a browser. The routes in `routes.rs` gather the
//! inputs and act on the references.

use std::collections::HashMap;

use serde_json::Value;

/// Snapshot text is capped so one page can't flood an agent's context.
pub const MAX_SNAPSHOT_BYTES: usize = 40_000;

/// Roles an agent can act on: each gets a reference.
const INTERACTIVE_ROLES: &[&str] = &[
    "button", "link", "textbox", "searchbox", "combobox", "listbox", "option", "checkbox", "radio",
    "switch", "slider", "spinbutton", "menuitem", "menuitemcheckbox", "menuitemradio", "tab",
    "treeitem", "textarea", "PopUpButton", "ComboBoxMenuButton", "ComboBoxSelect",
];

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
        Self { local_name, attrs }
    }

    fn attr(&self, name: &str) -> &str {
        self.attrs.get(name).map(String::as_str).unwrap_or("")
    }

    pub fn is_file_input(&self) -> bool {
        self.local_name == "input" && self.attr("type").eq_ignore_ascii_case("file")
    }
}

/// A field the agent must never type into or read: passwords, one-time codes
/// and card details (spec §5.1). A guard against mistakes and page tricks,
/// not a sandbox.
pub fn is_secret(f: &DomFacts) -> bool {
    if f.local_name == "input" && f.attr("type").eq_ignore_ascii_case("password") {
        return true;
    }
    let autocomplete = f.attr("autocomplete").to_ascii_lowercase();
    if autocomplete.split_whitespace().any(|t| {
        t == "current-password" || t == "new-password" || t == "one-time-code" || t.starts_with("cc-")
    }) {
        return true;
    }
    // Named like one: password, passcode, otp, one-time code, cvc/cvv, card number.
    let names = [f.attr("name"), f.attr("id")].join(" ").to_ascii_lowercase();
    let squashed: String = names.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').collect();
    squashed.split_whitespace().any(|w| {
        w.contains("password")
            || w.contains("passwd")
            || w.contains("passcode")
            || w == "otp"
            || w.starts_with("otp")
            || w.contains("onetimecode")
            || w.contains("onetimepassword")
            || w == "cvc"
            || w == "cvv"
            || w.contains("cardnumber")
            || w.contains("ccnumber")
            || w.contains("securitycode")
    }) || names.contains("one-time") || names.contains("one_time") || names.contains("card-number") || names.contains("card_number")
}

/// A rendered snapshot and the references it handed out (`e1` → backend node id).
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub text: String,
    pub refs: Vec<(String, i64)>,
    pub truncated: bool,
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
    let clipped: String = if one_line.chars().count() > 160 {
        one_line.chars().take(157).collect::<String>() + "…"
    } else {
        one_line
    };
    serde_json::to_string(&clipped).unwrap_or_else(|_| "\"\"".to_string())
}

/// Render `Accessibility.getFullAXTree`'s `nodes` as an indented list, one
/// line per meaningful node, with a reference on each interactive one.
/// `scope_backend_id` narrows the snapshot to the subtree of that element.
pub fn render(nodes: &[Value], facts: &HashMap<i64, DomFacts>, scope_backend_id: Option<i64>) -> Snapshot {
    let by_id: HashMap<&str, &Value> =
        nodes.iter().filter_map(|n| Some((n.get("nodeId")?.as_str()?, n))).collect();
    let root = match scope_backend_id {
        Some(b) => nodes.iter().find(|n| n.get("backendDOMNodeId").and_then(|v| v.as_i64()) == Some(b)),
        None => nodes
            .iter()
            .find(|n| n.get("parentId").is_none())
            .or_else(|| nodes.first()),
    };
    let mut out = Snapshot { text: String::new(), refs: Vec::new(), truncated: false };
    if let Some(root) = root {
        walk(root, &by_id, facts, 0, "", false, &mut out);
    }
    if out.truncated {
        out.text.push_str("- … snapshot truncated at 40 KB: pass `scope` (a reference) to read one part of the page\n");
    }
    out
}

fn walk(
    node: &Value,
    by_id: &HashMap<&str, &Value>,
    facts: &HashMap<i64, DomFacts>,
    depth: usize,
    parent_name: &str,
    hide_text: bool,
    out: &mut Snapshot,
) {
    if out.truncated {
        return;
    }
    let ignored = node.get("ignored").and_then(|v| v.as_bool()).unwrap_or(false);
    let role = str_of(node, "role");
    let name = str_of(node, "name").trim();
    let backend = node.get("backendDOMNodeId").and_then(|v| v.as_i64());
    let is_text = role == "StaticText";
    let transparent = ignored
        || TRANSPARENT_ROLES.contains(&role)
        || (is_text && (hide_text || name.is_empty() || name == parent_name))
        || role.is_empty();

    let mut child_depth = depth;
    let mut name_for_children = parent_name;
    if !transparent {
        let line = line_for(node, role, name, backend, facts, out);
        let indent = "  ".repeat(depth);
        if out.text.len() + indent.len() + line.len() + 1 > MAX_SNAPSHOT_BYTES {
            out.truncated = true;
            return;
        }
        out.text.push_str(&indent);
        out.text.push_str(&line);
        out.text.push('\n');
        child_depth = depth + 1;
        name_for_children = name;
    }
    if let Some(children) = node.get("childIds").and_then(|v| v.as_array()) {
        for c in children {
            if let Some(child) = c.as_str().and_then(|id| by_id.get(id)) {
                walk(child, by_id, facts, child_depth, name_for_children, hide_text || TEXT_SHOWN_ELSEWHERE.contains(&role), out);
            }
        }
    }
}

fn line_for(
    node: &Value,
    role: &str,
    name: &str,
    backend: Option<i64>,
    facts: &HashMap<i64, DomFacts>,
    out: &mut Snapshot,
) -> String {
    let fact = backend.and_then(|b| facts.get(&b));
    let secret = fact.is_some_and(is_secret);
    let file = fact.is_some_and(DomFacts::is_file_input);
    let shown_role = if role == "StaticText" { "text" } else if file { "file input" } else { role };
    let mut line = format!("- {shown_role}");
    if !name.is_empty() {
        line.push(' ');
        line.push_str(&quote(name));
    }
    if let Some(b) = backend {
        if INTERACTIVE_ROLES.contains(&role) || file {
            let r = format!("e{}", out.refs.len() + 1);
            line.push_str(&format!(" [ref={r}]"));
            out.refs.push((r, b));
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
    if secret {
        line.push_str(" [secret: ask the user to fill this]");
    } else if matches!(role, "textbox" | "searchbox" | "combobox" | "spinbutton" | "slider" | "textarea") {
        if let Some(v) = value_text(node) {
            line.push_str(&format!(" value={}", quote(&v)));
        }
    }
    line
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
                    },
                )
            })
            .collect()
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
        let s = render(&nodes, &HashMap::new(), None);
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
        let s = render(&nodes, &HashMap::new(), None);
        assert_eq!(s.text, "- textbox \"Your name\" [ref=e1] value=\"Lark\"
");
    }

    #[test]
    fn a_password_field_is_marked_secret_and_its_value_never_shown() {
        let mut pw = n("2", Some("1"), "textbox", "Password", Some(20), &[]);
        pw["value"] = json!({ "type": "string", "value": "hunter2" });
        let nodes = vec![n("1", None, "RootWebArea", "", Some(1), &["2"]), pw];
        let f = facts(&[(20, "input", &[("type", "password")])]);
        let s = render(&nodes, &f, None);
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
        let s = render(&nodes, &f, None);
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
        let s = render(&nodes, &HashMap::new(), Some(2));
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
        let s = render(&nodes, &HashMap::new(), None);
        assert!(s.truncated);
        assert!(s.text.len() <= MAX_SNAPSHOT_BYTES + 200);
        assert!(s.text.ends_with("pass `scope` (a reference) to read one part of the page\n"));
    }

    #[test]
    fn secret_fields_are_recognized_by_type_autocomplete_and_name() {
        let f = |tag: &str, attrs: &[(&str, &str)]| DomFacts {
            local_name: tag.into(),
            attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        };
        assert!(is_secret(&f("input", &[("type", "password")])));
        assert!(is_secret(&f("input", &[("autocomplete", "one-time-code")])));
        assert!(is_secret(&f("input", &[("autocomplete", "section-pay cc-number")])));
        assert!(is_secret(&f("input", &[("name", "user_password")])));
        assert!(is_secret(&f("input", &[("id", "otp-input")])));
        assert!(is_secret(&f("input", &[("name", "card-number")])));
        assert!(is_secret(&f("input", &[("name", "cvc")])));
        assert!(!is_secret(&f("input", &[("type", "email"), ("name", "email")])));
        assert!(!is_secret(&f("input", &[("name", "detection_name"), ("autocomplete", "off")])));
        assert!(!is_secret(&f("textarea", &[("name", "comments")])));
    }

    #[test]
    fn describe_node_facts_parse() {
        let d = DomFacts::from_described(&json!({ "localName": "INPUT", "attributes": ["type", "File", "Name", "upload"] }));
        assert!(d.is_file_input());
        assert_eq!(d.attrs.get("name").map(String::as_str), Some("upload"));
    }
}
