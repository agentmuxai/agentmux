// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the editor file-tree mutation commands.
//!
//! Every one of these lived as a `struct Cmd` declared inside its handler's
//! own closure, with the response built by an inline `json!({..})`. Neither
//! form can be named, so neither could be generated from — the frontend
//! restated all fourteen shapes by hand.
//!
//! Specs: `docs/specs/SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md`,
//! `docs/specs/SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md`.

use serde::{Deserialize, Serialize};

/// Request for `openinshell`. Reveals a path in the OS file manager.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct OpenInShellReq {
    pub path: String,
}

/// Request for `renameeditorfile`. `new_name` is a plain filename, not a path
/// — the handler rejects anything containing a separator, so a rename cannot
/// move a file out of its directory.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct RenameEditorFileReq {
    pub old_path: String,
    pub new_name: String,
}

/// Response for `renameeditorfile`. The resolved absolute path, which is not
/// simply `old_path`'s parent joined with `new_name`: `old_path` is
/// canonicalized first, so symlinks are already resolved here.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct RenameEditorFileResult {
    pub new_path: String,
}

/// Request for `createeditorfile`. `name` is a plain filename (see
/// `RenameEditorFileReq`).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CreateEditorFileReq {
    pub parent_path: String,
    pub name: String,
}

/// Response for `createeditorfile`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CreateEditorFileResult {
    pub file_path: String,
}

/// Request for `createeditordir`. Same shape as `CreateEditorFileReq` but a
/// separate type: they are separate commands with separate validation, and
/// sharing one name would make a future divergence look like a bug in the
/// binding rather than in the caller.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CreateEditorDirReq {
    pub parent_path: String,
    pub name: String,
}

/// Response for `createeditordir`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CreateEditorDirResult {
    pub dir_path: String,
}

/// Request for `deleteeditorfile`.
///
/// `recursive` has no `serde(default)` on purpose: defaulting it would make a
/// caller that forgot the field delete a directory tree it never asked to,
/// or fail confusingly on a non-empty one. Requiring it keeps the destructive
/// choice explicit, and the hand-written stub already typed it as required.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct DeleteEditorFileReq {
    pub path: String,
    pub recursive: bool,
}

/// Request for `createscratchfile`. Both fields are genuinely optional —
/// `Option<T>` in Rust, so ts-rs can say so without help.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CreateScratchFileReq {
    #[ts(optional)]
    pub display_name: Option<String>,
    /// Ids the caller already has open, so the generated "Untitled-N" name
    /// does not collide with a buffer that exists only in the renderer.
    #[ts(optional)]
    pub exclude_scratch_ids: Option<Vec<String>>,
}

/// Response for `createscratchfile`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CreateScratchFileResult {
    pub scratch_id: String,
    pub file_path: String,
    /// The name actually assigned, which is not necessarily the requested
    /// `display_name` — collisions get a suffix.
    pub display_name: String,
}

/// Request for `movescratchfile` (the editor's "Save As").
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct MoveScratchFileReq {
    pub scratch_id: String,
    pub destination_path: String,
}

/// Response for `movescratchfile`. The canonical destination, which differs
/// from the requested `destination_path` whenever that path went through a
/// symlink or `~`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct MoveScratchFileResult {
    pub file_path: String,
}
