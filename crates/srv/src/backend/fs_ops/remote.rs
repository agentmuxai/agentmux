// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Hangar's file operations on an SSH connection (spec §6.3 of
//! SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md): the same
//! requests and results as a local folder, carried out by the host's helper
//! (`remote::files`). Paths there are the host's POSIX paths; `~` is its home.
//!
//! The same rules as a WSL distro's folders hold (`wsl_fs`): a host's system
//! folders, its root and the home folder itself are never changed, only read.
//! There is no Trash on a remote host: deleting there is permanent, and the
//! UI says so before it does.

use agentmux_remote::fsproto::{Entry, ErrKind, Kind};

use crate::backend::remote::files::{self, RemoteError, RemoteFiles};
use crate::backend::remote::sessions::AskIn;
use crate::backend::rpc_types::{
    FsCreateKind, FsEntry, FsError, FsErrorKind, FsListResult, FsOpResult,
};

/// Folders on a host that are read, never changed (Linux and macOS).
const SYSTEM_DIRS: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/etc",
    "/var",
    "/boot",
    "/lib",
    "/lib32",
    "/lib64",
    "/proc",
    "/sys",
    "/dev",
    "/mnt",
    "/run",
    "/opt",
    "/System",
    "/Library",
    "/Applications",
    "/private",
    "/cores",
];

/// Where a request runs: an SSH connection's name and the pane asking (whose
/// window shows any ssh prompt).
#[derive(Debug, Clone, Copy)]
pub struct Remote<'a> {
    pub connection: &'a str,
    pub block_id: Option<&'a str>,
    pub auth_key: &'a str,
}

/// The SSH connection a request names, if it names one: local and WSL
/// requests (and none at all) go the usual way.
pub fn ssh_connection(connection: Option<&str>) -> Option<&str> {
    let c = connection?.trim();
    matches!(
        crate::backend::remote::ConnTarget::parse(c),
        Ok(crate::backend::remote::ConnTarget::Ssh(_))
    )
    .then_some(c)
}

async fn open(r: Remote<'_>) -> Result<std::sync::Arc<RemoteFiles>, String> {
    let ask = r.block_id.filter(|b| !b.is_empty()).map(|block_id| AskIn {
        block_id,
        auth_key: r.auth_key,
    });
    files::connect(r.connection, ask)
        .await
        .map_err(|e| e.message)
}

/// `path` on the host, absolute and tidy: `~` and a relative path under
/// `home`, `.` and empty parts dropped, `..` taken back a level (never above
/// `/`), no trailing `/`.
pub fn resolve(home: &str, path: &str) -> String {
    let path = path.trim();
    let full = if path.is_empty() || path == "~" {
        home.to_string()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{home}/{path}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in full.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

fn parent(path: &str) -> Option<&str> {
    if path == "/" {
        return None;
    }
    Some(match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => "/",
    })
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Whether changing `path` (absolute, [`resolve`]d) is refused: the root, a
/// system folder or anything in one, or the home folder itself.
pub fn protected(home: &str, path: &str) -> bool {
    path == "/"
        || path == home
        || SYSTEM_DIRS
            .iter()
            .any(|d| path == *d || path.starts_with(&format!("{d}/")))
}

fn refusal(conn: &str, path: &str) -> String {
    format!("AgentMux doesn't change {path} on {conn}: it's a system folder or your home folder.")
}

fn entry(e: Entry) -> FsEntry {
    FsEntry {
        hidden: e.name.starts_with('.'),
        // No write bit for anyone: read-only (0 is "unknown", not read-only).
        readonly: e.mode != 0 && e.mode & 0o222 == 0,
        is_dir: e.kind == Kind::Dir,
        is_symlink: e.symlink,
        size: (e.kind != Kind::Dir).then_some(e.size),
        mtime: (e.mtime_ms > 0).then_some(e.mtime_ms as u64),
        link_target: (!e.link_target.is_empty()).then_some(e.link_target),
        error: None,
        name: e.name,
    }
}

fn list_error(e: &RemoteError) -> FsError {
    FsError {
        kind: match e.kind {
            Some(ErrKind::NotFound) => FsErrorKind::NotFound,
            Some(ErrKind::PermissionDenied) => FsErrorKind::PermissionDenied,
            Some(ErrKind::NotADirectory) => FsErrorKind::NotADirectory,
            _ => FsErrorKind::Other,
        },
        message: e.message.clone(),
    }
}

/// `fs.list` on a host: the whole folder in one page (the helper pages it
/// underneath), in its canonical form.
pub async fn list(r: Remote<'_>, path: &str) -> Result<FsListResult, String> {
    let f = open(r).await?;
    let dir = resolve(&f.home, path);
    Ok(match f.list(&dir).await {
        Ok(entries) => FsListResult {
            path: dir,
            entries: entries.into_iter().map(entry).collect(),
            cursor: None,
            error: None,
        },
        Err(e) => FsListResult {
            path: dir,
            entries: Vec::new(),
            cursor: None,
            error: Some(list_error(&e)),
        },
    })
}

fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err("That name can't be used.".to_string());
    }
    Ok(())
}

fn friendly(e: RemoteError, what: &str, name: &str) -> String {
    match e.kind {
        Some(ErrKind::AlreadyExists) => format!("\"{name}\" already exists here."),
        Some(ErrKind::NotFound) => "It isn't there anymore.".to_string(),
        Some(ErrKind::PermissionDenied) => format!("You don't have permission to {what} that."),
        _ => format!("Couldn't {what} it: {}", e.message),
    }
}

/// `fs.rename` on a host; the new path.
pub async fn rename(r: Remote<'_>, path: &str, new_name: &str) -> Result<String, String> {
    valid_name(new_name)?;
    let f = open(r).await?;
    let src = resolve(&f.home, path);
    let dir = parent(&src).ok_or_else(|| refusal(r.connection, &src))?;
    let dest = join(dir, new_name);
    for p in [&src, &dest] {
        if protected(&f.home, p) {
            return Err(refusal(r.connection, p));
        }
    }
    if src == dest {
        return Ok(dest);
    }
    let result = f
        .rename(&src, &dest)
        .await
        .map_err(|e| friendly(e, "rename", new_name));
    tracing::info!(op = "fs.rename", connection = %r.connection, from = %src, to = %dest,
        outcome = if result.is_ok() { "ok" } else { "error" }, "remote fs mutation");
    result.map(|()| dest)
}

/// `fs.create` on a host; the new path.
pub async fn create(
    r: Remote<'_>,
    parent_dir: &str,
    name: &str,
    kind: FsCreateKind,
) -> Result<String, String> {
    valid_name(name)?;
    let f = open(r).await?;
    let dir = resolve(&f.home, parent_dir);
    let target = join(&dir, name);
    if protected(&f.home, &target) {
        return Err(refusal(r.connection, &target));
    }
    match f.stat(&target).await {
        Ok(_) => return Err(format!("\"{name}\" already exists here.")),
        Err(e) if e.is_not_found() => {}
        Err(e) => return Err(friendly(e, "create", name)),
    }
    let result = match kind {
        FsCreateKind::Dir => f.mkdir(&target, false).await,
        FsCreateKind::File => f.write(&target, Vec::new()).await,
    }
    .map_err(|e| friendly(e, "create", name));
    tracing::info!(op = "fs.create", connection = %r.connection, path = %target,
        outcome = if result.is_ok() { "ok" } else { "error" }, "remote fs mutation");
    result.map(|()| target)
}

/// `fs.delete` on a host: PERMANENT, each path on its own. A link is
/// removed, never followed (the helper's rule).
pub async fn delete(r: Remote<'_>, paths: &[String]) -> Vec<FsOpResult> {
    let f = match open(r).await {
        Ok(f) => f,
        Err(e) => {
            return paths
                .iter()
                .map(|p| FsOpResult {
                    path: p.clone(),
                    ok: false,
                    error: Some(e.clone()),
                })
                .collect()
        }
    };
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let target = resolve(&f.home, p);
        let result = if protected(&f.home, &target) {
            Err(refusal(r.connection, &target))
        } else {
            f.delete(&target, true)
                .await
                .map_err(|e| friendly(e, "delete", &target))
        };
        tracing::info!(op = "fs.delete", connection = %r.connection, path = %target,
            outcome = if result.is_ok() { "ok" } else { "error" }, "remote fs mutation");
        out.push(FsOpResult {
            path: p.clone(),
            ok: result.is_ok(),
            error: result.err(),
        });
    }
    out
}

/// What `fs.trash` (and `fs.restore`) say for a remote path: there is no
/// Trash there.
pub fn no_trash(connection: &str) -> String {
    format!("{connection} has no Trash: delete permanently instead.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_under_home_and_stay_tidy() {
        let h = "/home/u";
        assert_eq!(resolve(h, "~"), "/home/u");
        assert_eq!(resolve(h, ""), "/home/u");
        assert_eq!(resolve(h, "~/a/./b/"), "/home/u/a/b");
        assert_eq!(resolve(h, "src"), "/home/u/src");
        assert_eq!(resolve(h, "/etc/../srv//x"), "/srv/x");
        assert_eq!(resolve(h, "/../.."), "/");
        assert_eq!(parent("/home/u/a"), Some("/home/u"));
        assert_eq!(parent("/a"), Some("/"));
        assert_eq!(parent("/"), None);
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/home/u", "a"), "/home/u/a");
    }

    #[test]
    fn system_folders_root_and_home_are_never_changed() {
        let h = "/home/u";
        for p in [
            "/",
            "/home/u",
            "/etc",
            "/etc/hosts",
            "/usr/bin/x",
            "/System/Library",
            "/opt",
        ] {
            assert!(protected(h, p), "{p}");
        }
        for p in [
            "/home/u/a",
            "/srv/data",
            "/tmp/x",
            "/home/other/a",
            "/etcetera",
        ] {
            assert!(!protected(h, p), "{p}");
        }
    }

    #[test]
    fn helper_entries_read_like_local_ones() {
        let e = entry(Entry {
            name: ".bashrc".into(),
            kind: Kind::File,
            size: 10,
            mtime_ms: 1_700_000_000_000,
            mode: 0o444,
            symlink: true,
            link_target: "dotfiles/bashrc".into(),
        });
        assert!(e.hidden && e.readonly && e.is_symlink && !e.is_dir);
        assert_eq!((e.size, e.mtime), (Some(10), Some(1_700_000_000_000)));
        assert_eq!(e.link_target.as_deref(), Some("dotfiles/bashrc"));
        let d = entry(Entry {
            name: "src".into(),
            kind: Kind::Dir,
            size: 4096,
            mtime_ms: 0,
            mode: 0,
            symlink: false,
            link_target: String::new(),
        });
        assert!(d.is_dir && !d.readonly && !d.hidden);
        assert_eq!((d.size, d.mtime, d.link_target), (None, None, None));
    }

    /// Hangar's operations end to end against the real helper (in this
    /// process, through the pool as over ssh). Linux: a host's paths are
    /// POSIX, so the test home's must be, and macOS keeps temp folders under
    /// /var, which is a system folder here.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn hangar_lists_creates_renames_and_deletes_on_a_host() {
        let home = tempfile::tempdir().unwrap();
        let h = home.path().to_string_lossy().into_owned();
        let conn = "fsremote-test-host";
        files::connect_in_process(conn, home.path().to_path_buf()).await;
        let r = Remote {
            connection: conn,
            block_id: None,
            auth_key: "",
        };

        let dir = create(r, "~", "proj", FsCreateKind::Dir).await.unwrap();
        assert_eq!(dir, format!("{h}/proj"));
        let file = create(r, "~/proj", "a.txt", FsCreateKind::File)
            .await
            .unwrap();
        assert!(create(r, "~/proj", "a.txt", FsCreateKind::File)
            .await
            .unwrap_err()
            .contains("already exists"));
        assert!(create(r, "~/proj", "../x", FsCreateKind::File)
            .await
            .is_err());

        let listed = list(r, "proj/").await.unwrap();
        assert_eq!(listed.path, format!("{h}/proj"));
        assert_eq!(listed.entries.len(), 1);
        assert_eq!(listed.entries[0].name, "a.txt");
        assert_eq!(listed.entries[0].size, Some(0));

        let moved = rename(r, &file, "b.txt").await.unwrap();
        assert_eq!(moved, format!("{h}/proj/b.txt"));
        let missing = list(r, "~/nope").await.unwrap();
        assert_eq!(missing.error.unwrap().kind, FsErrorKind::NotFound);

        // Home itself and the root are never changed.
        assert!(rename(r, "~", "elsewhere")
            .await
            .unwrap_err()
            .contains("doesn't change"));
        let out = delete(r, &["~".into(), "/etc".into(), "~/proj".into()]).await;
        assert_eq!(
            out.iter().map(|o| o.ok).collect::<Vec<_>>(),
            vec![false, false, true]
        );
        assert!(!home.path().join("proj").exists());
        assert!(home.path().exists());
    }

    #[test]
    fn only_ssh_connections_go_remote() {
        assert_eq!(ssh_connection(Some("user@box")), Some("user@box"));
        assert_eq!(ssh_connection(Some(" box:2222 ")), Some("box:2222"));
        assert_eq!(ssh_connection(Some("local")), None);
        assert_eq!(ssh_connection(Some("wsl://Ubuntu")), None);
        assert_eq!(ssh_connection(Some("")), None);
        assert_eq!(ssh_connection(None), None);
    }
}
