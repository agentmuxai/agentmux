// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The editor's file RPCs (read, write, list, reveal, rename, create, delete), moved out of
//! `register_editor_handlers` unchanged as named functions. Each mutation takes the user's home folder as a
//! parameter, so a test can point it at a temp dir; the registration passes `dirs::home_dir()`.

use std::path::Path;

use super::{editor_may_change, is_a_home, remote_fs};
use crate::backend::base::expand_home_dir_safe;
use crate::backend::rpc_types::{
    CommandReadEditorFileData, CommandReadEditorFileResult, CommandWriteEditorFileData, CreateEditorDirReq,
    CreateEditorDirResult, CreateEditorFileReq, CreateEditorFileResult, DeleteEditorFileReq, DirEntry, ListEditorDirReq,
    ListEditorDirResult, OpenInShellReq, RenameEditorFileReq, RenameEditorFileResult,
};
use crate::backend::text_encoding::DecodedFile;

/// readeditorfile and writeeditorfile refuse more than this.
const MAX_EDITOR_FILE_BYTES: usize = 10_000_000;

/// The reply to a read: the detected encoding, BOM and line ending ride back so a save round-trips the original
/// format. See docs/specs/SPEC_EDITOR_FILE_ENCODINGS_2026_06_17.md.
fn read_result(decoded: DecodedFile, read_only: bool) -> CommandReadEditorFileResult {
    CommandReadEditorFileResult {
        content: decoded.content,
        encoding: decoded.encoding,
        bom: decoded.bom.to_string(),
        line_ending: decoded.line_ending.to_string(),
        had_decode_errors: decoded.had_decode_errors,
        read_only,
    }
}

/// A file on an SSH host: read there by its helper, decoded here the same way (remote terminals spec §6.3).
pub(super) async fn read_remote_file(
    connection: &str,
    cmd: &CommandReadEditorFileData,
    auth_key: &str,
) -> Result<CommandReadEditorFileResult, String> {
    let r = remote_fs::Remote { connection, block_id: cmd.block_id.as_deref(), auth_key };
    let (bytes, read_only) = remote_fs::read_file(r, &cmd.path).await.map_err(|e| format!("readeditorfile: {e}"))?;
    Ok(read_result(crate::backend::text_encoding::decode_file(&bytes), read_only))
}

/// Reads the bytes and detects the encoding (BOM, then UTF-8, then a heuristic), decoding to UTF-8 for the
/// editor, so non-UTF-8 files (Windows-1252 .ini, UTF-16, Shift_JIS, ...) open instead of erroring.
pub(super) fn read_local_file(path: &str) -> Result<CommandReadEditorFileResult, String> {
    let expanded = expand_home_dir_safe(path);
    let path = expanded.as_path();
    let metadata = std::fs::metadata(path).map_err(|e| format!("readeditorfile: {e}"))?;
    if metadata.len() > MAX_EDITOR_FILE_BYTES as u64 {
        return Err("File too large (>10MB)".to_string());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("readeditorfile: {e}"))?;
    Ok(read_result(crate::backend::text_encoding::decode_file(&bytes), metadata.permissions().readonly()))
}

/// The file's original encoding, BOM and line endings, so a non-UTF-8 file round-trips instead of being
/// silently rewritten as UTF-8. Absent fields default to UTF-8. The bool is true when some character has no
/// representation in that encoding.
fn encode_content(cmd: &CommandWriteEditorFileData) -> (Vec<u8>, bool) {
    crate::backend::text_encoding::encode_file(
        &cmd.content,
        cmd.encoding.as_deref().unwrap_or(""),
        cmd.bom.as_deref().unwrap_or("none"),
        cmd.line_ending.as_deref().unwrap_or("lf"),
    )
}

/// A file on an SSH host: encoded here as for a local file, then written there atomically by its helper.
pub(super) async fn write_remote_file(connection: &str, cmd: &CommandWriteEditorFileData, auth_key: &str) -> Result<(), String> {
    let (out_bytes, had_unmappable) = encode_content(cmd);
    if had_unmappable {
        return Err("writeeditorfile: file contains characters that can't be represented in its encoding — save it as UTF-8 instead".to_string());
    }
    let r = remote_fs::Remote { connection, block_id: cmd.block_id.as_deref(), auth_key };
    remote_fs::write_file(r, &cmd.path, out_bytes).await
}

/// Writes restricted to under the user's home directory: an allowlist, safer than an incomplete denylist.
pub(super) fn write_local_file(home: &Path, cmd: &CommandWriteEditorFileData) -> Result<(), String> {
    let expanded = expand_home_dir_safe(&cmd.path);
    let path = expanded.as_path();
    let canonical_home = home.canonicalize().map_err(|e| format!("writeeditorfile: home dir: {e}"))?;

    // The target path, resolved: canonicalize an existing one, or the parent plus the filename for a new file.
    let canonical = path
        .canonicalize()
        .or_else(|_| {
            path.parent()
                .and_then(|p| p.canonicalize().ok())
                .map(|p| p.join(path.file_name().unwrap_or_default()))
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "invalid path"))
        })
        .map_err(|e| format!("writeeditorfile: {e}"))?;

    if !editor_may_change(&canonical, &canonical_home) {
        return Err(format!("writeeditorfile: path {} is outside home directory", canonical.display()));
    }

    let (out_bytes, had_unmappable) = encode_content(cmd);
    // Refuse a lossy write: rather than silently emit numeric-character-reference replacements (corrupting
    // the file), fail loudly so the user can save as UTF-8. The UI for an in-place "Save with Encoding →
    // UTF-8" is Phase 3 of SPEC_EDITOR_FILE_ENCODINGS.
    if had_unmappable {
        let enc = cmd.encoding.as_deref().filter(|s| !s.is_empty()).unwrap_or("its current encoding");
        return Err(format!("writeeditorfile: file contains characters that can't be represented in {enc} — save it as UTF-8 instead"));
    }
    std::fs::write(&canonical, &out_bytes).map_err(|e| format!("writeeditorfile: {e}"))?;
    tracing::info!(path = %canonical.display(), bytes = out_bytes.len(), "editor file saved");
    Ok(())
}

/// Content past the limit is refused before anything else, remote or local.
pub(super) fn check_write_size(cmd: &CommandWriteEditorFileData) -> Result<(), String> {
    if cmd.content.len() > MAX_EDITOR_FILE_BYTES {
        return Err("Content too large (>10MB)".to_string());
    }
    Ok(())
}

/// Lists a directory for the editor's file-tree pane. Symlinks are followed (matches VS Code semantics; the
/// frontend marks followed symlinks with a ↗ overlay). Spec: docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md
pub(super) fn list_dir(cmd: &ListEditorDirReq) -> Result<ListEditorDirResult, String> {
    let expanded = expand_home_dir_safe(&cmd.path);
    let canonical = expanded.canonicalize().map_err(|e| format!("listeditordir: {e}"))?;
    let read_dir = std::fs::read_dir(&canonical).map_err(|e| format!("listeditordir: {e}"))?;

    let mut entries: Vec<DirEntry> = read_dir.flatten().map(|entry| dir_entry(&entry)).collect();
    // Folders first, then files; alphabetical, case-insensitive.
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(ListEditorDirResult { path: canonical.to_string_lossy().into_owned(), entries })
}

/// `file_type()` is the entry's own type, so a symlink shows as one (no follow); `metadata()` follows symlinks,
/// so a symlinked dir reports `is_dir`. Both are needed: `is_symlink` from the first, `is_dir` and size from
/// the second (a symlink-to-dir reads as a directory, matching VS Code).
fn dir_entry(entry: &std::fs::DirEntry) -> DirEntry {
    let name = entry.file_name().to_string_lossy().to_string();
    let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
    let metadata = entry.metadata().ok();
    let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    let size = metadata.as_ref().and_then(|m| if !m.is_dir() { Some(m.len()) } else { None });
    let mtime = metadata
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    DirEntry { name, is_dir, is_symlink, size, mtime }
}

// ── Editor file-tree mutations ─────────────────────────────────────────────
// Spec: docs/specs/SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md. All validate the target path is under the user's
// home directory before performing any mutation: the same policy as writeeditorfile.

/// A new file or folder name is one plain path component.
fn is_plain_name(name: &str) -> bool {
    !(name.contains('/') || name.contains('\\') || name.contains('\0') || name == "." || name == ".." || name.is_empty())
}

/// Reveals a path in the OS file manager.
pub(super) fn reveal_in_shell(home: &Path, cmd: &OpenInShellReq) -> Result<(), String> {
    let expanded = expand_home_dir_safe(&cmd.path);
    let path = expanded.as_path();
    let canonical_home = home.canonicalize().map_err(|e| format!("openinshell: home: {e}"))?;
    let canonical = path.canonicalize().map_err(|e| format!("openinshell: {e}"))?;
    if !editor_may_change(&canonical, &canonical_home) {
        return Err("openinshell: path outside home directory".to_string());
    }
    // The file manager is third-party and long-lived, and a user can launch anything from it, including another
    // AgentMux build, which would then adopt THIS instance's channel and data dir (invariant I7;
    // retro-env-inheritance-instance-isolation-breach-2026-09-17 §2.2). So: strict policy, not the pane keep-set.
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("explorer.exe");
        crate::backend::pane_env::sanitize_external_std_command(&mut cmd);
        let _ = cmd.arg(format!("/select,{}", canonical.display())).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        crate::backend::pane_env::sanitize_external_std_command(&mut cmd);
        let _ = cmd.arg("-R").arg(&canonical).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let target = if canonical.is_dir() { canonical.clone() } else { canonical.parent().unwrap_or(&canonical).to_path_buf() };
        let mut cmd = std::process::Command::new("xdg-open");
        crate::backend::pane_env::sanitize_external_std_command(&mut cmd);
        let _ = cmd.arg(&target).spawn();
    }
    Ok(())
}

/// Renames a file or folder: the name only, in the same parent directory.
pub(super) fn rename_file(home: &Path, cmd: &RenameEditorFileReq) -> Result<RenameEditorFileResult, String> {
    let expanded = expand_home_dir_safe(&cmd.old_path);
    let canonical_home = home.canonicalize().map_err(|e| format!("renameeditorfile: home: {e}"))?;
    let canonical_old = expanded.as_path().canonicalize().map_err(|e| format!("renameeditorfile: {e}"))?;
    if !editor_may_change(&canonical_old, &canonical_home) || is_a_home(&canonical_old, &canonical_home) {
        return Err("renameeditorfile: path outside or is the home directory".to_string());
    }
    if !is_plain_name(&cmd.new_name) {
        return Err("renameeditorfile: new_name must be a plain filename".to_string());
    }
    let new_path = canonical_old.parent().ok_or("renameeditorfile: no parent directory")?.join(&cmd.new_name);
    if new_path.exists() {
        return Err("renameeditorfile: destination already exists".to_string());
    }
    std::fs::rename(&canonical_old, &new_path).map_err(|e| format!("renameeditorfile: {e}"))?;
    Ok(RenameEditorFileResult { new_path: new_path.to_string_lossy().into_owned() })
}

/// Creates an empty file inside an existing directory.
pub(super) fn create_file(home: &Path, cmd: &CreateEditorFileReq) -> Result<CreateEditorFileResult, String> {
    let expanded = expand_home_dir_safe(&cmd.parent_path);
    let canonical_home = home.canonicalize().map_err(|e| format!("createeditorfile: home: {e}"))?;
    let canonical_parent = expanded.as_path().canonicalize().map_err(|e| format!("createeditorfile: {e}"))?;
    if !editor_may_change(&canonical_parent, &canonical_home) {
        return Err("createeditorfile: path outside home directory".to_string());
    }
    if !is_plain_name(&cmd.name) {
        return Err("createeditorfile: name must be a plain filename".to_string());
    }
    let file_path = canonical_parent.join(&cmd.name);
    if file_path.exists() {
        return Err("createeditorfile: file already exists".to_string());
    }
    std::fs::write(&file_path, "").map_err(|e| format!("createeditorfile: {e}"))?;
    Ok(CreateEditorFileResult { file_path: file_path.to_string_lossy().into_owned() })
}

/// Creates a directory inside an existing directory.
pub(super) fn create_dir(home: &Path, cmd: &CreateEditorDirReq) -> Result<CreateEditorDirResult, String> {
    let expanded = expand_home_dir_safe(&cmd.parent_path);
    let canonical_home = home.canonicalize().map_err(|e| format!("createeditordir: home: {e}"))?;
    let canonical_parent = expanded.as_path().canonicalize().map_err(|e| format!("createeditordir: {e}"))?;
    if !editor_may_change(&canonical_parent, &canonical_home) {
        return Err("createeditordir: path outside home directory".to_string());
    }
    if !is_plain_name(&cmd.name) {
        return Err("createeditordir: name must be a plain name".to_string());
    }
    let dir_path = canonical_parent.join(&cmd.name);
    if dir_path.exists() {
        return Err("createeditordir: already exists".to_string());
    }
    std::fs::create_dir(&dir_path).map_err(|e| format!("createeditordir: {e}"))?;
    Ok(CreateEditorDirResult { dir_path: dir_path.to_string_lossy().into_owned() })
}

/// Deletes a file or directory.
pub(super) fn delete_file(home: &Path, cmd: &DeleteEditorFileReq) -> Result<(), String> {
    let expanded = expand_home_dir_safe(&cmd.path);
    let path = expanded.as_path();
    let canonical_home = home.canonicalize().map_err(|e| format!("deleteeditorfile: home: {e}"))?;
    let canonical = path.canonicalize().map_err(|e| format!("deleteeditorfile: {e}"))?;
    if !editor_may_change(&canonical, &canonical_home) || is_a_home(&canonical, &canonical_home) {
        return Err("deleteeditorfile: path outside or is the home directory".to_string());
    }
    // symlink_metadata (not metadata) detects a symlink without following it. Deleting via `canonical` would
    // delete the symlink TARGET; the entry at the original path is always what's removed, so a symlink is
    // unlinked.
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("deleteeditorfile: {e}"))?;
    if meta.file_type().is_symlink() {
        // On Windows, directory junctions/symlinks must use remove_dir; remove_file fails for directory
        // symlinks on that platform.
        #[cfg(windows)]
        if path.is_dir() {
            std::fs::remove_dir(path).map_err(|e| format!("deleteeditorfile: {e}"))?;
        } else {
            std::fs::remove_file(path).map_err(|e| format!("deleteeditorfile: {e}"))?;
        }
        #[cfg(not(windows))]
        std::fs::remove_file(path).map_err(|e| format!("deleteeditorfile: {e}"))?;
    } else if canonical.is_dir() {
        if cmd.recursive {
            std::fs::remove_dir_all(&canonical).map_err(|e| format!("deleteeditorfile: {e}"))?;
        } else {
            std::fs::remove_dir(&canonical).map_err(|e| format!("deleteeditorfile: directory not empty: {e}"))?;
        }
    } else {
        std::fs::remove_file(&canonical).map_err(|e| format!("deleteeditorfile: {e}"))?;
    }
    Ok(())
}
