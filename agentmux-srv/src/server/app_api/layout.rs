// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `layout.save` — write one window as an `agentmux.layout` file
//! (docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md §6.1). The walk and the
//! per-view allowlist live in `backend::layout_file`; this is the RPC shell.
//!
//! The destination normally comes from the host's Save dialog. The handler
//! still refuses anything but an absolute `*.agentmux-layout.json` path: a
//! caller holding the auth key could otherwise use this to overwrite any
//! file the user can write.

use super::*;
use crate::backend::layout_file;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let mstore = state.mstore.clone();
    engine.register_typed(COMMAND_LAYOUT_SAVE, move |req: CommandLayoutSaveData, _ctx| {
        let mstore = mstore.clone();
        async move {
            // Store reads and a file write: off the async worker.
            tokio::task::spawn_blocking(move || layout_save_impl(&mstore, req))
                .await
                .map_err(|e| format!("layout.save: task: {e}"))?
        }
    });
}

pub(super) fn layout_save_impl(store: &Store, req: CommandLayoutSaveData) -> Result<LayoutSaveResult, String> {
    let path = std::path::PathBuf::from(&req.path);
    if !layout_file::is_layout_path(&path) {
        return Err(format!(
            "layout.save: the path must be absolute and end in {}",
            layout_file::LAYOUT_EXTENSION
        ));
    }
    let name = req.name.trim();
    let name = if name.is_empty() { "Layout" } else { name };
    let ctx = layout_file::ExportContext::live(req.include_resume.unwrap_or(false));
    let exported =
        layout_file::export_window(store, &req.window_id, name, &ctx).map_err(|e| format!("layout.save: {e}"))?;
    layout_file::write_layout_file(&path, &exported.doc).map_err(|e| format!("layout.save: {e}"))?;
    Ok(LayoutSaveResult { path: path.to_string_lossy().to_string(), warnings: exported.warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::{Block, LayoutState, Tab, Window, Workspace};
    use agentmux_common::layout_types::{LayoutNode, LayoutNodeData};

    fn one_pane_window(store: &Store) {
        let mut b = Block { oid: "b1".into(), ..Default::default() };
        b.meta.insert("view".into(), serde_json::json!("sysinfo"));
        store.insert(&mut b).unwrap();
        let mut l = LayoutState {
            oid: "l1".into(),
            rootnode: Some(LayoutNode {
                id: "n1".into(),
                data: Some(LayoutNodeData { block_id: "b1".into(), ..Default::default() }),
                ..Default::default()
            }),
            ..Default::default()
        };
        store.insert(&mut l).unwrap();
        let mut t = Tab { oid: "t1".into(), name: "T".into(), layoutstate: "l1".into(), blockids: vec!["b1".into()], ..Default::default() };
        store.insert(&mut t).unwrap();
        let mut ws = Workspace { oid: "ws1".into(), tabids: vec!["t1".into()], activetabid: "t1".into(), ..Default::default() };
        store.insert(&mut ws).unwrap();
        let mut w = Window { oid: "w1".into(), workspaceid: "ws1".into(), ..Default::default() };
        store.insert(&mut w).unwrap();
    }

    fn req(path: &std::path::Path, name: &str) -> CommandLayoutSaveData {
        CommandLayoutSaveData {
            window_id: "w1".into(),
            name: name.into(),
            path: path.to_string_lossy().to_string(),
            include_resume: None,
        }
    }

    #[test]
    fn saves_the_window_to_the_requested_file() {
        let store = Store::open_in_memory().unwrap();
        one_pane_window(&store);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mine.agentmux-layout.json");
        let out = layout_save_impl(&store, req(&path, "  Mine  ")).unwrap();
        assert!(out.warnings.is_empty());
        let doc = layout_file::parse_layout(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc.name, "Mine", "the name is trimmed");
        assert_eq!(doc.windows[0].tabs[0].name, "T");
    }

    #[test]
    fn a_blank_name_becomes_layout() {
        let store = Store::open_in_memory().unwrap();
        one_pane_window(&store);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.agentmux-layout.json");
        layout_save_impl(&store, req(&path, "  ")).unwrap();
        assert_eq!(layout_file::parse_layout(&std::fs::read_to_string(&path).unwrap()).unwrap().name, "Layout");
    }

    #[test]
    fn refuses_any_other_destination_and_writes_nothing() {
        let store = Store::open_in_memory().unwrap();
        one_pane_window(&store);
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("settings.json");
        std::fs::write(&target, "keep me").unwrap();
        assert!(layout_save_impl(&store, req(&target, "x")).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
    }

    #[test]
    fn deserializes_the_documented_request_shape() {
        let r: CommandLayoutSaveData = serde_json::from_value(serde_json::json!({
            "window_id": "w1", "name": "n", "path": "/tmp/a.agentmux-layout.json"
        }))
        .unwrap();
        assert_eq!(r.include_resume, None);
    }
}
