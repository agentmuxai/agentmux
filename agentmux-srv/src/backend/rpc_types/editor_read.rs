// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the editor pane's read commands.
//!
//! Like the mutation commands in `editor.rs`, every one of these was a
//! `struct Cmd` declared inside its handler's closure with an inline
//! `json!({..})` response, so none of them could be generated from.
//!
//! Specs: `docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md`,
//! `docs/specs/SPEC_EDITOR_FILE_ENCODINGS_2026_06_17.md`.

use serde::{Deserialize, Serialize};

/// Request for `readeditorfile`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandReadEditorFileData {
    pub path: String,
}

/// Response for `readeditorfile`.
///
/// The four encoding fields are NOT optional, though the hand-written
/// declaration marked them `?:` "for back-compat; absent ⇒ treat as UTF-8".
/// `decode_file` returns all four unconditionally and the handler writes all
/// four every time, so the server has never omitted them — the optionality was
/// only ever true of a client reading a response from a build that predates
/// SPEC_EDITOR_FILE_ENCODINGS, which is not a thing the binding describes.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandReadEditorFileResult {
    pub content: String,
    /// Encoding label (WHATWG / `encoding_rs` name, e.g. "windows-1252").
    pub encoding: String,
    /// The BOM that was present: "none" | "utf-8" | "utf-16le" | "utf-16be".
    pub bom: String,
    /// "crlf" if any CRLF was present, else "lf".
    pub line_ending: String,
    /// True when decoding inserted U+FFFD replacements, i.e. the detected
    /// encoding is probably wrong.
    pub had_decode_errors: bool,
    pub read_only: bool,
}

/// Request for `writeeditorfile`.
///
/// The three encoding fields ARE optional here, and for a real reason rather
/// than back-compat hedging: omitting them means "write UTF-8/no BOM/LF",
/// which is what a caller that never read the file back wants. They are
/// `Option<T>` in Rust, so ts-rs marks them optional without help.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandWriteEditorFileData {
    pub path: String,
    pub content: String,
    #[serde(default)]
    #[ts(optional)]
    pub encoding: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub bom: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub line_ending: Option<String>,
}

/// Request for `listeditordir`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ListEditorDirReq {
    pub path: String,
}

/// One editor file-tree row.
///
/// `size` and `mtime` are genuinely absent rather than zero: directories have
/// no meaningful size, and `modified()` is not available on every platform or
/// filesystem. Sending `0` would be indistinguishable from a real empty file
/// or a real epoch timestamp, so they stay omitted.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct DirEntry {
    pub name: String,
    /// Follows symlinks, so a symlink to a directory reads as a directory —
    /// which is what VS Code does and what the tree renders.
    pub is_dir: bool,
    /// Does NOT follow symlinks; this is the entry's own type.
    pub is_symlink: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub size: Option<u64>,
    /// Unix millis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub mtime: Option<u64>,
}

/// Response for `listeditordir`. `path` is the canonicalized directory, which
/// is not necessarily the `path` the caller sent — `~` and symlinks are
/// resolved.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ListEditorDirResult {
    pub path: String,
    pub entries: Vec<DirEntry>,
}

/// Request for `geteditorhome` / `geteditorroots`, which ignore their payload.
///
/// A struct rather than `()`, registered as `Option<Self>`: the stub sends
/// `{}` and a client that omits `data` sends `null`, and serde accepts each of
/// those from only one of `()` and a struct.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct EditorRootsReq {}

/// Response for `geteditorhome`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct GetEditorHomeResult {
    pub home: String,
}

/// One extra top-level root beside `$HOME` — a Windows drive letter, or a
/// Linux mount under /mnt, /media or /Volumes. Empty on macOS, where the
/// file-tree is scoped to `$HOME` only.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct EditorDrive {
    pub name: String,
    pub path: String,
}

/// Response for `geteditorroots`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct GetEditorRootsResult {
    pub home: String,
    pub drives: Vec<EditorDrive>,
}

/// Request for `watcheditorfile` AND `unwatcheditorfile`.
///
/// Deliberately one type for both, unlike `CreateEditorFileReq` /
/// `CreateEditorDirReq` which are separate despite having identical fields.
/// The difference is that these two must address the SAME thing: an unwatch
/// that does not name exactly what the watch named leaks a watcher. Sharing
/// the type makes that a compile error rather than a leak.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct WatchEditorFileReq {
    pub path: String,
    /// Scopes the `editor:file_changed` event to `block:<block_id>`, and is
    /// half of the watch key -- two panes watching one path each hold their
    /// own registration.
    pub block_id: String,
}

/// Request for `watchmediadir`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct WatchMediaDirReq {
    pub path: String,
    pub block_id: String,
    /// Lowercase, no leading dot. Only files matching one of these raise
    /// `media:file_changed`.
    pub extensions: Vec<String>,
}

/// Request for `unwatchmediadir`.
///
/// Separate from `WatchMediaDirReq` rather than shared, because unwatching
/// really does take fewer fields: the registration is keyed on
/// (path, block_id) and the extension filter is not part of the key.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct UnwatchMediaDirReq {
    pub path: String,
    pub block_id: String,
}
