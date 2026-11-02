// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Text the page hides with CSS but leaves in the accessibility tree, left out of a snapshot.
//!
//! The accessibility tree already drops `display: none`, `hidden`, `visibility: hidden` and a
//! closed `<details>`. It keeps text hidden by `opacity: 0`, by a box moved off the page, or by a
//! zero-size box that clips its content (a template a wizard shows later), and an agent that reads
//! such a heading reports it to the person as a finding. The layout is read with
//! `DOMSnapshot.captureSnapshot`; a hidden subtree with nothing an agent can act on in it is cut out
//! of the tree before it is rendered. A control hidden that way (the `opacity: 0` input over a
//! styled checkbox, a skip link moved off screen) stays: agents click those.
//!
//! Visually hidden text that is meant for screen readers (a 1 px clipped box) is not hidden here.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use super::cdp::CdpSession;

/// The styles asked for, in the order `layout.styles` returns them.
const STYLES: [&str; 3] = ["opacity", "overflow-x", "overflow-y"];

/// A page with more nodes than this is not read: the answer would cost more than it saves.
const MAX_NODES: usize = 60_000;

/// The backend ids of the nodes the page hides with CSS, from a `DOMSnapshot.captureSnapshot`
/// result asked for [`STYLES`]. Only the top document: a frame's content is read separately.
pub(super) fn hidden_backend_ids(result: &Value) -> HashSet<i64> {
    let mut out = HashSet::new();
    let (Some(strings), Some(doc)) = (result.get("strings").and_then(Value::as_array), result.pointer("/documents/0")) else {
        return out;
    };
    let text = |i: &Value| i.as_i64().and_then(|i| usize::try_from(i).ok()).and_then(|i| strings.get(i)).and_then(Value::as_str).unwrap_or("");
    let ints = |p: &str| -> Vec<i64> { doc.pointer(p).and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_i64().unwrap_or(-1)).collect()).unwrap_or_default() };
    let (parents, backends) = (ints("/nodes/parentIndex"), ints("/nodes/backendNodeId"));
    let layout_nodes = ints("/layout/nodeIndex");
    let (Some(styles), Some(bounds)) = (doc.pointer("/layout/styles").and_then(Value::as_array), doc.pointer("/layout/bounds").and_then(Value::as_array)) else {
        return out;
    };
    if parents.len() > MAX_NODES || parents.len() != backends.len() {
        return out;
    }
    let layout_of: HashMap<usize, usize> = layout_nodes.iter().enumerate().filter_map(|(l, n)| Some((usize::try_from(*n).ok()?, l))).collect();
    // Parents come before their children, so one pass carries a hidden ancestor down.
    let mut hidden = vec![false; parents.len()];
    for i in 0..parents.len() {
        let inherited = usize::try_from(parents[i]).ok().is_some_and(|p| hidden[p]);
        let own = layout_of.get(&i).is_some_and(|&l| {
            let style = |k: usize| styles.get(l).and_then(|s| s.get(k)).map_or("", text);
            let b: Vec<f64> = bounds.get(l).and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default();
            let &[x, y, w, h] = b.as_slice() else { return false };
            let clips = style(1) != "visible" || style(2) != "visible";
            style(0).parse::<f64>().is_ok_and(|o| o <= 0.0) || x + w <= 0.0 || y + h <= 0.0 || (clips && (w <= 0.0 || h <= 0.0))
        });
        hidden[i] = inherited || own;
        if hidden[i] && backends[i] > 0 {
            out.insert(backends[i]);
        }
    }
    out
}

/// Cut the hidden subtrees with nothing to act on out of `nodes`, returning how many were cut.
pub(super) fn prune(nodes: &mut [Value], hidden: &HashSet<i64>) -> usize {
    if hidden.is_empty() {
        return 0;
    }
    let by_id: HashMap<String, usize> = nodes.iter().enumerate().filter_map(|(i, n)| Some((n.get("nodeId")?.as_str()?.to_string(), i))).collect();
    let is_hidden = |n: &Value| n.get("backendDOMNodeId").and_then(Value::as_i64).is_some_and(|b| hidden.contains(&b));
    // Which nodes hold something to act on, themselves or below: a post-order over a tree.
    let mut actionable = vec![false; nodes.len()];
    let mut order = Vec::with_capacity(nodes.len());
    let mut stack: Vec<usize> = nodes.iter().enumerate().filter(|(_, n)| n.get("parentId").is_none()).map(|(i, _)| i).collect();
    while let Some(i) = stack.pop() {
        order.push(i);
        for k in nodes[i].get("childIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            stack.extend(by_id.get(k));
        }
    }
    for &i in order.iter().rev() {
        let role = nodes[i].pointer("/role/value").and_then(Value::as_str).unwrap_or("");
        let below = nodes[i].get("childIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).any(|k| by_id.get(k).is_some_and(|&c| actionable[c]));
        actionable[i] = below || super::snapshot::is_actionable_role(role);
    }
    let mut cut = 0;
    for p in 0..nodes.len() {
        let drop: Vec<String> = nodes[p]
            .get("childIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|k| by_id.get(*k).is_some_and(|&c| is_hidden(&nodes[c]) && !actionable[c]))
            .map(str::to_string)
            .collect();
        if drop.is_empty() {
            continue;
        }
        cut += drop.len();
        if let Some(kids) = nodes[p].get_mut("childIds").and_then(Value::as_array_mut) {
            kids.retain(|k| !k.as_str().is_some_and(|k| drop.iter().any(|d| d == k)));
        }
    }
    cut
}

/// The backend ids of what the page hides with CSS, or none when the layout can't be read (then
/// nothing is left out: showing everything is the safe answer).
pub(super) async fn capture(cdp: &mut CdpSession) -> HashSet<i64> {
    match cdp.call("DOMSnapshot.captureSnapshot", json!({ "computedStyles": STYLES, "includeDOMRects": false })).await {
        Ok(r) => hidden_backend_ids(&r),
        Err(_) => HashSet::new(),
    }
}

/// The line a snapshot ends with when it left hidden text out.
pub(super) fn note(cut: usize) -> String {
    format!("- … {cut} hidden element(s) not shown (opacity 0, zero size or moved off the page): pass include_hidden: true to list them\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `DOMSnapshot.captureSnapshot` result: `(parent, backend, layout)` per node, a layout being
    /// `(opacity, overflow, [x, y, w, h])` or none (not rendered).
    fn dom(nodes: &[(i64, i64, Option<(&str, &str, [f64; 4])>)]) -> Value {
        let mut strings: Vec<String> = Vec::new();
        let mut id = |s: &str| -> usize {
            strings.iter().position(|x| x == s).unwrap_or_else(|| {
                strings.push(s.to_string());
                strings.len() - 1
            })
        };
        let (mut layout_nodes, mut styles, mut bounds) = (vec![], vec![], vec![]);
        for (i, (_, _, l)) in nodes.iter().enumerate() {
            if let Some((opacity, overflow, b)) = l {
                layout_nodes.push(i);
                styles.push(vec![id(opacity), id(overflow), id(overflow)]);
                bounds.push(b.to_vec());
            }
        }
        json!({
            "strings": strings,
            "documents": [{
                "nodes": { "parentIndex": nodes.iter().map(|n| n.0).collect::<Vec<_>>(), "backendNodeId": nodes.iter().map(|n| n.1).collect::<Vec<_>>() },
                "layout": { "nodeIndex": layout_nodes, "styles": styles, "bounds": bounds },
            }],
        })
    }

    const SHOWN: (&str, &str, [f64; 4]) = ("1", "visible", [8.0, 20.0, 700.0, 27.0]);

    #[test]
    fn opacity_zero_a_box_off_the_page_and_a_clipping_zero_size_box_are_hidden() {
        let r = dom(&[
            (-1, 1, Some(SHOWN)),
            (0, 2, Some(("0", "visible", [8.0, 20.0, 700.0, 27.0]))),
            (0, 3, Some(("1", "visible", [-9999.0, 20.0, 120.0, 27.0]))),
            (0, 4, Some(("1", "hidden", [8.0, 20.0, 700.0, 0.0]))),
            (0, 5, Some(SHOWN)),
        ]);
        assert_eq!(hidden_backend_ids(&r), HashSet::from([2, 3, 4]));
    }

    #[test]
    fn a_zero_size_box_that_does_not_clip_and_a_one_pixel_screen_reader_box_stay() {
        let r = dom(&[
            (-1, 1, Some(SHOWN)),
            (0, 2, Some(("1", "visible", [8.0, 20.0, 700.0, 0.0]))),
            (0, 3, Some(("1", "hidden", [8.0, 20.0, 1.0, 1.0]))),
        ]);
        assert!(hidden_backend_ids(&r).is_empty());
    }

    #[test]
    fn what_is_inside_a_hidden_box_is_hidden_and_a_box_not_rendered_is_left_to_the_tree() {
        let r = dom(&[
            (-1, 1, Some(("0", "visible", [8.0, 20.0, 700.0, 27.0]))),
            (0, 2, Some(SHOWN)),
            (0, 3, None),
        ]);
        assert_eq!(hidden_backend_ids(&r), HashSet::from([1, 2, 3]));
        let alone = dom(&[(-1, 1, Some(SHOWN)), (0, 2, None)]);
        assert!(hidden_backend_ids(&alone).is_empty(), "display: none is the accessibility tree's to drop");
    }

    #[test]
    fn a_result_that_cannot_be_read_hides_nothing() {
        assert!(hidden_backend_ids(&json!({})).is_empty());
        assert!(hidden_backend_ids(&json!({ "strings": [], "documents": [{}] })).is_empty());
    }

    fn ax(id: &str, parent: Option<&str>, role: &str, name: &str, backend: i64, kids: &[&str]) -> Value {
        let mut v = json!({
            "nodeId": id, "role": { "value": role }, "name": { "value": name },
            "childIds": kids, "ignored": false, "backendDOMNodeId": backend,
        });
        if let Some(p) = parent {
            v["parentId"] = json!(p);
        }
        v
    }

    #[test]
    fn a_hidden_heading_is_cut_and_a_hidden_control_stays() {
        let mut nodes = vec![
            ax("1", None, "RootWebArea", "", 1, &["2", "3", "4", "6"]),
            ax("2", Some("1"), "heading", "Your company is a government organization", 2, &[]),
            ax("3", Some("1"), "heading", "Visible", 3, &[]),
            ax("4", Some("1"), "generic", "", 4, &["5"]),
            ax("5", Some("4"), "checkbox", "Agree", 5, &[]),
            ax("6", Some("1"), "StaticText", "template text", 6, &[]),
        ];
        let hidden = HashSet::from([2, 4, 5, 6]);
        assert_eq!(prune(&mut nodes, &hidden), 2);
        let kids: Vec<&str> = nodes[0]["childIds"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(kids, vec!["3", "4"], "the hidden checkbox's container stays with it");
        assert_eq!(prune(&mut nodes, &HashSet::new()), 0);
    }
}
