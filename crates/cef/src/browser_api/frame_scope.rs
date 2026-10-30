// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A snapshot scoped to an element inside an iframe (`scope: "f1e12"`): the
//! element's part of that frame's tree, its references prefixed with the
//! scope's (`f1e12.e1`) and acting in the same frame
//! (docs/specs/PLAN_BROWSER_PANE_FORM_FILLING_FIXES_2026_10_17.md §3.3).

use serde_json::Value;

use super::act::SnapshotData;
use super::cdp::CdpSession;
use super::frames::{self, FrameRef};
use super::ref_tables::RefTables;
use super::snapshot;

/// Reads the frame holding `scope_ref` and renders the part of its tree under
/// the element `backend`. The references go into the pane's table beside the
/// full snapshot's, each with the frame it is in. Iframes nested inside the
/// scope show as their element only: scope to an element in one to read it.
pub(super) async fn scoped_snapshot(
    cdp: &mut CdpSession,
    tables: &RefTables,
    block_id: &str,
    epoch: u64,
    url: &str,
    scope_ref: &str,
    backend: i64,
    frame: &FrameRef,
    allowed: Option<&[String]>,
) -> Result<SnapshotData, String> {
    frames::check_frame(cdp, frame, scope_ref).await?;
    let mut describes_left = super::act::MAX_DESCRIBES;
    let (trees, refs) = frames::read_frames(cdp, &mut describes_left, allowed).await;
    let ordinal = refs
        .iter()
        .find(|(_, r)| r.frame_id == frame.frame_id && r.session == frame.session)
        .map(|(ordinal, _)| *ordinal)
        .ok_or_else(|| format!("the frame holding {scope_ref} couldn't be read; take a new snapshot"))?;
    let tree = trees.iter().find(|t| t.ordinal == ordinal).ok_or_else(|| format!("the frame holding {scope_ref} couldn't be read; take a new snapshot"))?;
    if tree.blocked {
        return Err(format!("the frame holding {scope_ref} isn't on this pane's allowed sites, so it can't be read for acting: ask the user to allow it"));
    }
    if let Some(note) = &tree.note {
        return Err(format!("the frame holding {scope_ref} is {note}"));
    }
    let present = tree.nodes.iter().any(|n| n.get("backendDOMNodeId").and_then(Value::as_i64) == Some(backend));
    if !present {
        return Err(format!("the element for {scope_ref} is gone from its frame; take a new snapshot"));
    }
    let rendered = snapshot::render_with_frames(&tree.nodes, &tree.facts, Some(backend), &format!("{scope_ref}."), &[]);
    let framed: Vec<(String, FrameRef)> = rendered.refs.iter().map(|(r, _)| (r.clone(), frame.clone())).collect();
    if !tables.merge_framed_at(block_id, Some(epoch), url, &rendered.refs, &framed) {
        return Err("the page closed while this was being read; take a new snapshot".to_string());
    }
    Ok(SnapshotData { url: url.to_string(), refs: rendered.refs.len(), truncated: rendered.truncated, snapshot: rendered.text })
}
