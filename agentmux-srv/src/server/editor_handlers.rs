// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_WRITE_AGENT_CONFIG,
    CommandWriteAgentConfigData,
};
use crate::backend::base::expand_home_dir_safe;

use super::AppState;

// Per-process token written into scratch claim files. Stale claim files from a
// previous process run have a different token and are ignored during reuse scans,
// allowing crash-recovery (the scratch becomes reclaimable on restart).
static SCRATCH_SESSION_TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn scratch_session_token() -> &'static str {
    SCRATCH_SESSION_TOKEN.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// Enumerate drives/mounts accessible from the current OS user.
/// Used by the `geteditorroots` RPC so the editor file-tree exposes
/// every reachable filesystem root, not just $HOME.
/// Spec: specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md (multi-root follow-up).
#[cfg(target_os = "windows")]
fn list_drives() -> Vec<serde_json::Value> {
    let mut drives = Vec::new();
    for letter in b'A'..=b'Z' {
        let path = format!("{}:\\", letter as char);
        if std::path::Path::new(&path).exists() {
            drives.push(serde_json::json!({
                "name": format!("{}:", letter as char),
                "path": path,
            }));
        }
    }
    drives
}

#[cfg(not(target_os = "windows"))]
fn list_drives() -> Vec<serde_json::Value> {
    let mut drives = vec![serde_json::json!({ "name": "/", "path": "/" })];
    for mount_dir in ["/mnt", "/media", "/Volumes"] {
        if let Ok(entries) = std::fs::read_dir(mount_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    drives.push(serde_json::json!({
                        "name": entry.file_name().to_string_lossy().to_string(),
                        "path": path.to_string_lossy().to_string(),
                    }));
                }
            }
        }
    }
    drives
}

pub fn register_editor_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let _ = state; // state not currently needed by these handlers; kept for API consistency

    // writeagentconfig → write config files atomically to agent working directory
    engine.register_handler(
        COMMAND_WRITE_AGENT_CONFIG,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandWriteAgentConfigData = serde_json::from_value(data)
                    .map_err(|e| format!("writeagentconfig: {e}"))?;
                tracing::info!(
                    working_dir = %cmd.working_dir,
                    file_count = cmd.files.len(),
                    auto_allocate = cmd.auto_allocate,
                    "WriteAgentConfig"
                );

                // Resolve to a final on-disk path. For auto-generated
                // instance paths (`auto_allocate: true`), use the
                // atomic `<base>-N` allocator so concurrent same-hour
                // launches don't share a workdir. For user-specified
                // paths, mkdir-p as before — never rewrite.
                let expanded_working_dir = expand_home_dir_safe(&cmd.working_dir);
                let final_working_dir = if cmd.auto_allocate {
                    let desired = expanded_working_dir.to_string_lossy().to_string();
                    crate::server::app_api::allocate_agent_workdir(&desired)?
                } else {
                    let p = expanded_working_dir.as_path();
                    if !p.exists() {
                        std::fs::create_dir_all(p)
                            .map_err(|e| format!("failed to create working dir: {e}"))?;
                    }
                    expanded_working_dir.to_string_lossy().to_string()
                };
                let base_path = std::path::Path::new(&final_working_dir);
                // Canonicalize base ONCE (it exists — allocate_agent_workdir
                // or the explicit-path mkdir-p created it just above). Used
                // by the per-file symlink-escape verifier so we catch a
                // symlinked ancestor like `<base>/.claude -> /tmp/outside`
                // before fs::write follows it.
                let canonical_base = base_path.canonicalize().map_err(|e| {
                    format!("failed to canonicalize working dir {}: {e}", base_path.display())
                })?;

                for file in &cmd.files {
                    // Lexical join + traversal check — works on Windows where
                    // canonicalize() adds the `\\?\` UNC prefix and breaks
                    // starts_with against not-yet-created files. Catches `..`,
                    // absolute paths, drive-letter prefixes (root and inner).
                    let file_path = crate::backend::base::safe_join_within_base(
                        base_path,
                        &file.path,
                    )
                    .map_err(|e| format!("path traversal denied: {} ({e})", file.path))?;
                    // Symlink-escape guard: if any EXISTING ancestor is a
                    // symlink that resolves outside the workdir, reject.
                    // No-op for fully-fresh agent dirs (the common case
                    // where every component is new).
                    crate::backend::base::verify_no_symlink_escape(&file_path, &canonical_base)
                        .map_err(|e| format!("path traversal denied: {} ({e})", file.path))?;
                    // Create parent directories if needed
                    if let Some(parent) = file_path.parent() {
                        if !parent.exists() {
                            std::fs::create_dir_all(parent)
                                .map_err(|e| format!("failed to create dir for {}: {e}", file.path))?;
                        }
                    }
                    std::fs::write(&file_path, &file.content)
                        .map_err(|e| format!("failed to write {}: {e}", file.path))?;
                    tracing::debug!(path = %file_path.display(), "wrote config file");
                }

                // Return the final path so the caller can patch
                // `cmd:cwd` if collision resolution changed it.
                Ok(Some(serde_json::json!({
                    "working_dir": final_working_dir,
                })))
            })
        }),
    );

    // readeditorfile → read file from disk for the editor pane
    engine.register_handler(
        "readeditorfile",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { path: String }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("readeditorfile: {e}"))?;
                let expanded = expand_home_dir_safe(&cmd.path);
                let path = expanded.as_path();

                // Size guard: reject files > 10MB
                let metadata = std::fs::metadata(path)
                    .map_err(|e| format!("readeditorfile: {e}"))?;
                if metadata.len() > 10_000_000 {
                    return Err("File too large (>10MB)".to_string());
                }

                // Read bytes and detect the encoding (BOM → UTF-8 → heuristic),
                // decoding to UTF-8 for the editor. This lets non-UTF-8 files
                // (Windows-1252 .ini, UTF-16, Shift_JIS, …) open instead of
                // erroring on read_to_string. The detected encoding/bom/line
                // ending ride back so save round-trips the original format.
                // See docs/specs/SPEC_EDITOR_FILE_ENCODINGS_2026_06_17.md.
                let bytes = std::fs::read(path)
                    .map_err(|e| format!("readeditorfile: {e}"))?;
                let decoded = crate::backend::text_encoding::decode_file(&bytes);
                let read_only = metadata.permissions().readonly();

                Ok(Some(serde_json::json!({
                    "content": decoded.content,
                    "encoding": decoded.encoding,
                    "bom": decoded.bom,
                    "line_ending": decoded.line_ending,
                    "had_decode_errors": decoded.had_decode_errors,
                    "read_only": read_only,
                })))
            })
        }),
    );

    // writeeditorfile → write file to disk from the editor pane
    engine.register_handler(
        "writeeditorfile",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd {
                    path: String,
                    content: String,
                    // Encoding to write back in (defaults preserve old UTF-8
                    // behavior when a caller doesn't send them).
                    #[serde(default)]
                    encoding: Option<String>,
                    #[serde(default)]
                    bom: Option<String>,
                    #[serde(default)]
                    line_ending: Option<String>,
                }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("writeeditorfile: {e}"))?;

                // Size guard: match readeditorfile's 10MB limit
                if cmd.content.len() > 10_000_000 {
                    return Err("Content too large (>10MB)".to_string());
                }

                let expanded = expand_home_dir_safe(&cmd.path);
                let path = expanded.as_path();

                // Path safety: restrict writes to under the user's home directory.
                // Allowlist approach — safer than an incomplete denylist.
                let home = dirs::home_dir()
                    .ok_or("writeeditorfile: cannot determine home directory")?;
                let canonical_home = home.canonicalize()
                    .map_err(|e| format!("writeeditorfile: home dir: {e}"))?;

                // Resolve the target path (canonicalize existing, or parent + filename for new files)
                let canonical = path.canonicalize().or_else(|_| {
                    path.parent()
                        .and_then(|p| p.canonicalize().ok())
                        .map(|p| p.join(path.file_name().unwrap_or_default()))
                        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "invalid path"))
                }).map_err(|e| format!("writeeditorfile: {e}"))?;

                if !canonical.starts_with(&canonical_home) {
                    return Err(format!(
                        "writeeditorfile: path {} is outside home directory",
                        canonical.display()
                    ));
                }

                // Re-encode to the file's original encoding (+ BOM + line
                // endings) so a non-UTF-8 file round-trips instead of being
                // silently rewritten as UTF-8. Absent fields default to UTF-8.
                let (out_bytes, had_unmappable) = crate::backend::text_encoding::encode_file(
                    &cmd.content,
                    cmd.encoding.as_deref().unwrap_or(""),
                    cmd.bom.as_deref().unwrap_or("none"),
                    cmd.line_ending.as_deref().unwrap_or("lf"),
                );
                // Refuse a lossy write: rather than silently emit
                // numeric-character-reference replacements (corrupting the
                // file), fail loudly so the user can save as UTF-8. The UI for
                // an in-place "Save with Encoding → UTF-8" is Phase 3 of
                // SPEC_EDITOR_FILE_ENCODINGS.
                if had_unmappable {
                    let enc = cmd
                        .encoding
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("its current encoding");
                    return Err(format!(
                        "writeeditorfile: file contains characters that can't be represented in {enc} — save it as UTF-8 instead"
                    ));
                }
                std::fs::write(&canonical, &out_bytes)
                    .map_err(|e| format!("writeeditorfile: {e}"))?;
                tracing::info!(path = %canonical.display(), bytes = out_bytes.len(), "editor file saved");

                Ok(None)
            })
        }),
    );

    // listeditordir → list directory contents for the editor's file-tree pane.
    // Symlinks are followed (matches VS Code semantics; the frontend marks
    // followed symlinks with a ↗ overlay).
    // Spec: specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md
    engine.register_handler(
        "listeditordir",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { path: String }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("listeditordir: {e}"))?;
                let expanded = expand_home_dir_safe(&cmd.path);
                let canonical = expanded.canonicalize()
                    .map_err(|e| format!("listeditordir: {e}"))?;
                let read_dir = std::fs::read_dir(&canonical)
                    .map_err(|e| format!("listeditordir: {e}"))?;

                let mut entries: Vec<serde_json::Value> = Vec::new();
                for entry in read_dir.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    // `file_type()` returns the entry's own type — symlinks
                    // appear as symlinks (no follow). `metadata()` follows
                    // symlinks, so a symlinked dir reports is_dir=true.
                    // We need both: is_symlink from file_type, is_dir/size
                    // from metadata (so a symlink-to-dir reads as a directory,
                    // matching VS Code).
                    let is_symlink = entry
                        .file_type()
                        .map(|t| t.is_symlink())
                        .unwrap_or(false);
                    let metadata = entry.metadata().ok();
                    let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                    let size = metadata
                        .as_ref()
                        .and_then(|m| if !m.is_dir() { Some(m.len()) } else { None });
                    let mtime = metadata
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as u64);

                    let mut entry_obj = serde_json::json!({
                        "name": name,
                        "is_dir": is_dir,
                        "is_symlink": is_symlink,
                    });
                    if let Some(s) = size {
                        entry_obj["size"] = serde_json::json!(s);
                    }
                    if let Some(m) = mtime {
                        entry_obj["mtime"] = serde_json::json!(m);
                    }
                    entries.push(entry_obj);
                }

                // Folders first, then files; alphabetical, case-insensitive.
                entries.sort_by(|a, b| {
                    let a_dir = a["is_dir"].as_bool().unwrap_or(false);
                    let b_dir = b["is_dir"].as_bool().unwrap_or(false);
                    let a_name = a["name"].as_str().unwrap_or("").to_lowercase();
                    let b_name = b["name"].as_str().unwrap_or("").to_lowercase();
                    match (a_dir, b_dir) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => a_name.cmp(&b_name),
                    }
                });

                Ok(Some(serde_json::json!({
                    "path": canonical.to_string_lossy(),
                    "entries": entries,
                })))
            })
        }),
    );

    // geteditorhome → OS home directory, used as the editor file-tree default root.
    engine.register_handler(
        "geteditorhome",
        Box::new(|_data, _ctx| {
            Box::pin(async move {
                let home = dirs::home_dir()
                    .ok_or_else(|| "geteditorhome: cannot determine home directory".to_string())?;
                Ok(Some(serde_json::json!({
                    "home": home.to_string_lossy(),
                })))
            })
        }),
    );

    // geteditorroots → home + every reachable drive/mount on the system.
    // The editor file-tree renders these as sibling top-level roots so the
    // user can navigate to anywhere on the machine, not just inside $HOME.
    // Spec: specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md (multi-root follow-up)
    engine.register_handler(
        "geteditorroots",
        Box::new(|_data, _ctx| {
            Box::pin(async move {
                let home = dirs::home_dir()
                    .ok_or_else(|| "geteditorroots: cannot determine home directory".to_string())?;
                let drives = list_drives();
                Ok(Some(serde_json::json!({
                    "home": home.to_string_lossy(),
                    "drives": drives,
                })))
            })
        }),
    );

    // ── Editor file-tree mutations ─────────────────────────────────────
    // Spec: specs/SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md
    // All handlers validate the target path is under the user's home directory
    // before performing any mutation — same policy as writeeditorfile.

    // openinshell → reveal a path in the OS file manager
    engine.register_handler(
        "openinshell",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { path: String }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("openinshell: {e}"))?;
                let expanded = expand_home_dir_safe(&cmd.path);
                let path = expanded.as_path();
                let home = dirs::home_dir()
                    .ok_or("openinshell: cannot determine home directory")?;
                let canonical_home = home.canonicalize()
                    .map_err(|e| format!("openinshell: home: {e}"))?;
                let canonical = path.canonicalize()
                    .map_err(|e| format!("openinshell: {e}"))?;
                if !canonical.starts_with(&canonical_home) {
                    return Err(format!("openinshell: path outside home directory"));
                }
                #[cfg(target_os = "windows")]
                { let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", canonical.display())).spawn(); }
                #[cfg(target_os = "macos")]
                { let _ = std::process::Command::new("open").arg("-R").arg(&canonical).spawn(); }
                #[cfg(target_os = "linux")]
                {
                    let target = if canonical.is_dir() { canonical.clone() } else { canonical.parent().unwrap_or(&canonical).to_path_buf() };
                    let _ = std::process::Command::new("xdg-open").arg(&target).spawn();
                }
                Ok(None)
            })
        }),
    );

    // renameeditorfile → rename a file or folder (name only, same parent directory)
    engine.register_handler(
        "renameeditorfile",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { old_path: String, new_name: String }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("renameeditorfile: {e}"))?;
                let expanded = expand_home_dir_safe(&cmd.old_path);
                let old_path = expanded.as_path();
                let home = dirs::home_dir().ok_or("renameeditorfile: cannot determine home")?;
                let canonical_home = home.canonicalize().map_err(|e| format!("renameeditorfile: home: {e}"))?;
                let canonical_old = old_path.canonicalize().map_err(|e| format!("renameeditorfile: {e}"))?;
                if !canonical_old.starts_with(&canonical_home) || canonical_old == canonical_home {
                    return Err("renameeditorfile: path outside or is the home directory".to_string());
                }
                if cmd.new_name.contains('/') || cmd.new_name.contains('\\') || cmd.new_name.contains('\0') || cmd.new_name == ".." || cmd.new_name == "." || cmd.new_name.is_empty() {
                    return Err("renameeditorfile: new_name must be a plain filename".to_string());
                }
                let new_path = canonical_old.parent()
                    .ok_or("renameeditorfile: no parent directory")?
                    .join(&cmd.new_name);
                if new_path.exists() {
                    return Err(format!("renameeditorfile: destination already exists"));
                }
                std::fs::rename(&canonical_old, &new_path)
                    .map_err(|e| format!("renameeditorfile: {e}"))?;
                Ok(Some(serde_json::json!({ "new_path": new_path.to_string_lossy() })))
            })
        }),
    );

    // createeditorfile → create an empty file inside an existing directory
    engine.register_handler(
        "createeditorfile",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { parent_path: String, name: String }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("createeditorfile: {e}"))?;
                let expanded = expand_home_dir_safe(&cmd.parent_path);
                let parent = expanded.as_path();
                let home = dirs::home_dir().ok_or("createeditorfile: cannot determine home")?;
                let canonical_home = home.canonicalize().map_err(|e| format!("createeditorfile: home: {e}"))?;
                let canonical_parent = parent.canonicalize().map_err(|e| format!("createeditorfile: {e}"))?;
                if !canonical_parent.starts_with(&canonical_home) {
                    return Err("createeditorfile: path outside home directory".to_string());
                }
                if cmd.name.contains('/') || cmd.name.contains('\\') || cmd.name.contains('\0') || cmd.name == "." || cmd.name == ".." || cmd.name.is_empty() {
                    return Err("createeditorfile: name must be a plain filename".to_string());
                }
                let file_path = canonical_parent.join(&cmd.name);
                if file_path.exists() {
                    return Err("createeditorfile: file already exists".to_string());
                }
                std::fs::write(&file_path, "").map_err(|e| format!("createeditorfile: {e}"))?;
                Ok(Some(serde_json::json!({ "file_path": file_path.to_string_lossy() })))
            })
        }),
    );

    // createeditordir → create a directory inside an existing directory
    engine.register_handler(
        "createeditordir",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { parent_path: String, name: String }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("createeditordir: {e}"))?;
                let expanded = expand_home_dir_safe(&cmd.parent_path);
                let parent = expanded.as_path();
                let home = dirs::home_dir().ok_or("createeditordir: cannot determine home")?;
                let canonical_home = home.canonicalize().map_err(|e| format!("createeditordir: home: {e}"))?;
                let canonical_parent = parent.canonicalize().map_err(|e| format!("createeditordir: {e}"))?;
                if !canonical_parent.starts_with(&canonical_home) {
                    return Err("createeditordir: path outside home directory".to_string());
                }
                if cmd.name.contains('/') || cmd.name.contains('\\') || cmd.name.contains('\0') || cmd.name == "." || cmd.name == ".." || cmd.name.is_empty() {
                    return Err("createeditordir: name must be a plain name".to_string());
                }
                let dir_path = canonical_parent.join(&cmd.name);
                if dir_path.exists() {
                    return Err("createeditordir: already exists".to_string());
                }
                std::fs::create_dir(&dir_path).map_err(|e| format!("createeditordir: {e}"))?;
                Ok(Some(serde_json::json!({ "dir_path": dir_path.to_string_lossy() })))
            })
        }),
    );

    // deleteeditorfile → delete a file or directory
    engine.register_handler(
        "deleteeditorfile",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { path: String, recursive: bool }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("deleteeditorfile: {e}"))?;
                let expanded = expand_home_dir_safe(&cmd.path);
                let path = expanded.as_path();
                let home = dirs::home_dir().ok_or("deleteeditorfile: cannot determine home")?;
                let canonical_home = home.canonicalize().map_err(|e| format!("deleteeditorfile: home: {e}"))?;
                let canonical = path.canonicalize().map_err(|e| format!("deleteeditorfile: {e}"))?;
                if !canonical.starts_with(&canonical_home) || canonical == canonical_home {
                    return Err("deleteeditorfile: path outside or is the home directory".to_string());
                }
                // Use symlink_metadata (not metadata) to detect symlinks without following them.
                // Deleting via `canonical` would delete the symlink TARGET; we always remove the
                // entry at the original path so the symlink itself is unlinked.
                let meta = std::fs::symlink_metadata(path)
                    .map_err(|e| format!("deleteeditorfile: {e}"))?;
                if meta.file_type().is_symlink() {
                    // On Windows, directory junctions/symlinks must use remove_dir;
                    // remove_file fails for directory symlinks on that platform.
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
                Ok(None)
            })
        }),
    );

    // ── Scratch file service ────────────────────────────────────────────
    // Spec: specs/SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md

    // createscratchfile → create a scratch buffer file in ~/.agentmux/cache/scratch/
    engine.register_handler(
        "createscratchfile",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { display_name: Option<String>, exclude_scratch_ids: Option<Vec<String>> }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("createscratchfile: {e}"))?;
                let home = dirs::home_dir()
                    .ok_or("createscratchfile: cannot determine home directory")?;
                let scratch_dir = home.join(".agentmux").join("cache").join("scratch");
                std::fs::create_dir_all(&scratch_dir)
                    .map_err(|e| format!("createscratchfile: create dir: {e}"))?;
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                const THIRTY_DAYS_MS: u64 = 30 * 24 * 60 * 60 * 1000;
                let session_token = scratch_session_token();

                // Phase 1: scan .md.meta files, collect candidates and prune expired pairs.
                // A candidate is: not saved, not expired, not in exclude_scratch_ids.
                let mut candidates: Vec<(u64, String, String)> = Vec::new(); // (created_at, sid, display_name)
                if let Ok(entries) = std::fs::read_dir(&scratch_dir) {
                    for entry in entries.flatten() {
                        let meta_path = entry.path();
                        let fname = meta_path
                            .file_name()
                            .and_then(|f| f.to_str())
                            .unwrap_or("")
                            .to_string();
                        if !fname.ends_with(".md.meta") {
                            continue;
                        }
                        let Ok(content) = std::fs::read_to_string(&meta_path) else { continue };
                        let Ok(meta_json) = serde_json::from_str::<serde_json::Value>(&content) else { continue };
                        if meta_json.get("saved_to").map_or(false, |v| !v.is_null()) {
                            continue;
                        }
                        let created_at = meta_json.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
                        let sid = meta_json.get("scratch_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if created_at == 0 || sid.is_empty() {
                            continue;
                        }
                        if cmd.exclude_scratch_ids.as_deref().map_or(false, |ex| ex.iter().any(|e| e == &sid)) {
                            continue;
                        }
                        if now_ms.saturating_sub(created_at) > THIRTY_DAYS_MS {
                            // Prune expired pair (all three files: .md, .md.meta, .md.claim).
                            let _ = std::fs::remove_file(scratch_dir.join(format!("{}.md", sid)));
                            let _ = std::fs::remove_file(&meta_path);
                            let _ = std::fs::remove_file(scratch_dir.join(format!("{}.md.claim", sid)));
                            continue;
                        }
                        let dname = meta_json
                            .get("display_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Untitled")
                            .to_string();
                        candidates.push((created_at, sid, dname));
                    }
                }

                // Phase 2: try to atomically claim the most-recent candidate.
                // Claim files prevent two concurrent createscratchfile calls from
                // returning the same backing file to different panes. Each claim file
                // stores the per-process session token; a claim from a previous process
                // (crash/restart) has a different token and is cleared so recovery works.
                candidates.sort_by(|a, b| b.0.cmp(&a.0)); // most recent first
                let mut chosen: Option<(String, String)> = None;
                for (_, sid, dname) in &candidates {
                    let claim_path = scratch_dir.join(format!("{}.md.claim", sid));
                    // Evict stale claims from a previous process session.
                    if claim_path.exists() {
                        if let Ok(tok) = std::fs::read_to_string(&claim_path) {
                            if tok.trim() == session_token {
                                // Valid live claim — another concurrent call already owns this.
                                continue;
                            }
                        }
                        // Stale claim (different session) — remove so we can reclaim.
                        let _ = std::fs::remove_file(&claim_path);
                    }
                    // Try to exclusively create the claim file. If another concurrent
                    // call wins the race, it will have already created it → we skip.
                    match std::fs::OpenOptions::new().write(true).create_new(true).open(&claim_path) {
                        Ok(mut f) => {
                            use std::io::Write as _;
                            let _ = f.write_all(session_token.as_bytes());
                            let file_path = scratch_dir.join(format!("{}.md", sid));
                            if file_path.exists() {
                                chosen = Some((sid.clone(), dname.clone()));
                            } else {
                                // Backing file gone (deleted externally) — release claim.
                                let _ = std::fs::remove_file(&claim_path);
                            }
                        }
                        Err(_) => continue, // Concurrent call claimed this one first.
                    }
                    if chosen.is_some() {
                        break;
                    }
                }

                if let Some((scratch_id, display_name)) = chosen {
                    let file_path = scratch_dir.join(format!("{}.md", scratch_id));
                    return Ok(Some(serde_json::json!({
                        "scratch_id": scratch_id,
                        "file_path": file_path.to_string_lossy(),
                        "display_name": display_name,
                    })));
                }

                // No reusable candidate — mint a fresh UUID pair.
                let scratch_id = uuid::Uuid::new_v4().to_string();
                let display_name = cmd.display_name.unwrap_or_else(|| "Untitled".to_string());
                let file_path = scratch_dir.join(format!("{}.md", scratch_id));
                std::fs::write(&file_path, "")
                    .map_err(|e| format!("createscratchfile: {e}"))?;
                let meta = serde_json::json!({
                    "display_name": display_name,
                    "scratch_id": scratch_id,
                    "created_at": now_ms,
                });
                let meta_path = scratch_dir.join(format!("{}.md.meta", scratch_id));
                std::fs::write(&meta_path, meta.to_string())
                    .map_err(|e| format!("createscratchfile: meta: {e}"))?;
                // Claim the fresh scratch so subsequent calls don't immediately reuse it.
                let claim_path = scratch_dir.join(format!("{}.md.claim", scratch_id));
                let _ = std::fs::write(&claim_path, session_token);
                Ok(Some(serde_json::json!({
                    "scratch_id": scratch_id,
                    "file_path": file_path.to_string_lossy(),
                    "display_name": display_name,
                })))
            })
        }),
    );

    // movescratchfile → promote a scratch buffer to a real user-chosen path (Save As)
    engine.register_handler(
        "movescratchfile",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Cmd { scratch_id: String, destination_path: String }
                let cmd: Cmd = serde_json::from_value(data)
                    .map_err(|e| format!("movescratchfile: {e}"))?;
                let home = dirs::home_dir()
                    .ok_or("movescratchfile: cannot determine home")?;
                let canonical_home = home.canonicalize()
                    .map_err(|e| format!("movescratchfile: home: {e}"))?;
                // Validate scratch_id has no path components.
                if cmd.scratch_id.contains('/') || cmd.scratch_id.contains('\\') || cmd.scratch_id.contains("..") {
                    return Err("movescratchfile: invalid scratch_id".to_string());
                }
                let scratch_dir = home.join(".agentmux").join("cache").join("scratch");
                let scratch_path = scratch_dir.join(format!("{}.md", cmd.scratch_id));
                if !scratch_path.exists() {
                    return Err(format!("movescratchfile: scratch file not found"));
                }
                // Validate + resolve destination.
                let expanded_dest = expand_home_dir_safe(&cmd.destination_path);
                let dest_path = expanded_dest.as_path();

                // Coarse home-boundary check before any filesystem mutation.
                // We use the unexpanded path here because the dest may not exist yet
                // (canonicalize fails on non-existent paths).
                if !dest_path.starts_with(&home) {
                    return Err("movescratchfile: destination outside home directory".to_string());
                }
                // Reject ".." components before create_dir_all — the coarse starts_with
                // check is purely lexical and passes paths like ~/foo/../../../tmp/evil
                // because the prefix matches, but create_dir_all would then materialize
                // directories outside the home boundary before the canonical check fires.
                if dest_path.components().any(|c| c == std::path::Component::ParentDir) {
                    return Err("movescratchfile: destination path must not contain '..' components".to_string());
                }
                // Reject existing destination to avoid silent data loss.
                if dest_path.exists() {
                    return Err("movescratchfile: destination already exists".to_string());
                }
                // Create parent directories if needed.
                if let Some(parent) = dest_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("movescratchfile: create dirs: {e}"))?;
                }
                // Canonicalize parent (now guaranteed to exist) + append filename.
                let canonical_dest = dest_path.parent()
                    .and_then(|p| p.canonicalize().ok())
                    .map(|p| p.join(dest_path.file_name().unwrap_or_default()))
                    .ok_or_else(|| "movescratchfile: invalid destination path".to_string())?;
                // Fine-grained home-boundary check with canonical paths.
                if !canonical_dest.starts_with(&canonical_home) {
                    return Err("movescratchfile: destination outside home directory".to_string());
                }
                // Copy then remove scratch source (cross-device-safe move).
                std::fs::copy(&scratch_path, &canonical_dest)
                    .map_err(|e| format!("movescratchfile: copy: {e}"))?;
                std::fs::remove_file(&scratch_path)
                    .map_err(|e| format!("movescratchfile: remove scratch: {e}"))?;
                // Clean up the .meta sidecar and the claim file.
                let meta_path = scratch_dir.join(format!("{}.md.meta", cmd.scratch_id));
                let _ = std::fs::remove_file(&meta_path);
                let claim_path = scratch_dir.join(format!("{}.md.claim", cmd.scratch_id));
                let _ = std::fs::remove_file(&claim_path);
                Ok(Some(serde_json::json!({ "file_path": canonical_dest.to_string_lossy() })))
            })
        }),
    );
}
