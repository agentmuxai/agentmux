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

/// A frame is cross-origin when it differs from the page it is shown on, or
/// from the frame that holds it: a same-site frame inside a frame from another
/// site is still sitting in that site's page.
fn is_cross_origin(origin: &str, top: &str, parent: Option<&str>) -> bool {
    origin != top || parent.is_some_and(|p| p != origin)
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
    chain_offset(root, &f.chain).await
}

/// [`frame_offset`] for a frame known only by its chain.
pub async fn chain_offset(root: &mut CdpSession, chain: &[FrameHop]) -> Result<(f64, f64), String> {
    let (mut x, mut y) = (0.0, 0.0);
    for hop in hops_to_add(chain) {
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

/// Where a frame is on the page, and whether a point inside it is placed by
/// adding an offset (every `<iframe>` on the way is an unscaled, unrotated
/// box).
#[derive(Debug, Clone, PartialEq)]
pub struct ChainGeometry {
    /// The frame's content origin in the main frame's CSS pixels.
    pub offset: (f64, f64),
    /// The box around the frame's `<iframe>` content on the page (x, y, w, h).
    pub bounds: [f64; 4],
    /// The frame's own `<iframe>` is a plain box.
    pub plain: bool,
    /// Every outer `<iframe>` is a plain box, so `bounds` is right even when
    /// the frame's own isn't.
    pub outer_plain: bool,
}

/// The eight numbers of a quad as four points.
fn points(q: &[f64]) -> Option<[(f64, f64); 4]> {
    (q.len() >= 8).then(|| [(q[0], q[1]), (q[2], q[3]), (q[4], q[5]), (q[6], q[7])])
}

/// The axis-aligned box around a quad (x, y, w, h).
fn quad_bounds(q: &[f64]) -> Option<[f64; 4]> {
    let p = points(q)?;
    let (xs, ys) = (p.map(|p| p.0), p.map(|p| p.1));
    let (x0, x1) = (xs.iter().copied().fold(f64::MAX, f64::min), xs.iter().copied().fold(f64::MIN, f64::max));
    let (y0, y1) = (ys.iter().copied().fold(f64::MAX, f64::min), ys.iter().copied().fold(f64::MIN, f64::max));
    Some([x0, y0, x1 - x0, y1 - y0])
}

/// A box model whose element is neither rotated, skewed nor scaled: its border
/// quad is an axis-aligned rectangle the size of its layout box.
fn is_plain_box(border: &[f64], width: f64, height: f64) -> bool {
    let Some([a, b, c, d]) = points(border) else { return false };
    let close = |x: f64, y: f64| (x - y).abs() < 0.5;
    close(a.1, b.1) && close(b.0, c.0) && close(c.1, d.1) && close(d.0, a.0) && close(b.0 - a.0, width) && close(d.1 - a.1, height)
}

/// Where a frame is drawn, if it is.
#[derive(Debug, Clone, PartialEq)]
pub enum Placement {
    Shown(ChainGeometry),
    /// An `<iframe>` on the way isn't drawn at all (`display: none`, or inside
    /// something hidden): nothing in the frame appears on the page.
    NotRendered,
    /// It couldn't be worked out.
    Unknown,
}

/// A `DOM.getContentQuads` answer that says the element isn't drawn.
fn has_no_quads(reply: &Value) -> bool {
    reply.get("quads").and_then(Value::as_array).is_some_and(|q| q.is_empty())
}

/// [`ChainGeometry`] for a frame known by its chain, or why there is none.
pub async fn chain_geometry(root: &mut CdpSession, chain: &[FrameHop]) -> Placement {
    match chain_geometry_inner(root, chain).await {
        Ok(Some(g)) => Placement::Shown(g),
        Ok(None) => Placement::Unknown,
        Err(placement) => placement,
    }
}

async fn chain_geometry_inner(root: &mut CdpSession, chain: &[FrameHop]) -> Result<Option<ChainGeometry>, Placement> {
    let hops = hops_to_add(chain);
    let (mut ox, mut oy) = (0.0, 0.0);
    let mut outer_plain = true;
    for (i, hop) in hops.iter().enumerate() {
        let mut on = root.in_session(hop.parent_session.as_deref());
        let model = match on.call("DOM.getBoxModel", json!({ "backendNodeId": hop.owner_backend })).await {
            Ok(m) => m,
            // No box: either it isn't drawn (no content quads), or it is unknown.
            Err(_) => {
                let quads = on.call("DOM.getContentQuads", json!({ "backendNodeId": hop.owner_backend })).await;
                return Err(match quads {
                    Ok(q) if has_no_quads(&q) => Placement::NotRendered,
                    _ => Placement::Unknown,
                });
            }
        };
        let Some(m) = model.get("model") else { return Ok(None) };
        let parts = || -> Option<(Vec<f64>, Vec<f64>, f64, f64)> {
            let content: Vec<f64> = m.get("content")?.as_array()?.iter().filter_map(Value::as_f64).collect();
            let border: Vec<f64> = m.get("border")?.as_array()?.iter().filter_map(Value::as_f64).collect();
            Some((content, border, m.get("width")?.as_f64()?, m.get("height")?.as_f64()?))
        };
        let Some((content, border, width, height)) = parts() else { return Ok(None) };
        let plain = is_plain_box(&border, width, height);
        let Some([x, y, w, h]) = quad_bounds(&content) else { return Ok(None) };
        if i + 1 < hops.len() {
            outer_plain &= plain;
            ox += content[0];
            oy += content[1];
        } else {
            return Ok(Some(ChainGeometry { offset: (ox + content[0], oy + content[1]), bounds: [x + ox, y + oy, w, h], plain, outer_plain }));
        }
    }
    Ok(None)
}

/// A frame's place on the page, without its contents: enough to find it,
/// place a point inside it on the page, and label it.
#[derive(Debug, Clone)]
pub struct FrameLayout {
    /// `f{ordinal}`, 1.. in document order.
    pub ordinal: usize,
    /// The ordinal of the frame holding its `<iframe>` (0: the top).
    pub parent: usize,
    pub depth: usize,
    pub raw: RawFrame,
    /// The `<iframe>` element's backend node id, in the parent's tree.
    pub owner: Option<i64>,
    pub chain: Vec<FrameHop>,
    /// It is the root frame of its own session (a frame in another process).
    pub is_session_root: bool,
    /// Its own origin, or for an `about:` frame the one it inherits.
    pub origin: String,
    pub cross_origin: bool,
    /// Its `<iframe>` element, and every ancestor's, was found, so `chain`
    /// places it on the page. When false, a point inside it can't be placed.
    pub placeable: bool,
}

/// The origin a frame runs with: an `about:blank` or `about:srcdoc` frame (or
/// one whose URL hasn't been set yet) reports none of its own and inherits its
/// parent's.
fn effective_origin(raw: &RawFrame, parent_origin: &str) -> String {
    if raw.origin.is_empty() && (raw.url.is_empty() || raw.url.starts_with("about:")) {
        parent_origin.to_string()
    } else {
        raw.origin.clone()
    }
}

/// Every iframe of the pane, in document order, placed and labelled.
pub async fn layout(root: &mut CdpSession) -> Vec<FrameLayout> {
    layout_counting_unplaced(root).await.0
}

/// [`layout`], and how many frames it had to leave out because their parent
/// frame isn't known (so they can't be shown, placed or hidden).
pub async fn layout_counting_unplaced(root: &mut CdpSession) -> (Vec<FrameLayout>, usize) {
    let (all, targets) = current_frames(root).await;
    let Some(main) = all.first().cloned() else {
        return (Vec::new(), 0);
    };
    if all.len() == 1 {
        return (Vec::new(), 0);
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
    let mut out: Vec<FrameLayout> = Vec::new();
    for p in arrange(&all, &owners) {
        let raw = all[p.index].clone();
        let parent_session = all.iter().find(|f| Some(f.id.as_str()) == raw.parent_id.as_deref()).and_then(|f| f.session.clone());
        let (mut chain, parent_origin, parent_placeable) = if p.parent == 0 {
            (Vec::new(), main.origin.clone(), true)
        } else {
            let holder = &out[p.parent - 1];
            (holder.chain.clone(), holder.origin.clone(), holder.placeable)
        };
        let placeable = parent_placeable && owners.contains_key(&raw.id);
        if let Some(&owner) = owners.get(&raw.id) {
            chain.push(FrameHop { owner_backend: owner, parent_session });
        }
        let origin = effective_origin(&raw, &parent_origin);
        let cross_origin = is_cross_origin(&origin, &main.origin, Some(&parent_origin));
        let is_session_root = raw.session.is_some() && targets.iter().any(|t| t.target_id == raw.id);
        out.push(FrameLayout {
            ordinal: out.len() + 1,
            parent: p.parent,
            depth: p.depth,
            owner: owners.get(&raw.id).copied(),
            raw,
            chain,
            is_session_root,
            origin,
            cross_origin,
            placeable,
        });
    }
    let unplaced = (all.len() - 1).saturating_sub(out.len());
    (out, unplaced)
}

/// Read every iframe of the pane: each one's accessibility tree and the
/// references' routes back to their elements. `describes_left` is the shared
/// budget of DOM lookups (the top frame spent some).
pub async fn read_frames(root: &mut CdpSession, describes_left: &mut usize) -> (Vec<FrameTree>, HashMap<usize, FrameRef>) {
    let mut trees = Vec::new();
    let mut refs = HashMap::new();
    let mut enabled: HashSet<Option<String>> = HashSet::new();
    for fl in layout(root).await {
        let mut tree = FrameTree {
            ordinal: fl.ordinal,
            parent: fl.parent,
            owner_backend: fl.owner,
            origin: fl.origin.clone(),
            cross_origin: fl.cross_origin,
            ..Default::default()
        };
        if fl.ordinal > MAX_FRAMES || fl.depth > MAX_FRAME_DEPTH {
            tree.note = Some("not shown: over the frame limit".to_string());
            trees.push(tree);
            continue;
        }
        let mut on = root.in_session(fl.raw.session.as_deref());
        if enabled.insert(fl.raw.session.clone()) {
            let _ = on.call("Accessibility.enable", json!({})).await;
        }
        // The root frame of an attached session is the whole of that session's tree.
        let params = if fl.is_session_root { json!({}) } else { json!({ "frameId": fl.raw.id }) };
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
                    fl.ordinal,
                    FrameRef {
                        frame_id: fl.raw.id.clone(),
                        session: fl.raw.session.clone(),
                        origin: fl.origin.clone(),
                        url: fl.raw.url.clone(),
                        chain: fl.chain.clone(),
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
    fn a_frame_is_cross_origin_to_the_page_or_to_the_frame_holding_it() {
        let page = "http://a.test";
        assert!(!is_cross_origin(page, page, Some(page)), "same as both");
        assert!(is_cross_origin("http://b.test", page, Some(page)), "differs from the page");
        assert!(is_cross_origin(page, page, Some("http://b.test")), "same as the page, inside another site's frame");
        assert!(is_cross_origin("http://b.test", page, Some("http://b.test")), "differs from the page");
        assert!(!is_cross_origin(page, page, None), "parent unknown: judged against the page only");
    }

    #[test]
    fn an_about_frame_inherits_its_parents_origin() {
        let srcdoc = RawFrame { id: "F".into(), parent_id: Some("M".into()), url: "about:srcdoc".into(), origin: String::new(), session: None };
        assert_eq!(effective_origin(&srcdoc, "https://forms.example.gov"), "https://forms.example.gov");
        let blank = RawFrame { url: String::new(), ..srcdoc.clone() };
        assert_eq!(effective_origin(&blank, "https://a.test"), "https://a.test");
        let own = RawFrame { url: "https://b.test/x".into(), origin: "https://b.test".into(), ..srcdoc.clone() };
        assert_eq!(effective_origin(&own, "https://a.test"), "https://b.test", "a frame with its own origin keeps it");
        let opaque = RawFrame { url: "data:text/html,hi".into(), origin: String::new(), ..srcdoc };
        assert_eq!(effective_origin(&opaque, "https://a.test"), "", "only about: frames inherit");
    }

    #[test]
    fn a_plain_box_is_axis_aligned_and_its_layout_size() {
        let border = [10.0, 20.0, 110.0, 20.0, 110.0, 70.0, 10.0, 70.0];
        assert!(is_plain_box(&border, 100.0, 50.0));
        assert!(!is_plain_box(&border, 50.0, 25.0), "drawn at twice its layout size: scaled");
        let rotated = [60.0, 0.0, 110.0, 50.0, 60.0, 100.0, 10.0, 50.0];
        assert!(!is_plain_box(&rotated, 70.7, 70.7));
        assert!(!is_plain_box(&[1.0, 2.0], 1.0, 1.0), "not a quad");
    }

    #[test]
    fn an_element_with_no_content_quads_is_not_drawn() {
        assert!(has_no_quads(&json!({ "quads": [] })));
        assert!(!has_no_quads(&json!({ "quads": [[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]] })));
        assert!(!has_no_quads(&json!({})), "an answer without quads is unknown, not hidden");
    }

    #[test]
    fn the_bounds_of_a_rotated_quad_cover_all_its_corners() {
        let rotated = [60.0, 0.0, 110.0, 50.0, 60.0, 100.0, 10.0, 50.0];
        assert_eq!(quad_bounds(&rotated), Some([10.0, 0.0, 100.0, 100.0]));
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
