// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for the generic `fs.*` command family behind the Files pane
//! ("Hangar", view type `files`).
//!
//! Unlike the editor's file-tree commands (`editor.rs`, `editor_read.rs`),
//! these are named for what they do rather than for their first consumer, so
//! the editor tree can move onto them later without a second set of names.
//!
//! Every path the server sends back is in display form: on Windows the
//! `\\?\` verbatim prefix that `canonicalize` adds is stripped (and
//! `\\?\UNC\` turned back into `\\`), so the frontend never sees one.
//!
//! Spec: docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.3, §7, §9.

use serde::{Deserialize, Serialize};

/// Request for `fs.list`.
///
/// The first page names the directory (`path`, which may start with `~`).
/// Later pages pass the `cursor` the previous page returned; `path` is then
/// only used to label an `expired` answer.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsListReq {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cursor: Option<String>,
    /// Entries per page. Default 1000, capped at 5000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub limit: Option<u32>,
}

/// One row of an `fs.list` page.
///
/// `size` and `mtime` are absent rather than zero when unknown, for the same
/// reason as `DirEntry`'s: `0` would read as a real empty file or a real epoch
/// timestamp. A directory never has a `size`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsEntry {
    pub name: String,
    /// Follows symlinks: a link to a directory reads as a directory.
    pub is_dir: bool,
    /// The entry's own type, not followed. Windows junctions count as links.
    pub is_symlink: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub size: Option<u64>,
    /// Unix millis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub mtime: Option<u64>,
    /// A dot-prefixed name, or (Windows) the hidden or system attribute.
    pub hidden: bool,
    /// The read-only permission/attribute. Always false for a folder on
    /// Windows, which ignores the attribute on folders.
    pub readonly: bool,
    /// Where a symlink points, as stored in the link (not resolved).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub link_target: Option<String>,
    /// Why this entry's details couldn't be read (a dangling link, a file
    /// that vanished mid-listing, access denied). The row is still listed;
    /// `size`/`mtime` are then absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

/// Why a listing could not be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum FsErrorKind {
    NotFound,
    PermissionDenied,
    /// macOS only: `EPERM` rather than `EACCES`, which is how a privacy
    /// (TCC) denial shows up. Spec §9.1.4 item 4.
    OsBlocked,
    NotADirectory,
    /// The `cursor` is unknown or its 30 s idle timeout passed. Re-list from
    /// the first page.
    Expired,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsError {
    pub kind: FsErrorKind,
    /// A sentence fit to show the user.
    pub message: String,
}

/// Response for `fs.list`.
///
/// Failing to open the directory is not an RPC error: the answer is `Ok` with
/// no entries, no cursor, and `error` set, so the pane can show a distinct
/// state (spec §9: errors are per entry or per folder, never a failed pane).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsListResult {
    /// The canonical directory, in display form, on every page.
    pub path: String,
    /// Unsorted; the frontend sorts.
    pub entries: Vec<FsEntry>,
    /// Absent once the listing is complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<FsError>,
}

/// Request for `fs.places`, which takes no argument.
///
/// A struct registered as `Option<Self>`, like `EditorRootsReq`: the stub
/// sends `{}` and a client that omits `data` sends `null`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsPlacesReq {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum FsPlaceKind {
    Home,
    /// Desktop, Documents, Downloads, Pictures.
    Known,
    /// A drive letter (Windows) or a mount (Linux). None on macOS.
    Drive,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsPlace {
    /// Stable within one answer: `home`, `desktop`, `documents`,
    /// `downloads`, `pictures`, or `drive:<path>`.
    pub id: String,
    pub label: String,
    pub path: String,
    pub kind: FsPlaceKind,
}

/// Response for `fs.places`.
///
/// Known folders are listed by name only; the server never reads or stats
/// them, because on macOS that alone can raise a privacy prompt the user did
/// not ask for (spec §9.1.5).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsPlacesResult {
    pub home: String,
    /// The platform's path separator: `\` on Windows, `/` elsewhere.
    pub sep: String,
    pub places: Vec<FsPlace>,
}

/// Request for `fs.watch`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsWatchReq {
    pub path: String,
    /// Scopes the `files:changed` event to `block:<block_id>`.
    pub block_id: String,
}

/// Response for `fs.watch`. Pass `watch_id` to `fs.unwatch`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsWatchResult {
    pub watch_id: String,
}

/// Request for `fs.unwatch`. An unknown id is not an error.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsUnwatchReq {
    pub watch_id: String,
}

/// The empty answer of `fs.unwatch`, `fs.open` and `fs.reveal`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsEmptyResult {}

/// Request for `fs.rename`: a new name in the same folder.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsRenameReq {
    pub path: String,
    /// A plain name: no separators, not `.` or `..`, and on Windows none of
    /// the names or characters Windows refuses or silently alters.
    pub new_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsRenameResult {
    pub new_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(rename_all = "snake_case")]
pub enum FsCreateKind {
    File,
    Dir,
}

/// Request for `fs.create`: a new empty file or folder. Fails if the name is
/// taken.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsCreateReq {
    pub parent: String,
    pub name: String,
    pub kind: FsCreateKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsCreateResult {
    pub path: String,
}

/// Request for `fs.trash`: move each path to the OS Trash / Recycle Bin.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsTrashReq {
    pub paths: Vec<String>,
}

/// Request for `fs.restore`: put each path, trashed earlier, back where it
/// was. The paths are the ORIGINAL locations, as `fs.trash` was given them.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsRestoreReq {
    pub paths: Vec<String>,
}

/// Request for `fs.delete`: PERMANENT removal. A symlink is unlinked, never
/// followed.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsDeleteReq {
    pub paths: Vec<String>,
}

/// One path's outcome in a batch operation.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsOpResult {
    /// The path as the request named it.
    pub path: String,
    pub ok: bool,
    /// A sentence fit to show the user. Absent when `ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

/// Response for `fs.trash`, `fs.restore` and `fs.delete`: one result per
/// requested path, in request order. One path failing never stops the rest.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsOpResults {
    pub results: Vec<FsOpResult>,
}

/// Request for `fs.open` (the OS default application) and `fs.reveal` (select
/// in the OS file manager).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct FsPathReq {
    pub path: String,
}
