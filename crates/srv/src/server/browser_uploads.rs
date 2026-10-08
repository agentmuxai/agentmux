// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which files an agent may upload into a browser pane
//! (docs/specs/SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md §5.3): only
//! files inside its own workspace, so a page (or a tricked agent) can't use
//! an upload field to carry off anything else on the machine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};


/// At most this many files per upload.
pub(crate) const MAX_UPLOAD_FILES: usize = 10;

fn launch_workspaces() -> &'static Mutex<HashMap<String, PathBuf>> {
    static W: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

/// Remember the workspace the agent on `block_id` was started in: the
/// `AGENTMUX_AGENT_WORKDIR` srv just put in its spawn env (host agents only;
/// a container agent has none). Called by every spawn path right after
/// `agent_handlers::input::carry_agent_workdir_env`.
///
/// Uploads are checked against this record, not the block's `cmd:cwd`: any
/// client with the instance auth key can rewrite that meta, so an agent
/// could otherwise point its "workspace" at `/` and upload anything.
pub(crate) fn record_launch_workspace(block_id: &str, env_vars: &HashMap<String, String>) {
    let mut map = launch_workspaces().lock().unwrap_or_else(|p| p.into_inner());
    match env_vars.get("AGENTMUX_AGENT_WORKDIR").filter(|w| !w.trim().is_empty()) {
        Some(w) => {
            map.insert(block_id.to_string(), PathBuf::from(w));
        }
        None => {
            map.remove(block_id);
        }
    }
}

/// The workspace the agent on `block_id` was launched in, if srv started it
/// as a host agent.
pub(crate) fn agent_workspace(block_id: &str) -> Option<PathBuf> {
    launch_workspaces().lock().unwrap_or_else(|p| p.into_inner()).get(block_id).cloned()
}

/// `canonicalize` without Windows' `\\?\` verbatim prefix, which the browser
/// doesn't expect in a file path.
fn canonical(p: &Path) -> std::io::Result<PathBuf> {
    let c = std::fs::canonicalize(p)?;
    let s = c.to_string_lossy();
    Ok(match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => c,
    })
}

/// Check `paths` (absolute, or relative to the workspace) and return their
/// canonical forms: each must be an existing file inside `workspace` after
/// resolving `..` and symlinks.
pub(crate) fn check_upload_paths(workspace: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Err("no files given".to_string());
    }
    if paths.len() > MAX_UPLOAD_FILES {
        return Err(format!("at most {MAX_UPLOAD_FILES} files per upload"));
    }
    let ws = canonical(workspace)
        .map_err(|e| format!("your workspace {} can't be read: {e}", workspace.display()))?;
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let raw = Path::new(p.trim());
        let joined = if raw.is_absolute() { raw.to_path_buf() } else { workspace.join(raw) };
        let c = canonical(&joined).map_err(|_| format!("no such file: {p}"))?;
        if !c.starts_with(&ws) {
            return Err(format!(
                "{p} is outside your workspace ({}); an agent can upload only files in its own workspace: copy the file there first",
                ws.display()
            ));
        }
        if !c.is_file() {
            return Err(format!("{p} is not a file"));
        }
        out.push(c.to_string_lossy().into_owned());
    }
    Ok(out)
}

/// At most this many bytes in one upload, all files together. Bigger files
/// are for the user to pick (`BrowserHandoff`).
pub(crate) const MAX_UPLOAD_BYTES: u64 = 25 * 1024 * 1024;

/// A file ready to hand to the page: its name, a content type from its
/// extension, and its bytes.
pub(crate) struct UploadFile {
    pub name: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

/// Read the files `check_upload_paths` approved. Each is opened once, and the
/// check is repeated on the open handle: the file it really refers to must be
/// a regular file inside the workspace. The bytes are read from that same
/// handle and the page is given the bytes, never a path, so replacing a
/// checked file with a link afterwards changes nothing.
pub(crate) fn read_upload_files(workspace: &Path, checked: &[String]) -> Result<Vec<UploadFile>, String> {
    use std::io::Read;
    let ws = canonical(workspace).map_err(|e| format!("your workspace {} can't be read: {e}", workspace.display()))?;
    let mut total: u64 = 0;
    let mut out = Vec::with_capacity(checked.len());
    for p in checked {
        let file = std::fs::File::open(p).map_err(|_| format!("no such file: {p}"))?;
        let real = handle_path(&file).map_err(|e| format!("can't tell where {p} is ({e}), so it isn't uploaded"))?;
        if !real.starts_with(&ws) {
            return Err(format!("{p} is outside your workspace ({})", ws.display()));
        }
        let meta = file.metadata().map_err(|e| format!("{p}: {e}"))?;
        if !meta.is_file() {
            return Err(format!("{p} is not a file"));
        }
        total += meta.len();
        if total > MAX_UPLOAD_BYTES {
            return Err(format!(
                "uploads are limited to {} MB in all; for bigger files, ask the user to pick them (BrowserHandoff)",
                MAX_UPLOAD_BYTES / (1024 * 1024)
            ));
        }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        file.take(MAX_UPLOAD_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("{p}: {e}"))?;
        if bytes.len() as u64 != meta.len() {
            return Err(format!("{p} changed while it was being read; try again"));
        }
        let name = real.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
        out.push(UploadFile { mime: mime_for(&name), name, bytes });
    }
    Ok(out)
}

/// The path the open file really is, after every link: what the OS says
/// about the handle, not a lookup of the name.
#[cfg(windows)]
fn handle_path(file: &std::fs::File) -> std::io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, VOLUME_NAME_DOS};
    let mut buf = vec![0u16; 1024];
    loop {
        let n = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle() as _,
                buf.as_mut_ptr(),
                buf.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        } as usize;
        if n == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n < buf.len() {
            let p = PathBuf::from(std::ffi::OsString::from_wide(&buf[..n]));
            let s = p.to_string_lossy();
            return Ok(match s.strip_prefix(r"\\?\") {
                Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
                _ => p,
            });
        }
        buf.resize(n + 1, 0);
    }
}

#[cfg(target_os = "linux")]
fn handle_path(file: &std::fs::File) -> std::io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

#[cfg(target_os = "macos")]
fn handle_path(file: &std::fs::File) -> std::io::Result<PathBuf> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..end])))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn handle_path(_file: &std::fs::File) -> std::io::Result<PathBuf> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "not supported on this platform"))
}

/// A content type for the file, from its extension, as a browser's own file
/// picker would give it. Unknown types go as generic bytes.
fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "htm" | "html" => "text/html",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "exe" | "dll" | "msi" => "application/x-msdownload",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::MetaMapType;
    use serde_json::json;

    fn ws() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("agent-ws");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::write(ws.join("report.txt"), "x").unwrap();
        std::fs::write(ws.join("sub").join("a.bin"), "y").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "z").unwrap();
        (dir, ws)
    }

    #[test]
    fn files_inside_the_workspace_are_allowed_absolute_or_relative() {
        let (_d, ws) = ws();
        let got = check_upload_paths(&ws, &["report.txt".into(), ws.join("sub").join("a.bin").to_string_lossy().into_owned()]).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got[0].ends_with("report.txt") && !got[0].starts_with(r"\\?\"), "{got:?}");
    }

    #[test]
    fn a_file_outside_the_workspace_is_refused_including_through_dotdot() {
        let (d, ws) = ws();
        let e = check_upload_paths(&ws, &[d.path().join("secret.txt").to_string_lossy().into_owned()]).unwrap_err();
        assert!(e.contains("outside your workspace"), "{e}");
        let e = check_upload_paths(&ws, &["../secret.txt".into()]).unwrap_err();
        assert!(e.contains("outside your workspace"), "{e}");
    }

    #[test]
    fn missing_files_directories_and_too_many_are_refused() {
        let (_d, ws) = ws();
        assert!(check_upload_paths(&ws, &["nope.txt".into()]).unwrap_err().contains("no such file"));
        assert!(check_upload_paths(&ws, &["sub".into()]).unwrap_err().contains("not a file"));
        assert!(check_upload_paths(&ws, &[]).is_err());
        let many: Vec<String> = (0..11).map(|_| "report.txt".to_string()).collect();
        assert!(check_upload_paths(&ws, &many).unwrap_err().contains("at most"));
    }

    #[test]
    fn the_workspace_is_what_srv_launched_the_agent_in() {
        // Through the real spawn-env helper: host agents get one, a
        // container agent (or no launch directory) gets none.
        let spawn = |pairs: &[(&str, &str)]| {
            let mut m = MetaMapType::new();
            for (k, v) in pairs {
                m.insert(k.to_string(), json!(v));
            }
            let mut env = HashMap::new();
            crate::server::agent_handlers::input::carry_agent_workdir_env(&mut env, &m);
            env
        };
        record_launch_workspace("ws-test-a", &spawn(&[("cmd:cwd", "C:/work/agent")]));
        assert!(agent_workspace("ws-test-a").is_some());
        record_launch_workspace("ws-test-a", &spawn(&[("cmd:cwd", "C:/work/agent"), ("agentMode", "container")]));
        assert_eq!(agent_workspace("ws-test-a"), None);
        assert_eq!(agent_workspace("ws-test-never-launched"), None);
    }

    #[test]
    fn checked_files_are_read_from_the_handle_with_a_content_type() {
        let (_d, ws) = ws();
        let checked = check_upload_paths(&ws, &["report.txt".into(), "sub/a.bin".into()]).unwrap();
        let files = read_upload_files(&ws, &checked).unwrap();
        assert_eq!(files[0].name, "report.txt");
        assert_eq!(files[0].bytes, b"x");
        assert_eq!(files[0].mime, "text/plain");
        assert_eq!(files[1].mime, "application/octet-stream");
    }

    #[test]
    fn the_handle_check_refuses_a_file_outside_the_workspace_on_its_own() {
        // As if the path had been swapped after check_upload_paths: hand the
        // reader an outside file directly. The open handle, not the name, has
        // to stop it.
        let (d, ws) = ws();
        let outside = d.path().join("secret.txt").to_string_lossy().into_owned();
        let e = read_upload_files(&ws, &[outside]).err().expect("refused");
        assert!(e.contains("outside your workspace"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_swapped_in_after_the_check_is_caught_by_the_handle() {
        let (d, ws) = ws();
        let checked = check_upload_paths(&ws, &["report.txt".into()]).unwrap();
        std::fs::remove_file(ws.join("report.txt")).unwrap();
        std::os::unix::fs::symlink(d.path().join("secret.txt"), ws.join("report.txt")).unwrap();
        let e = read_upload_files(&ws, &checked).err().expect("refused");
        assert!(e.contains("outside your workspace"), "{e}");
    }

    #[test]
    fn uploads_over_the_size_limit_are_refused() {
        let (_d, ws) = ws();
        let big = ws.join("big.bin");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_UPLOAD_BYTES + 1).unwrap();
        let checked = check_upload_paths(&ws, &["big.bin".into()]).unwrap();
        let e = read_upload_files(&ws, &checked).err().expect("refused");
        assert!(e.contains("limited to"), "{e}");
    }
}
