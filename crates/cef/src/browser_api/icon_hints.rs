// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A hint for a control with no accessible name (#47629).
//!
//! An icon-only button has no text, and the accessibility tree names it nothing: a snapshot line was
//! `- button [ref=e7]`, and an agent chose among three of them by trying. The page does say what each one
//! is, just not in the name: a tooltip attribute (`data-tooltip`, `data-original-title`), the control's
//! `description` (an `aria-describedby` tooltip), or the icon itself (a class such as `icon-eye`, a
//! `data-icon`, an SVG `<use href="#copy">`, an SVG `<title>`). `DOMSnapshot.captureSnapshot`, which a
//! snapshot already asks for once (`visibility.rs`), lists every element's attributes, so the hints cost
//! no further call.
//!
//! A control with a name of its own keeps it. One with no name gets `(no name; ...)` as its name, so a
//! snapshot line, a `find` by that word and the pane Log all say what it looks like. It is a hint, never a
//! promise: the page's own words (a tooltip) are quoted, an icon is given as the word it matched.

use std::collections::HashMap;

use serde_json::Value;

/// The longest tooltip text quoted.
const MAX_TIP: usize = 60;
/// How far below the control its icon is looked for.
const DEPTH: usize = 4;
/// At most this many controls are given a hint in one snapshot.
const MAX_HINTS: usize = 60;

/// Roles of the controls that get a hint when they have no name.
const CONTROLS: &[&str] = &["button", "link", "menuitem", "tab", "switch", "checkbox", "summary"];

/// Attributes that hold a tooltip's text, in the order they are tried.
const TIP_ATTRS: &[&str] = &["data-tooltip", "data-original-title", "data-title", "data-tip", "data-balloon", "data-bs-original-title", "tooltip"];
/// Attributes that name an icon, besides `class`.
const ICON_ATTRS: &[&str] = &["data-icon", "data-testid", "data-test", "data-test-id", "data-qa", "data-cy", "name", "id", "icon"];

/// Icon words (the lower-case pieces of a class or id) and what each is called in a hint.
const ICONS: &[(&str, &str)] = &[
    ("eye", "eye"),
    ("visibility", "eye"),
    ("reveal", "eye"),
    ("unmask", "eye"),
    ("copy", "copy"),
    ("clipboard", "copy"),
    ("duplicate", "copy"),
    ("refresh", "refresh"),
    ("rotate", "rotate"),
    ("roll", "roll"),
    ("regenerate", "regenerate"),
    ("renew", "renew"),
    ("reload", "reload"),
    ("sync", "refresh"),
    ("trash", "trash"),
    ("delete", "trash"),
    ("bin", "trash"),
    ("edit", "edit"),
    ("pencil", "pencil"),
    ("download", "download"),
    ("upload", "upload"),
    ("close", "close"),
    ("dismiss", "close"),
    ("cancel", "close"),
    ("menu", "menu"),
    ("hamburger", "menu"),
    ("ellipsis", "more"),
    ("kebab", "more"),
    ("dots", "more"),
    ("overflow", "more"),
    ("settings", "settings"),
    ("gear", "settings"),
    ("cog", "settings"),
    ("search", "search"),
    ("plus", "add"),
    ("add", "add"),
    ("minus", "minus"),
    ("chevron", "chevron"),
    ("caret", "chevron"),
    ("arrow", "arrow"),
    ("info", "info"),
    ("help", "help"),
    ("question", "help"),
    ("share", "share"),
    ("star", "star"),
    ("link", "link"),
    ("lock", "lock"),
    ("unlock", "lock"),
    ("key", "key"),
    ("play", "play"),
    ("pause", "pause"),
    ("stop", "stop"),
    ("check", "check"),
    ("save", "save"),
];
/// Words that turn an eye into its crossed-out form.
const OFF: &[&str] = &["off", "slash", "closed", "hide", "hidden", "invisible", "crossed", "blind", "mask", "masked"];

/// The words of an attribute value, lower-cased: split on anything that is not a letter and on a
/// lower-to-upper step, with digits dropped (`icon-eye-16`, `ContentCopy`, `fa fa-eye-slash`).
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in text.chars() {
        if !c.is_alphabetic() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        prev_lower = c.is_lowercase();
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// What the icon words say, as one or two hint words (`eye`, `eye-off`, `copy`), or none.
fn icon_of(pieces: &[String]) -> Option<String> {
    // The first icon word in the order the page wrote them, not the order of the table.
    let found = pieces.iter().find_map(|p| ICONS.iter().find(|(w, _)| w == p).map(|(_, n)| *n))?;
    if found == "eye" && pieces.iter().any(|p| OFF.contains(&p.as_str())) {
        return Some("eye-off".to_string());
    }
    Some(found.to_string())
}

/// A tooltip's text, trimmed and cut, with a card or ID number, a code or a token masked first. It is cut at
/// the end of a word, never inside one: a token cut in two shows its first part, which no later cleanup (a
/// secret withheld in the same row is taken out of every name that repeats it) can know was a secret's.
fn tip(text: &str) -> Option<String> {
    let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        return None;
    }
    // A secret's shape is found in the whole text, before any of it is cut.
    let masked = agentmux_common::pii_mask::mask_bounded(&agentmux_common::secret_shape::scrub(&clean), usize::MAX);
    if masked.chars().count() <= MAX_TIP {
        return Some(masked);
    }
    let head: String = masked.chars().take(MAX_TIP + 1).collect();
    // One long word with no end inside the limit is not quoted at all.
    let end = head.rfind(' ')?;
    Some(format!("{}…", head[..end].trim_end()))
}

/// What one element of a `DOMSnapshot` document says it is: its tooltip text, else its icon words.
#[derive(Default)]
struct Look {
    tip: Option<String>,
    pieces: Vec<String>,
}

/// The hint (`title "Rotate"`, `icon: eye`) for each control element in a `DOMSnapshot.captureSnapshot`
/// result, by backend id. A control with a name of its own never shows it (`mark`).
pub(super) fn hints(result: &Value) -> HashMap<i64, String> {
    let mut out = HashMap::new();
    let Some(strings) = result.get("strings").and_then(Value::as_array) else { return out };
    let text = |i: i64| usize::try_from(i).ok().and_then(|i| strings.get(i)).and_then(Value::as_str).unwrap_or("");
    for doc in result.get("documents").and_then(Value::as_array).into_iter().flatten() {
        let ints = |p: &str| -> Vec<i64> { doc.pointer(p).and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_i64().unwrap_or(-1)).collect()).unwrap_or_default() };
        let (parents, backends, names, values) = (ints("/nodes/parentIndex"), ints("/nodes/backendNodeId"), ints("/nodes/nodeName"), ints("/nodes/nodeValue"));
        let attrs: Vec<Vec<i64>> = doc
            .pointer("/nodes/attributes")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|n| n.as_array().map(|p| p.iter().map(|v| v.as_i64().unwrap_or(-1)).collect()).unwrap_or_default()).collect())
            .unwrap_or_default();
        if parents.len() != backends.len() || parents.len() != names.len() {
            continue;
        }
        let attr = |i: usize, key: &str| -> Option<&str> {
            attrs.get(i)?.chunks(2).find_map(|p| match p {
                [k, v] if text(*k).eq_ignore_ascii_case(key) => Some(text(*v)),
                _ => None,
            })
        };
        let mut kids: Vec<Vec<usize>> = vec![Vec::new(); parents.len()];
        for (i, &p) in parents.iter().enumerate() {
            if let Some(p) = usize::try_from(p).ok().filter(|&p| p < parents.len() && p != i) {
                kids[p].push(i);
            }
        }
        for i in 0..parents.len() {
            if out.len() >= MAX_HINTS {
                return out;
            }
            let tag = text(names[i]).to_ascii_lowercase();
            let role = attr(i, "role").unwrap_or("").to_ascii_lowercase();
            let is_control = matches!(tag.as_str(), "button" | "a" | "summary") || CONTROLS.contains(&role.as_str());
            if !is_control || backends[i] <= 0 {
                continue;
            }
            let mut look = Look::default();
            // The control and what is under it, a few levels down.
            let mut stack = vec![(i, 0usize)];
            while let Some((n, depth)) = stack.pop() {
                let own_tag = text(names[n]).to_ascii_lowercase();
                if look.tip.is_none() {
                    look.tip = TIP_ATTRS.iter().find_map(|k| attr(n, k)).and_then(tip);
                }
                if let Some(c) = attr(n, "class") {
                    look.pieces.extend(words(c));
                }
                for k in ICON_ATTRS {
                    if let Some(v) = attr(n, k) {
                        look.pieces.extend(words(v));
                    }
                }
                // An SVG's `<use href="#icon-copy">`, and its `<title>`.
                for k in ["href", "xlink:href"] {
                    if own_tag == "use" {
                        if let Some(v) = attr(n, k) {
                            look.pieces.extend(words(v.rsplit('#').next().unwrap_or(v)));
                        }
                    }
                }
                if own_tag == "title" && look.tip.is_none() {
                    look.tip = kids[n].iter().filter_map(|&c| (values.get(c).copied().unwrap_or(-1) >= 0).then(|| text(values[c]))).next().and_then(tip);
                }
                if depth < DEPTH {
                    stack.extend(kids[n].iter().rev().map(|&c| (c, depth + 1)));
                }
            }
            let hint = match (look.tip, icon_of(&look.pieces)) {
                (Some(t), _) => Some(format!("title \"{t}\"")),
                (None, Some(icon)) => Some(format!("icon: {icon}")),
                _ => None,
            };
            if let Some(h) = hint {
                out.insert(backends[i], h);
            }
        }
    }
    out
}

/// Give the controls with no name a name that says what the page shows for them: their description (a
/// tooltip the page links with `aria-describedby`), else the hint `hints` found. Returns how many.
pub(super) fn mark(nodes: &mut [Value], hints: &HashMap<i64, String>) -> usize {
    let mut marked = 0;
    for node in nodes.iter_mut() {
        let role = node.pointer("/role/value").and_then(Value::as_str).unwrap_or("");
        if !CONTROLS.contains(&role) || node.get("ignored").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        if !node.pointer("/name/value").and_then(Value::as_str).unwrap_or("").trim().is_empty() {
            continue;
        }
        let described = node.pointer("/description/value").and_then(Value::as_str).map(str::trim).filter(|d| !d.is_empty()).and_then(tip).map(|d| format!("title \"{d}\""));
        let by_page = node.get("backendDOMNodeId").and_then(Value::as_i64).and_then(|b| hints.get(&b)).cloned();
        if let Some(h) = described.or(by_page) {
            node["name"] = serde_json::json!({ "type": "computedString", "value": format!("(no name; {h})") });
            marked += 1;
        }
    }
    marked
}

#[cfg(test)]
#[path = "icon_hints_tests.rs"]
mod tests;
