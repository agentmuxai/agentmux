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
    // A block is one or the other: a host launch ends any container record.
    container_workspaces().lock().unwrap_or_else(|p| p.into_inner()).remove(block_id);
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

fn container_workspaces() -> &'static Mutex<HashMap<String, PathBuf>> {
    static W: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

/// Where a container agent sees its workspace.
pub(crate) const CONTAINER_WORKSPACE: &str = "/workspace";

/// Remember the host folder mounted at `/workspace` in the container of the
/// agent on `block_id`, as srv started it: the mount's source, not the block's
/// `cmd:cwd`, which any client can rewrite after the launch. Called where the
/// container's mounts are decided.
pub(crate) fn record_container_workspace(block_id: &str, host_dir: Option<&str>) {
    launch_workspaces().lock().unwrap_or_else(|p| p.into_inner()).remove(block_id);
    let mut map = container_workspaces().lock().unwrap_or_else(|p| p.into_inner());
    match host_dir.map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => {
            map.insert(block_id.to_string(), PathBuf::from(d));
        }
        None => {
            map.remove(block_id);
        }
    }
}

/// The host folder behind `/workspace` for the container agent on `block_id`.
pub(crate) fn container_workspace(block_id: &str) -> Option<PathBuf> {
    container_workspaces().lock().unwrap_or_else(|p| p.into_inner()).get(block_id).cloned()
}

/// The host paths for files a container agent names: `/workspace/...`, or a path
/// relative to its workspace. Anything else in the container (its home, `/tmp`)
/// isn't on the host and isn't uploaded, and neither is a path with `..` in it,
/// which would reach the host folders beside the workspace. The result goes
/// through [`check_upload_paths`] like a host agent's, which resolves links on the
/// host: a link in the workspace that leads outside it is refused.
pub(crate) fn map_container_paths(host_dir: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|p| {
            let raw = p.trim();
            let outside = || {
                format!("{p} is outside {CONTAINER_WORKSPACE}; a container agent can upload only files in its own workspace: copy the file there first")
            };
            let rest = match raw.strip_prefix(CONTAINER_WORKSPACE) {
                Some("") => "",
                Some(r) if r.starts_with('/') => r.trim_start_matches('/'),
                _ if raw.starts_with('/') || raw.starts_with('\\') || raw.contains(':') => return Err(outside()),
                _ => raw,
            };
            if Path::new(rest).components().any(|c| matches!(c, std::path::Component::ParentDir)) {
                return Err(outside());
            }
            Ok(host_dir.join(rest).to_string_lossy().into_owned())
        })
        .collect()
}

/// What a container agent is told when a file it named is not inside its
/// workspace: the same as for a file that isn't there, so a link in the
/// workspace can't be used to ask whether a host path exists.
fn container_view(error: String) -> String {
    match error.split_once(" is outside your workspace") {
        Some((path, _)) => format!("no such file: {path}"),
        None => error,
    }
}

/// The files `paths` name, read for the agent on `own` (its block) as the
/// objects the page is given: name, type and base64 bytes. They come from the
/// agent's own workspace only: where srv launched a host agent (not its block's
/// `cmd:cwd`, which any client can rewrite, spec §5.3), or the folder behind a
/// container agent's `/workspace`, whose answers never name the host folder.
/// Up to 25 MB of file I/O and base64 happens off the async workers.
pub(crate) async fn files_for_agent(own: &str, paths: Vec<String>) -> Result<Vec<serde_json::Value>, (axum::http::StatusCode, String)> {
    use axum::http::StatusCode;
    let host = agent_workspace(own);
    let in_container = host.is_none();
    let Some(workspace) = host.or_else(|| container_workspace(own)) else {
        return Err((
            StatusCode::FORBIDDEN,
            "uploads come only from an agent's own workspace, and this agent has none (no launch directory)".to_string(),
        ));
    };
    let requested = if in_container { map_container_paths(&workspace, &paths).map_err(|e| (StatusCode::FORBIDDEN, e))? } else { paths };
    let host_dir = workspace.to_string_lossy().into_owned();
    let read = tokio::task::spawn_blocking(move || {
        use base64::Engine as _;
        let checked = check_upload_paths(&workspace, &requested)?;
        let files = read_upload_files(&workspace, &checked)?;
        Ok::<_, String>(
            files
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "name": f.name,
                        "type": f.mime,
                        "data": base64::engine::general_purpose::STANDARD.encode(&f.bytes),
                    })
                })
                .collect::<Vec<serde_json::Value>>(),
        )
    })
    .await;
    match read {
        Ok(Ok(files)) => Ok(files),
        Ok(Err(e)) if in_container => {
            let real = canonical(Path::new(&host_dir)).map(|p| p.to_string_lossy().into_owned()).ok();
            let e = [Some(host_dir), real].into_iter().flatten().fold(e, |e, h| e.replace(&h, CONTAINER_WORKSPACE));
            Err((StatusCode::FORBIDDEN, container_view(e)))
        }
        Ok(Err(e)) => Err((StatusCode::FORBIDDEN, e)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("reading the files failed: {e}"))),
    }
}

/// `canonicalize` without Windows' `\\?\` verbatim prefix, which the browser
/// doesn't expect in a file path.
pub(crate) fn canonical(p: &Path) -> std::io::Result<PathBuf> {
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
                "uploads are limited to {} MB in all; for bigger files, ask the user to pick them (Browser's handoff action)",
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
/// picker would give it. An extension not known here gets no type at all
/// (empty), as the picker gives one it doesn't know, rather than a forced
/// generic one a page might reject.
fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "htm" | "html" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "rtf" => "application/rtf",
        "json" => "application/json",
        "xml" => "text/xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        "mpeg" | "mpg" => "video/mpeg",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "exe" | "dll" => "application/x-msdownload",
        "msi" => "application/x-msi",
        "apk" => "application/vnd.android.package-archive",
        _ => "",
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
    fn a_container_agents_workspace_paths_map_to_the_host_folder_and_stay_inside_it() {
        let (_d, ws) = ws();
        let mapped = map_container_paths(&ws, &["/workspace/report.txt".into(), "sub/a.bin".into(), "/workspace/sub/a.bin".into()]).unwrap();
        let checked = check_upload_paths(&ws, &mapped).unwrap();
        let ws_real = canonical(&ws).unwrap();
        assert_eq!(checked[0], ws_real.join("report.txt").to_string_lossy());
        assert_eq!(checked[1], ws_real.join("sub").join("a.bin").to_string_lossy());
        assert_eq!(checked[2], checked[1], "absolute and relative name the same file");
        // Anywhere else in the container isn't on the host.
        for outside in ["/etc/passwd", "/home/agent/x.txt", "/workspaces/x", "C:/Users/me/x.txt"] {
            let why = map_container_paths(&ws, &[outside.into()]).unwrap_err();
            assert!(why.contains("outside /workspace"), "{outside}: {why}");
        }
        // `..` would reach the host folders beside the workspace: refused before any lookup.
        for climb in ["/workspace/../secret.txt", "../secret.txt", "sub/../../secret.txt", "/workspace/sub/../../x"] {
            let why = map_container_paths(&ws, &[climb.into()]).unwrap_err();
            assert!(why.contains("outside /workspace"), "{climb}: {why}");
        }
    }

    #[test]
    fn a_container_agent_cant_tell_a_missing_file_from_one_outside_its_workspace() {
        let outside = "link.txt is outside your workspace (/work/agent); an agent can upload only files in its own workspace: copy the file there first";
        assert_eq!(container_view(outside.into()), "no such file: link.txt");
        assert_eq!(container_view("no such file: gone.txt".into()), "no such file: gone.txt");
        assert_eq!(container_view("at most 10 files per upload".into()), "at most 10 files per upload");
    }

    #[test]
    fn the_container_workspace_is_recorded_per_block_and_forgotten_without_a_folder() {
        record_container_workspace("ws-test-container", Some("/some/work"));
        assert_eq!(container_workspace("ws-test-container"), Some(PathBuf::from("/some/work")));
        record_container_workspace("ws-test-container", None);
        assert_eq!(container_workspace("ws-test-container"), None);
        record_container_workspace("ws-test-container", Some("  "));
        assert_eq!(container_workspace("ws-test-container"), None);
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
        // Unknown to the table: no type, as a browser's picker gives it.
        assert_eq!(files[1].mime, "");
        assert_eq!(mime_for("Deck.PPTX"), "application/vnd.openxmlformats-officedocument.presentationml.presentation");
        assert_eq!(mime_for("clip.mov"), "video/quicktime");
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
