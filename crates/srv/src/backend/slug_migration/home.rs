// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a migration looks: the AgentMux root and every install on it, read
//! from the disk rather than from the running instance, so every channel and
//! version is covered, running or not (S2's "all channels and versions").

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

/// The AgentMux root a migration works on.
#[derive(Debug, Clone)]
pub struct Home {
    pub root: PathBuf,
    /// `shared/`, beside `channels/` and `dev/`.
    pub shared: PathBuf,
    /// bashwrap's cwd files (K11). bashwrap reads `~/.agentmux` from the
    /// user's home whatever the overrides say, so a test or an isolated
    /// `task dev` gets one under its own root instead.
    pub bashwrap_dir: PathBuf,
}

impl Home {
    /// A root holding `shared/`, `channels/` and `dev/` (tests, fixtures).
    pub fn at(root: &Path) -> Home {
        Home {
            root: root.to_path_buf(),
            shared: root.join("shared"),
            bashwrap_dir: root.join("state").join("bashwrap-cwd"),
        }
    }

    /// The root this srv runs on: `AGENTMUX_HOME_OVERRIDE` when it is set,
    /// else the root captured at startup.
    pub fn from_env() -> Result<Home, String> {
        if let Some(over) = std::env::var_os("AGENTMUX_HOME_OVERRIDE").filter(|s| !s.is_empty()) {
            return Ok(Home::at(Path::new(&over)));
        }
        let root = crate::srv_info::agentmux_root().ok_or("the AgentMux root can't be resolved")?;
        let mut home = Home::at(&root);
        if let Some(shared) = crate::registry::resolve_global_shared_root() {
            home.shared = shared;
        }
        if let Some(user) = dirs::home_dir() {
            home.bashwrap_dir = user.join(".agentmux").join("state").join("bashwrap-cwd");
        }
        Ok(home)
    }

    pub fn definitions(&self) -> PathBuf {
        self.shared.join("agents").join("definitions")
    }
    pub fn reactive(&self) -> PathBuf {
        self.shared.join("agents").join("reactive")
    }
    pub fn leases(&self) -> PathBuf {
        self.shared.join("agents").join("registry").join("leases")
    }
    pub fn lock_file(&self) -> PathBuf {
        self.shared.join("locks").join("slug-migration.lock")
    }
    pub fn backups(&self) -> PathBuf {
        self.shared.join("backups")
    }

    /// The host-wide stores that hold name-keyed rows: `store.db` (cron jobs,
    /// MuxBus credentials) and `identity-store.db` (the work queue), plus an
    /// install's own `identity-store.db` under isolated auth.
    pub fn host_stores(&self, installs: &[Install]) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = ["store.db", "identity-store.db"].iter().map(|f| self.shared.join(f)).collect();
        out.extend(installs.iter().map(|i| i.dir.join("identity-store.db")));
        out.retain(|p| p.is_file());
        out
    }

    /// Every install on this root, each with its versions' data directories:
    /// `channels/<ch>/versions/<v>/data` (and `channels/<ch>/data`, a dev
    /// build pointed at a channel), `dev/<branch>/data` and
    /// `dev/<branch>/<clone>/data`. A directory that exists but can't be
    /// listed is an error: a migration must not miss a copy of the slug.
    pub fn installs(&self) -> Result<Vec<Install>, String> {
        let mut out = Vec::new();
        for ch in list_dirs(&self.root.join("channels"))? {
            let name = format!("channels/{}", file_name(&ch));
            let mut versions = Vec::new();
            if has_objects_db(&ch.join("data")) {
                versions.push(VersionData { label: String::new(), data: ch.join("data") });
            }
            for v in list_dirs(&ch.join("versions"))? {
                if has_objects_db(&v.join("data")) {
                    versions.push(VersionData { label: file_name(&v), data: v.join("data") });
                }
            }
            out.push(Install { name, dir: ch, versions });
        }
        for branch in list_dirs(&self.root.join("dev"))? {
            if has_objects_db(&branch.join("data")) {
                let name = format!("dev/{}", file_name(&branch));
                out.push(Install { name, versions: vec![VersionData { label: String::new(), data: branch.join("data") }], dir: branch });
                continue;
            }
            for clone in list_dirs(&branch)? {
                if has_objects_db(&clone.join("data")) {
                    let name = format!("dev/{}/{}", file_name(&branch), file_name(&clone));
                    out.push(Install { name, versions: vec![VersionData { label: String::new(), data: clone.join("data") }], dir: clone });
                }
            }
        }
        Ok(out)
    }
}

/// One channel, or one dev checkout: what it shares across its versions.
#[derive(Debug, Clone)]
pub struct Install {
    pub name: String,
    pub dir: PathBuf,
    pub versions: Vec<VersionData>,
}

impl Install {
    /// Where its `gh-<slug>` folders live (K10).
    pub fn config_dir(&self) -> PathBuf {
        self.dir.join("config")
    }
    /// Its `wan.db` (K5), when it has one.
    pub fn wan_db(&self) -> Option<PathBuf> {
        Some(self.dir.join("wan-identity").join("wan.db")).filter(|p| p.is_file())
    }
}

/// One version's data directory.
#[derive(Debug, Clone)]
pub struct VersionData {
    /// The version folder's name; empty for a dev or unversioned install.
    pub label: String,
    pub data: PathBuf,
}

impl VersionData {
    pub fn objects_db(&self) -> PathBuf {
        self.data.join("db").join("objects.db")
    }
    /// Its per-version registrations (K9).
    pub fn registrations(&self) -> PathBuf {
        self.data.join("agents")
    }
    /// The srv lock, which holds the running srv's pid.
    pub fn srv_lock(&self) -> PathBuf {
        self.data.join(crate::backend::base::SRV_DATA_LOCK_FILE)
    }
    /// What the srv running here can do (`stamp.rs`).
    pub fn stamp(&self) -> PathBuf {
        self.data.join(super::stamp::STAMP_FILE)
    }
}

/// `<install>` or `<install>@<version>`, for reports.
pub fn place(install: &Install, version: &VersionData) -> String {
    if version.label.is_empty() {
        install.name.clone()
    } else {
        format!("{}@{}", install.name, version.label)
    }
}

fn has_objects_db(data: &Path) -> bool {
    data.join("db").join("objects.db").is_file()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The subdirectories of `dir`, sorted; none when it doesn't exist.
fn list_dirs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{} can't be listed: {e}", dir.display())),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("{} can't be listed: {e}", dir.display()))?.path();
        if path.is_dir() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// A database opened to read, never to change: no schema step, no pragma
/// that writes.
pub fn open_ro(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
    Ok(conn)
}

/// A database opened to write, as it is: an existing file only, and no
/// schema step. Another version's `objects.db` must not be upgraded by
/// opening it through `Store::open`.
pub fn open_rw(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
    Ok(conn)
}

pub fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [table], |_| Ok(()))
        .is_ok()
}

/// Whether an `objects.db` has agent slugs to read; one older than
/// `db_agents.slug` (or `db_agents` itself) holds none.
pub fn has_agent_slugs(conn: &Connection) -> bool {
    has_column(conn, "db_agents", "slug")
}

pub fn has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else { return false };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else { return false };
    let columns: Vec<String> = rows.flatten().collect();
    columns.iter().any(|c| c == column)
}

/// Run `f` in one `BEGIN IMMEDIATE` transaction: committed if it returns
/// `Ok`, rolled back otherwise.
pub fn in_tx<T>(conn: &Connection, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
    match f(conn) {
        Ok(v) => {
            conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
            Ok(v)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Write `bytes` to `path` through a temporary file beside it and a rename,
/// never wider than the file it replaces. On Unix the temporary file is
/// created owner-only (0600) from the start, so the bytes are never readable
/// by others for a moment. A file being replaced then keeps its own mode,
/// except a `.mcp.json`, which holds signing keys and is always 0600, as
/// AgentMux writes it (`Workdir::write(.., owner_only = true)`); a new file
/// stays 0600. On Windows the file takes its folder's ACL, as every write
/// through `Workdir` does: the profile's per-user ACL.
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let dir = path.parent().ok_or("no parent directory")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!(".{name}.{}.slug-migration", uuid::Uuid::new_v4()));
    let err = |e: std::io::Error| format!("{}: {e}", tmp.display());
    #[cfg(unix)]
    let keep = (name != ".mcp.json").then(|| std::fs::metadata(path).ok().map(|m| m.permissions())).flatten();
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let written = (|| {
        let mut f = opts.open(&tmp).map_err(err)?;
        #[cfg(unix)]
        if let Some(perms) = keep {
            f.set_permissions(perms).map_err(err)?;
        }
        f.write_all(bytes).map_err(err)?;
        f.sync_all().map_err(err)
    })();
    let result = written.and_then(|()| {
        crate::backend::workdir_fs::rename_with_retry(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Make `path` owner-only on Unix: 0700 for a folder, 0600 for a file. On
/// Windows it keeps the per-user ACL of the profile it lives in.
pub fn owner_only(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = if path.is_dir() { 0o700 } else { 0o600 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// A name as every key table folds it.
pub fn fold(name: &str) -> String {
    name.trim().to_lowercase()
}
