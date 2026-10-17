// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The iframes of a browser pane, for the snapshot and for acting inside them
//! (docs/specs/SPEC_BROWSER_PANE_FORMS_IN_FRAMES_AND_VISUAL_FALLBACK_2026_10_16.md
//! §4.2, §4.3).
//!
//! A page's frames live in different places. Same-process frames are in the
//! page's own frame tree. An iframe from another site runs in its own process
//! and has its own CDP session (see `cdp.rs`); the page's frame tree doesn't
//! list it, but its own session's does, and the tree's root frame names its
//! parent (`parentId`). So the frames are the union of every session's tree,
//! stitched together by `parentId`, with each `<iframe>` element found by
//! `DOM.getFrameOwner` asked of the frame that holds it.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use super::cdp::CdpSession;
use super::snapshot::{FrameTree, VALUE_ROLES};

/// Frames shown per snapshot, and how deep: the rest are named but not read.
pub const MAX_FRAMES: usize = 16;
pub const MAX_FRAME_DEPTH: usize = 4;

/// A frame as the frame trees describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFrame {
    pub id: String,
    /// `None` for the page's main frame.
    pub parent_id: Option<String>,
    pub url: String,
    pub origin: String,
    /// The session that reaches it: `None` for the page's own.
    pub session: Option<String>,
}

/// One step from a frame down to the frame inside it: the `<iframe>` element
/// and the session that reaches the frame that element is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHop {
    pub owner_backend: i64,
    pub parent_session: Option<String>,
}

/// What a reference inside a frame needs to find its element again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRef {
    pub frame_id: String,
    pub session: Option<String>,
    pub origin: String,
    pub url: String,
    /// From the top frame down to this one.
    pub chain: Vec<FrameHop>,
}

/// Every frame in a `Page.getFrameTree` result (the root frame first), as
/// reached through `session`.
pub fn flatten(tree: &Value, session: Option<&str>) -> Vec<RawFrame> {
    fn visit(node: &Value, session: Option<&str>, out: &mut Vec<RawFrame>) {
        let Some(frame) = node.get("frame") else {
            return;
        };
        let text = |k: &str| frame.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let url = text("url");
        let origin = match text("securityOrigin") {
            o if !o.is_empty() && o != "://" => o,
            _ => origin_of(&url),
        };
        out.push(RawFrame {
            id: text("id"),
            parent_id: Some(text("parentId")).filter(|p| !p.is_empty()),
            url,
            origin,
            session: session.map(str::to_string),
        });
        if let Some(kids) = node.get("childFrames").and_then(Value::as_array) {
            for k in kids {
                visit(k, session, out);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(root) = tree.get("frameTree") {
        visit(root, session, &mut out);
    }
    out
}

/// `scheme://host[:port]` of a URL, or empty when it has none.
fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return String::new();
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{scheme}://{host}")
}

/// A frame in document order: its place in `frames`, the ordinal of the frame
/// holding it (0: the top) and how deep it is (a child of the top is 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub index: usize,
    pub parent: usize,
    pub depth: usize,
}

/// Put `frames` (the main frame first) in order: each frame's children are
/// sorted by their `<iframe>` element's backend node id (nodes are numbered as
/// they are created, so that is document order and stays the same from one
/// snapshot to the next), depth first. Frames whose parent isn't known are
/// left out. The result is numbered from ordinal 1: `result[i]` is `f{i+1}`.
pub fn arrange(frames: &[RawFrame], owners: &HashMap<String, i64>) -> Vec<Placed> {
    let mut out: Vec<Placed> = Vec::new();
    fn descend(
        frames: &[RawFrame],
        owners: &HashMap<String, i64>,
        parent_index: usize,
        parent_ordinal: usize,
        depth: usize,
        out: &mut Vec<Placed>,
        seen: &mut HashSet<usize>,
    ) {
        let mut kids: Vec<usize> = frames
            .iter()
            .enumerate()
            .filter(|(i, f)| *i != 0 && !seen.contains(i) && f.parent_id.as_deref() == Some(frames[parent_index].id.as_str()))
            .map(|(i, _)| i)
            .collect();
        kids.sort_by_key(|i| (owners.get(&frames[*i].id).copied().unwrap_or(i64::MAX), *i));
        for i in kids {
            seen.insert(i);
            out.push(Placed { index: i, parent: parent_ordinal, depth });
            let ordinal = out.len();
            descend(frames, owners, i, ordinal, depth + 1, out, seen);
        }
    }
    if !frames.is_empty() {
        descend(frames, owners, 0, 0, 1, &mut out, &mut HashSet::new());
    }
    out
}

/// Role of the buttons whose DOM facts are also looked up (a file input's role).
const BUTTON_ROLE: &str = "button";

/// The accessibility nodes of `tree` that need DOM facts.
fn nodes_needing_facts(nodes: &[Value]) -> Vec<(i64, String)> {
    let role_of = |n: &Value| n.pointer("/role/value").and_then(Value::as_str).unwrap_or("").to_string();
    let wanted = nodes
        .iter()
        .filter(|n| VALUE_ROLES.contains(&role_of(n).as_str()))
        .chain(nodes.iter().filter(|n| role_of(n) == BUTTON_ROLE));
    wanted
        .filter_map(|n| {
            let b = n.get("backendDOMNodeId").and_then(Value::as_i64)?;
            let name = n.pointer("/name/value").and_then(Value::as_str).unwrap_or("").to_string();
            Some((b, name))
        })
        .collect()
}

/// Every frame the pane has now, the main one first, and the sessions of the
/// frames running in their own process.
pub async fn current_frames(root: &mut CdpSession) -> (Vec<RawFrame>, Vec<super::cdp::AttachedTarget>) {
    let Ok(main_tree) = root.call("Page.getFrameTree", json!({})).await else {
        return (Vec::new(), Vec::new());
    };
    let mut all = flatten(&main_tree, None);
    if all.is_empty() {
        return (all, Vec::new());
    }
    // Frames in another process: their own sessions' trees.
    let targets = root.attach_frames().await;
    for t in &targets {
        let mut on = root.in_session(Some(&t.session_id));
        if let Ok(tree) = on.call("Page.getFrameTree", json!({})).await {
            for f in flatten(&tree, Some(&t.session_id)) {
                if !all.iter().any(|known| known.id == f.id) {
                    all.push(f);
                }
            }
        }
    }
    (all, targets)
}

/// Is the frame behind a frame reference still there, still showing the page
/// the snapshot saw, and still reached the same way?
pub async fn check_frame(root: &mut CdpSession, f: &FrameRef, r: &str) -> Result<(), String> {
    let (all, _) = current_frames(root).await;
    match all.iter().find(|x| x.id == f.frame_id) {
        None => Err(format!("the frame holding {r} is gone from the page; take a new snapshot")),
        Some(now) if now.url != f.url => {
            Err(format!("the frame holding {r} has moved on (now {}); take a new snapshot", now.url))
        }
        Some(now) if now.session != f.session => {
            Err(format!("the frame holding {r} was reloaded in another process; take a new snapshot"))
        }
        Some(_) => Ok(()),
    }
}

/// The hops whose boxes are added to place a frame on the page. Chromium
/// measures an element's box from the top of its own process's frames (the
/// "local root"), so for frames nested in the same process the inner
/// `<iframe>`'s box already includes the outer frames' offsets: only the last
/// hop of each run of hops through the same session counts.
fn hops_to_add(chain: &[FrameHop]) -> Vec<&FrameHop> {
    let mut out: Vec<&FrameHop> = Vec::new();
    for hop in chain {
        match out.last() {
            Some(last) if last.parent_session == hop.parent_session => {
                out.pop();
            }
            _ => {}
        }
        out.push(hop);
    }
    out
}

/// Where the frame's top-left corner is in the main frame's viewport, in CSS
/// pixels: the sum of the `<iframe>` element boxes [`hops_to_add`] picks out.
/// Add it to a point measured inside the frame to click that point from the
/// page.
pub async fn frame_offset(root: &mut CdpSession, f: &FrameRef) -> Result<(f64, f64), String> {
    let (mut x, mut y) = (0.0, 0.0);
    for hop in hops_to_add(&f.chain) {
        let mut on = root.in_session(hop.parent_session.as_deref());
        let model = on
            .call("DOM.getBoxModel", json!({ "backendNodeId": hop.owner_backend }))
            .await
            .map_err(|e| format!("couldn't place the frame on the page: {e}"))?;
        let quad = model
            .pointer("/model/content")
            .and_then(Value::as_array)
            .filter(|q| q.len() >= 2)
            .ok_or("couldn't place the frame on the page: no box for its <iframe>")?;
        x += quad[0].as_f64().unwrap_or(0.0);
        y += quad[1].as_f64().unwrap_or(0.0);
    }
    Ok((x, y))
}

/// Read every iframe of the pane: each one's accessibility tree and the
/// references' routes back to their elements. `describes_left` is the shared
/// budget of DOM lookups (the top frame spent some).
pub async fn read_frames(root: &mut CdpSession, describes_left: &mut usize) -> (Vec<FrameTree>, HashMap<usize, FrameRef>) {
    let mut trees = Vec::new();
    let mut refs = HashMap::new();
    let (all, targets) = current_frames(root).await;
    let Some(main) = all.first().cloned() else {
        return (trees, refs);
    };
    if all.len() == 1 {
        return (trees, refs);
    }
    // Each `<iframe>` element, asked of the frame that holds it.
    let session_of: HashMap<&str, Option<&str>> = all.iter().map(|f| (f.id.as_str(), f.session.as_deref())).collect();
    let mut owners: HashMap<String, i64> = HashMap::new();
    for f in all.iter().skip(1) {
        let Some(parent) = f.parent_id.as_deref() else { continue };
        let mut on = root.in_session(session_of.get(parent).copied().flatten());
        if let Ok(v) = on.call("DOM.getFrameOwner", json!({ "frameId": f.id })).await {
            if let Some(b) = v.get("backendNodeId").and_then(Value::as_i64) {
                owners.insert(f.id.clone(), b);
            }
        }
    }
    let placed = arrange(&all, &owners);
    let mut enabled: HashSet<Option<String>> = HashSet::new();
    let mut chains: Vec<Vec<FrameHop>> = Vec::new();
    for (n, p) in placed.iter().enumerate() {
        let ordinal = n + 1;
        let raw = &all[p.index];
        let parent_session = all.iter().find(|f| Some(f.id.as_str()) == raw.parent_id.as_deref()).and_then(|f| f.session.clone());
        let mut chain = if p.parent == 0 { Vec::new() } else { chains[p.parent - 1].clone() };
        if let Some(&owner) = owners.get(&raw.id) {
            chain.push(FrameHop { owner_backend: owner, parent_session });
        }
        chains.push(chain.clone());
        let mut tree = FrameTree {
            ordinal,
            parent: p.parent,
            owner_backend: owners.get(&raw.id).copied(),
            origin: raw.origin.clone(),
            cross_origin: raw.origin != main.origin,
            ..Default::default()
        };
        if ordinal > MAX_FRAMES || p.depth > MAX_FRAME_DEPTH {
            tree.note = Some("not shown: over the frame limit".to_string());
            trees.push(tree);
            continue;
        }
        let mut on = root.in_session(raw.session.as_deref());
        if enabled.insert(raw.session.clone()) {
            let _ = on.call("Accessibility.enable", json!({})).await;
        }
        // The root frame of an attached session is the whole of that session's tree.
        let is_session_root = raw.session.is_some() && targets.iter().any(|t| t.target_id == raw.id);
        let params = if is_session_root { json!({}) } else { json!({ "frameId": raw.id }) };
        match on.call("Accessibility.getFullAXTree", params).await {
            Ok(t) => {
                tree.nodes = t.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
                for (backend, name) in nodes_needing_facts(&tree.nodes) {
                    if *describes_left == 0 {
                        break;
                    }
                    *describes_left -= 1;
                    if let Some(f) = super::act::describe_markup(&mut on, backend).await {
                        tree.facts.insert(backend, f.with_label(&name));
                    }
                }
                refs.insert(
                    ordinal,
                    FrameRef {
                        frame_id: raw.id.clone(),
                        session: raw.session.clone(),
                        origin: raw.origin.clone(),
                        url: raw.url.clone(),
                        chain,
                    },
                );
            }
            Err(e) => {
                let why: String = e.chars().take(120).collect();
                tree.note = Some(format!("not readable: {why}"));
            }
        }
        trees.push(tree);
    }
    (trees, refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: &str, parent: Option<&str>, origin: &str, session: Option<&str>) -> RawFrame {
        RawFrame {
            id: id.into(),
            parent_id: parent.map(str::to_string),
            url: format!("{origin}/{id}"),
            origin: origin.into(),
            session: session.map(str::to_string),
        }
    }

    #[test]
    fn a_frame_tree_flattens_root_first_with_parents_and_origins() {
        let tree = json!({ "frameTree": {
            "frame": { "id": "M", "url": "http://127.0.0.1:5311/index.html", "securityOrigin": "http://127.0.0.1:5311" },
            "childFrames": [
                { "frame": { "id": "A", "parentId": "M", "url": "http://127.0.0.1:5311/inner.html", "securityOrigin": "http://127.0.0.1:5311" },
                  "childFrames": [ { "frame": { "id": "A1", "parentId": "A", "url": "about:blank", "securityOrigin": "://" } } ] },
                { "frame": { "id": "B", "parentId": "M", "url": "http://localhost:5312/inner.html", "securityOrigin": "http://localhost:5312" } },
            ]
        }});
        let f = flatten(&tree, Some("S1"));
        let ids: Vec<&str> = f.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["M", "A", "A1", "B"]);
        assert_eq!(f[0].parent_id, None);
        assert_eq!(f[1].parent_id.as_deref(), Some("M"));
        assert_eq!(f[3].origin, "http://localhost:5312");
        assert_eq!(f[1].session.as_deref(), Some("S1"));
        assert_eq!(f[2].origin, "", "an opaque origin with no URL scheme is empty, not \"://\"");
    }

    #[test]
    fn an_origin_falls_back_to_the_url() {
        assert_eq!(origin_of("https://forms.example.gov:8443/a/b?c#d"), "https://forms.example.gov:8443");
        assert_eq!(origin_of("about:blank"), "");
    }

    #[test]
    fn frames_are_ordered_depth_first_by_their_iframe_elements_position() {
        let frames = vec![
            frame("M", None, "http://a", None),
            frame("B", Some("M"), "http://b", Some("S1")),
            frame("A", Some("M"), "http://a", None),
            frame("A1", Some("A"), "http://a", None),
            frame("B1", Some("B"), "http://c", Some("S2")),
        ];
        // A's iframe was created before B's; A1 inside A, B1 inside B.
        let owners = HashMap::from([("A".to_string(), 10), ("B".to_string(), 20), ("A1".to_string(), 11), ("B1".to_string(), 21)]);
        let placed = arrange(&frames, &owners);
        let order: Vec<(&str, usize, usize)> = placed.iter().map(|p| (frames[p.index].id.as_str(), p.parent, p.depth)).collect();
        assert_eq!(order, [("A", 0, 1), ("A1", 1, 2), ("B", 0, 1), ("B1", 3, 2)]);
    }

    #[test]
    fn the_same_frames_always_get_the_same_ordinals_whatever_order_they_were_found_in() {
        let owners = HashMap::from([("A".to_string(), 10), ("B".to_string(), 20)]);
        let one = vec![frame("M", None, "x", None), frame("A", Some("M"), "x", None), frame("B", Some("M"), "y", Some("S"))];
        let two = vec![frame("M", None, "x", None), frame("B", Some("M"), "y", Some("S")), frame("A", Some("M"), "x", None)];
        let name = |fs: &Vec<RawFrame>| arrange(fs, &owners).iter().map(|p| fs[p.index].id.clone()).collect::<Vec<_>>();
        assert_eq!(name(&one), name(&two));
        assert_eq!(name(&one), ["A", "B"]);
    }

    #[test]
    fn a_frame_whose_parent_is_unknown_is_left_out() {
        let frames = vec![frame("M", None, "x", None), frame("Z", Some("nobody"), "y", None), frame("A", Some("M"), "x", None)];
        let placed = arrange(&frames, &HashMap::new());
        assert_eq!(placed.len(), 1);
        assert_eq!(frames[placed[0].index].id, "A");
    }

    #[test]
    fn no_frames_below_the_main_one_is_an_empty_arrangement() {
        assert!(arrange(&[frame("M", None, "x", None)], &HashMap::new()).is_empty());
        assert!(arrange(&[], &HashMap::new()).is_empty());
    }

    fn hop(owner: i64, session: Option<&str>) -> FrameHop {
        FrameHop { owner_backend: owner, parent_session: session.map(str::to_string) }
    }

    #[test]
    fn one_hop_is_added_as_it_is() {
        assert_eq!(hops_to_add(&[hop(5, None)]), vec![&hop(5, None)]);
        assert!(hops_to_add(&[]).is_empty());
    }

    #[test]
    fn frames_nested_in_one_process_count_only_the_innermost_box() {
        // main -> A -> B, all in the page's own process: B's iframe box
        // (owner 9, in A) already includes A's offset, so owner 5 is skipped.
        let chain = [hop(5, None), hop(9, None)];
        assert_eq!(hops_to_add(&chain), vec![&hop(9, None)]);
        let deeper = [hop(5, None), hop(9, None), hop(12, None)];
        assert_eq!(hops_to_add(&deeper), vec![&hop(12, None)]);
    }

    #[test]
    fn crossing_into_another_process_starts_a_new_measurement() {
        // main -> A (same process) -> B (own process S) -> C (inside S):
        // owner 9 (in A, measured from the page) places B; owner 20 (in B,
        // measured from B) places C. Owner 5 is inside owner 9's box.
        let chain = [hop(5, None), hop(9, None), hop(20, Some("S"))];
        assert_eq!(hops_to_add(&chain), vec![&hop(9, None), &hop(20, Some("S"))]);
        // main -> B (S) -> C (S) -> D (S): one run in S, the last counts.
        let chain = [hop(7, None), hop(8, Some("S")), hop(9, Some("S"))];
        assert_eq!(hops_to_add(&chain), vec![&hop(7, None), &hop(9, Some("S"))]);
    }

    #[test]
    fn value_fields_and_buttons_need_facts_and_carry_their_name() {
        let nodes = vec![
            json!({ "role": { "value": "textbox" }, "name": { "value": "City" }, "backendDOMNodeId": 5 }),
            json!({ "role": { "value": "button" }, "name": { "value": "Next" }, "backendDOMNodeId": 6 }),
            json!({ "role": { "value": "heading" }, "name": { "value": "Hi" }, "backendDOMNodeId": 7 }),
            json!({ "role": { "value": "textbox" }, "name": { "value": "No node" } }),
        ];
        assert_eq!(nodes_needing_facts(&nodes), vec![(5, "City".to_string()), (6, "Next".to_string())]);
    }
}
