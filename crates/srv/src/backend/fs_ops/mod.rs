// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The filesystem layer behind the generic `fs.*` RPCs and the Files pane
//! ("Hangar"): paginated listing, known places, name validation, the
//! mutation safety policy, and the mutations themselves.
//!
//! Everything here is blocking and is called from `spawn_blocking` (or the
//! trash thread): on macOS the first access to a protected folder blocks the
//! calling thread while a privacy prompt is on screen (spec §9.1.1 fact 10).
//!
//! Spec: docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.3, §7, §9.

pub mod jobs;
pub mod platform;
pub mod trash_worker;

use std::collections::HashMap;
use std::io;
use std::iter::Peekable;
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::backend::rpc_types::{
    FsCreateKind, FsCreateReq, FsCreateResult, FsEntry, FsError, FsErrorKind, FsListReq,
    FsListResult, FsPlace, FsPlaceKind, FsRenameReq, FsRenameResult,
};

use platform::display_path;

// ── Paths ────────────────────────────────────────────────────────────────

/// A request path as an absolute host path: `~` expanded, an MSYS-style
/// `/c/…` converted on Windows, NUL refused, relative refused (a relative
/// path would resolve against srv's own working directory, which means
/// nothing to the user).
pub fn resolve_request_path(raw: &str) -> Result<PathBuf, String> {
    if raw.contains('\0') {
        return Err("That path contains an invalid character.".to_string());
    }
    if raw.is_empty() {
        return Err("No path was given.".to_string());
    }
    let converted = crate::backend::base::msys_to_windows_path(raw);
    let expanded = crate::backend::base::expand_home_dir_safe(&converted);
    if !expanded.is_absolute() {
        return Err(format!("“{raw}” isn't a full path."));
    }
    Ok(expanded)
}

/// The path a mutation acts on: the request path with its PARENT
/// canonicalized and its own last component kept as given.
///
/// Canonicalizing the whole path would resolve a final symlink to its
/// target, and a rename or delete would then act on the target instead of
/// the link. Canonicalizing the parent still resolves `..`, `~` and any
/// linked folder above it, so protection checks see where the entry really
/// is.
pub fn resolve_entry_path(raw: &str) -> Result<PathBuf, String> {
    let path = resolve_request_path(raw)?;
    let name = path.file_name().ok_or_else(|| {
        if path.parent().is_none() {
            ROOT_REFUSAL.to_string()
        } else {
            format!("“{raw}” isn't a valid path.")
        }
    })?;
    let parent = path.parent().ok_or_else(|| ROOT_REFUSAL.to_string())?;
    let canonical_parent = parent.canonicalize().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => "The folder it's in doesn't exist anymore.".to_string(),
        _ => format!("Couldn't open the folder it's in: {e}"),
    })?;
    Ok(canonical_parent.join(name))
}

/// Comparison key for paths: the display form, case-folded on Windows,
/// whose filesystems are case-insensitive. Component-wise `starts_with` on
/// these is a correct "is inside" test.
fn path_key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(display_path(path).to_lowercase())
    } else {
        path.to_path_buf()
    }
}

fn file_name_string(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

// ── Listing ──────────────────────────────────────────────────────────────

/// Page size when the request names none.
pub const DEFAULT_LIST_LIMIT: u32 = 1000;
/// Largest page a request may ask for.
pub const MAX_LIST_LIMIT: u32 = 5000;
/// A cursor is dropped after this long without a page being asked for. The
/// frontend navigating away simply stops asking, so this is the
/// cancellation (spec §6.3).
pub const CURSOR_TTL: Duration = Duration::from_secs(30);
/// Open listings held at once. Each holds an OS directory handle, so a
/// burst of abandoned listings is bounded rather than left to the TTL.
const MAX_CURSORS: usize = 256;
/// Consecutive unreadable directory entries after which a listing gives up,
/// so a directory whose iterator keeps failing ends instead of looping.
const MAX_CONSECUTIVE_ENTRY_ERRORS: usize = 64;

/// An open listing between pages.
pub struct ListCursor {
    iter: Peekable<std::fs::ReadDir>,
    display: String,
    last_used: Instant,
}

/// Open listings by cursor id.
#[derive(Default)]
pub struct CursorStore {
    cursors: HashMap<String, ListCursor>,
}

impl CursorStore {
    /// Remove and return the cursor `id`, unless it has expired.
    fn take(&mut self, id: &str, now: Instant) -> Option<ListCursor> {
        let cursor = self.cursors.remove(id)?;
        (now.saturating_duration_since(cursor.last_used) <= CURSOR_TTL).then_some(cursor)
    }

    /// Store `cursor` under a new id and return the id. Drops expired
    /// cursors first, then the least recently used if still at capacity.
    fn put(&mut self, mut cursor: ListCursor, now: Instant) -> String {
        self.sweep(now);
        while self.cursors.len() >= MAX_CURSORS {
            let Some(oldest) = self
                .cursors
                .iter()
                .min_by_key(|(_, c)| c.last_used)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            self.cursors.remove(&oldest);
        }
        cursor.last_used = now;
        let id = uuid::Uuid::new_v4().to_string();
        self.cursors.insert(id.clone(), cursor);
        id
    }

    /// Drop every cursor idle for longer than the TTL, closing its directory
    /// handle.
    pub fn sweep(&mut self, now: Instant) {
        self.cursors
            .retain(|_, c| now.saturating_duration_since(c.last_used) <= CURSOR_TTL);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.cursors.len()
    }
}

/// The process-wide cursor store. Process-wide rather than per connection
/// because a window reload reconnects; a cursor id is a random UUID, so one
/// connection can't guess another's.
pub static CURSORS: LazyLock<Mutex<CursorStore>> = LazyLock::new(|| Mutex::new(CursorStore::default()));

/// Start the periodic cursor sweep, once per process. Without it, cursors
/// abandoned after the last `fs.list` call would keep their directory
/// handles open until the next call.
pub fn ensure_cursor_sweeper() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        tokio::spawn(async {
            let mut tick = tokio::time::interval(Duration::from_secs(10));
            loop {
                tick.tick().await;
                lock_store(&CURSORS).sweep(Instant::now());
            }
        });
    });
}

/// One page of `fs.list`, against the process-wide cursor store.
pub fn list_page(req: &FsListReq) -> FsListResult {
    list_page_in(&CURSORS, req, Instant::now())
}

/// One page of `fs.list`. A failure to open the directory is an `Ok`
/// answer with `error` set, never an `Err` (spec §9: a denied folder must
/// never look empty, and must never fail the pane).
pub fn list_page_in(store: &Mutex<CursorStore>, req: &FsListReq, now: Instant) -> FsListResult {
    let limit = req.limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT) as usize;

    let mut cursor = match req.cursor.as_deref() {
        Some(id) => {
            let taken = lock_store(store).take(id, now);
            match taken {
                Some(c) => c,
                None => {
                    let path = resolve_request_path(&req.path)
                        .map(|p| display_path(&p))
                        .unwrap_or_else(|_| req.path.clone());
                    return list_failure(path, FsErrorKind::Expired, "This listing expired. Reload the folder.".to_string());
                }
            }
        }
        None => match open_listing(&req.path) {
            Ok(c) => c,
            Err(failure) => return failure,
        },
    };

    let mut entries = Vec::with_capacity(limit.min(1024));
    let mut consecutive_errors = 0usize;
    let mut gave_up = false;
    while entries.len() < limit {
        match cursor.iter.next() {
            None => break,
            Some(Ok(de)) => {
                consecutive_errors = 0;
                entries.push(build_entry(&de));
            }
            // An unreadable directory entry has no name to show a row for.
            Some(Err(e)) => {
                tracing::debug!(dir = %cursor.display, error = %e, "fs.list: skipping an unreadable directory entry");
                consecutive_errors += 1;
                if consecutive_errors >= MAX_CONSECUTIVE_ENTRY_ERRORS {
                    tracing::warn!(dir = %cursor.display, "fs.list: too many unreadable entries in a row; ending the listing");
                    gave_up = true;
                    break;
                }
            }
        }
    }

    let display = cursor.display.clone();
    let done = gave_up || cursor.iter.peek().is_none();
    let next = if done {
        None
    } else {
        Some(lock_store(store).put(cursor, now))
    };
    FsListResult { path: display, entries, cursor: next, error: None }
}

/// The store's lock, recovered if a panic elsewhere poisoned it: the map is
/// left consistent by every operation on it, and a poisoned lock must not
/// silently end every listing.
fn lock_store(store: &Mutex<CursorStore>) -> std::sync::MutexGuard<'_, CursorStore> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

fn list_failure(path: String, kind: FsErrorKind, message: String) -> FsListResult {
    FsListResult { path, entries: Vec::new(), cursor: None, error: Some(FsError { kind, message }) }
}

/// Canonicalize and open the directory for a first page.
fn open_listing(raw: &str) -> Result<ListCursor, FsListResult> {
    let path = resolve_request_path(raw)
        .map_err(|msg| list_failure(raw.to_string(), FsErrorKind::Other, msg))?;
    let canonical = path.canonicalize().map_err(|e| folder_failure(display_path(&path), &e))?;
    let display = display_path(&canonical);
    match std::fs::metadata(&canonical) {
        Ok(meta) if !meta.is_dir() => {
            return Err(list_failure(display, FsErrorKind::NotADirectory, "This isn't a folder.".to_string()));
        }
        Ok(_) => {}
        Err(e) => return Err(folder_failure(display, &e)),
    }
    let iter = std::fs::read_dir(&canonical).map_err(|e| folder_failure(display.clone(), &e))?;
    Ok(ListCursor { iter: iter.peekable(), display, last_used: Instant::now() })
}

/// The `fs.list` error for a folder that couldn't be opened.
fn folder_failure(display: String, err: &io::Error) -> FsListResult {
    let (kind, message) = classify_folder_error(err);
    list_failure(display, kind, message)
}

pub fn classify_folder_error(err: &io::Error) -> (FsErrorKind, String) {
    match err.kind() {
        io::ErrorKind::NotFound => (FsErrorKind::NotFound, "This folder doesn't exist.".to_string()),
        io::ErrorKind::PermissionDenied if platform::is_os_blocked(err) => (
            FsErrorKind::OsBlocked,
            "macOS blocked AgentMux from reading this folder. Open System Settings → Privacy & Security → \
             Files & Folders, turn AgentMux on, then try again."
                .to_string(),
        ),
        io::ErrorKind::PermissionDenied => (
            FsErrorKind::PermissionDenied,
            "You don't have permission to open this folder.".to_string(),
        ),
        io::ErrorKind::NotADirectory => (FsErrorKind::NotADirectory, "This isn't a folder.".to_string()),
        _ => (FsErrorKind::Other, format!("Couldn't open this folder: {err}")),
    }
}

/// One listing row. Never fails: what can't be read becomes the row's
/// `error`, and the page goes on (spec §9: errors are per entry).
fn build_entry(de: &std::fs::DirEntry) -> FsEntry {
    let name = de.file_name().to_string_lossy().into_owned();
    let mut entry = FsEntry {
        hidden: name.starts_with('.'),
        name,
        is_dir: false,
        is_symlink: false,
        size: None,
        mtime: None,
        readonly: false,
        link_target: None,
        error: None,
    };

    // `DirEntry::metadata` never follows a link: this is the entry's own
    // metadata, which is where the hidden/system attributes and the link
    // type live. On Windows it costs no extra system call.
    let own = match de.metadata() {
        Ok(m) => m,
        Err(e) => {
            if let Ok(ft) = de.file_type() {
                entry.is_symlink = ft.is_symlink();
                entry.is_dir = ft.is_dir();
            }
            entry.error = Some(entry_error_message(&e));
            return entry;
        }
    };
    entry.hidden |= platform::has_hidden_attribute(&own);
    entry.is_symlink = own.file_type().is_symlink();

    let effective = if entry.is_symlink {
        let path = de.path();
        entry.link_target = std::fs::read_link(&path).ok().map(|t| display_path(&t));
        match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                entry.error = Some(match e.kind() {
                    io::ErrorKind::NotFound => "The link's target doesn't exist.".to_string(),
                    _ => format!("Couldn't follow the link: {e}"),
                });
                return entry;
            }
        }
    } else {
        own
    };

    entry.is_dir = effective.is_dir();
    if !entry.is_dir {
        entry.size = Some(effective.len());
    }
    entry.mtime = effective
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    // Windows ignores the read-only attribute on folders (Explorer uses it
    // to mark customized folders such as Documents), so only files report it.
    entry.readonly = !(cfg!(windows) && entry.is_dir) && effective.permissions().readonly();
    entry
}

fn entry_error_message(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::NotFound => "It was removed while the folder was being read.".to_string(),
        io::ErrorKind::PermissionDenied => "Access denied.".to_string(),
        _ => err.to_string(),
    }
}

// ── Places ───────────────────────────────────────────────────────────────

/// Home and the known folders, by name only. Nothing here reads or stats a
/// known folder: on macOS that alone can raise a privacy prompt the user
/// didn't ask for (spec §9.1.5). A folder `dirs` can't name is left out.
pub fn home_and_known_places() -> (String, Vec<FsPlace>) {
    let home = dirs::home_dir().map(|h| display_path(&h)).unwrap_or_default();
    let mut places = Vec::new();
    if !home.is_empty() {
        places.push(FsPlace { id: "home".to_string(), label: "Home".to_string(), path: home.clone(), kind: FsPlaceKind::Home });
    }
    let known: [(&str, &str, Option<PathBuf>); 4] = [
        ("desktop", "Desktop", dirs::desktop_dir()),
        ("documents", "Documents", dirs::document_dir()),
        ("downloads", "Downloads", dirs::download_dir()),
        ("pictures", "Pictures", dirs::picture_dir()),
    ];
    for (id, label, dir) in known {
        if let Some(dir) = dir {
            places.push(FsPlace { id: id.to_string(), label: label.to_string(), path: display_path(&dir), kind: FsPlaceKind::Known });
        }
    }
    (home, places)
}

// ── Names ────────────────────────────────────────────────────────────────

/// Validate a new file or folder name with the current platform's rules.
pub fn validate_new_name(name: &str) -> Result<(), String> {
    validate_name_rules(name, cfg!(windows))
}

/// Device names Windows reserves in every folder, with or without an
/// extension (`nul.txt` is still the null device). The superscript digits
/// are reserved too.
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "conin$", "conout$", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "com¹", "com²", "com³", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8",
    "lpt9", "lpt¹", "lpt²", "lpt³",
];

/// Longest name most filesystems accept (NTFS: 255 UTF-16 units; ext4 and
/// APFS: 255 bytes). Checked as UTF-16 units and bytes both.
const MAX_NAME_LEN: usize = 255;

/// The name rules (spec §7.4), with the Windows-only ones switched by
/// `windows` so every rule can be tested on any platform. Errors are
/// sentences, shown to the user as they are.
pub fn validate_name_rules(name: &str, windows: bool) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("A name can't be empty.".to_string());
    }
    if name == "." || name == ".." {
        return Err("“.” and “..” can't be used as names.".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("A name can't contain / or \\.".to_string());
    }
    if name.contains('\0') {
        return Err("A name can't contain a NUL character.".to_string());
    }
    if name.len() > MAX_NAME_LEN || name.encode_utf16().count() > MAX_NAME_LEN {
        return Err("That name is too long.".to_string());
    }
    if windows {
        if let Some(c) = name.chars().find(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')) {
            return Err(format!("Windows doesn't allow {c} in a name."));
        }
        if name.chars().any(|c| (c as u32) < 0x20) {
            return Err("A name can't contain control characters.".to_string());
        }
        // Windows silently strips these, so the file would get a different
        // name from the one typed (and a verbatim path would create one that
        // most programs then can't open).
        if name.ends_with('.') || name.ends_with(' ') {
            return Err("Windows doesn't allow a name to end with a dot or a space.".to_string());
        }
        let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ').to_lowercase();
        if WINDOWS_RESERVED.contains(&stem.as_str()) {
            return Err(format!("“{name}” is reserved by Windows. Pick another name."));
        }
    }
    Ok(())
}

// ── Mutation safety ──────────────────────────────────────────────────────

const ROOT_REFUSAL: &str = "AgentMux won't change the root of a drive.";

/// What a mutation must never touch (spec §9, "Security"): a filesystem
/// root, the home folder itself, AgentMux's own data and installation, the
/// OS's system folders, and the inside of a macOS app bundle. Nor anything
/// that CONTAINS one of those: deleting `C:\Users` would delete the home
/// folder too.
///
/// Every path held here is in `path_key` form.
#[derive(Debug, Clone, Default)]
pub struct ProtectedPaths {
    home: Option<PathBuf>,
    /// `~/.agentmux`. Inside it only agents' own workspaces
    /// (`agents/<name>/…`) may be changed; the rest is AgentMux's state.
    data_root: Option<PathBuf>,
    install_dirs: Vec<PathBuf>,
    system_roots: Vec<PathBuf>,
    /// Inside these the system list doesn't apply: the temp folder, which
    /// on macOS lives under `/var`.
    exempt_roots: Vec<PathBuf>,
}

impl ProtectedPaths {
    /// The protected set for this machine, computed afresh so a check made
    /// right before a mutation reflects the filesystem as it is then.
    pub fn current() -> Self {
        let canon = |p: PathBuf| -> PathBuf { path_key(&p.canonicalize().unwrap_or(p)) };
        let home = dirs::home_dir().map(canon);
        let data_root = Some(canon(crate::backend::base::get_mux_data_dir()));

        // A binary sitting directly in home (or anywhere above it) must not
        // turn the whole home folder read-only: protect just the binary then.
        let covers_home = |dir: &Path| home.as_ref().is_some_and(|h| h.starts_with(dir));
        let mut install_dirs = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            let exe = exe.canonicalize().unwrap_or(exe);
            // On macOS the whole bundle is the installation, not just
            // Contents/MacOS.
            let bundle = exe.ancestors().skip(1).find(|a| {
                a.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"))
            });
            if let Some(dir) = bundle.or_else(|| exe.parent()).map(path_key) {
                install_dirs.push(if covers_home(&dir) { path_key(&exe) } else { dir });
            }
        }
        if let Some(app) = crate::backend::base::get_mux_app_path().map(canon) {
            if !covers_home(&app) {
                install_dirs.push(app);
            }
        }

        let system_roots = system_roots().into_iter().map(canon).collect();
        let exempt_roots = vec![canon(std::env::temp_dir())];
        Self { home, data_root, install_dirs, system_roots, exempt_roots }
    }

    /// `Ok` if a mutation may act on `target` (a `resolve_entry_path`
    /// result). The error is a sentence for the user.
    pub fn check(&self, target: &Path) -> Result<(), String> {
        if target.parent().is_none() {
            return Err(ROOT_REFUSAL.to_string());
        }
        let key = path_key(target);
        let contains = |protected: &Path| protected.starts_with(&key);

        if let Some(home) = &self.home {
            if *home == key {
                return Err("AgentMux won't rename, move or delete your home folder.".to_string());
            }
            if contains(home) {
                return Err("This folder contains your home folder, so AgentMux won't change it.".to_string());
            }
        }

        if let Some(data) = &self.data_root {
            if contains(data) {
                return Err("This is, or contains, AgentMux's own data folder; AgentMux won't change it.".to_string());
            }
            if let Ok(rel) = key.strip_prefix(data) {
                let parts: Vec<Component<'_>> = rel.components().collect();
                let in_workspace = parts.len() >= 3 && parts[0].as_os_str() == "agents";
                if !in_workspace {
                    return Err(
                        "This is part of AgentMux's own data. Only files inside an agent's workspace can be changed here."
                            .to_string(),
                    );
                }
            }
        }

        for dir in &self.install_dirs {
            if key.starts_with(dir) || contains(dir) {
                return Err("This is part of the AgentMux installation; AgentMux won't change it.".to_string());
            }
        }

        // The inside of a macOS app bundle: changing it breaks the app's
        // signature, and for apps in /Applications raises an App Management
        // prompt (spec §9.1.5). The bundle itself is treated as a file.
        if cfg!(target_os = "macos")
            && key
                .ancestors()
                .skip(1)
                .any(|a| a.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")))
        {
            return Err("AgentMux won't change the inside of an app bundle.".to_string());
        }

        let inside_home = self.home.as_ref().is_some_and(|h| key.starts_with(h));
        let exempt = self.exempt_roots.iter().any(|r| key.starts_with(r));
        if !inside_home && !exempt {
            for root in &self.system_roots {
                if key.starts_with(root) || contains(root) {
                    return Err("This is a protected system folder; AgentMux won't change it.".to_string());
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn for_test(home: &Path, data_root: &Path, install: &[&Path], system: &[&Path], exempt: &[&Path]) -> Self {
        Self {
            home: Some(path_key(home)),
            data_root: Some(path_key(data_root)),
            install_dirs: install.iter().map(|p| path_key(p)).collect(),
            system_roots: system.iter().map(|p| path_key(p)).collect(),
            exempt_roots: exempt.iter().map(|p| path_key(p)).collect(),
        }
    }
}

/// The OS's own folders (spec §9).
fn system_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut roots = Vec::new();
        for (var, fallback) in [
            ("SystemRoot", r"C:\Windows"),
            ("ProgramFiles", r"C:\Program Files"),
            ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
            ("ProgramW6432", r"C:\Program Files"),
            ("ProgramData", r"C:\ProgramData"),
        ] {
            let dir = std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(fallback));
            if !roots.contains(&dir) {
                roots.push(dir);
            }
        }
        roots
    }
    #[cfg(not(windows))]
    {
        let mut roots: Vec<PathBuf> = ["/usr", "/bin", "/sbin", "/etc", "/var", "/boot", "/lib", "/lib32", "/lib64", "/proc", "/sys", "/dev"]
            .iter()
            .map(PathBuf::from)
            .collect();
        if cfg!(target_os = "macos") {
            // /etc and /var are links into /private, so their canonical
            // forms are what a resolved path will start with.
            roots.extend(["/System", "/Library", "/private/etc", "/private/var"].iter().map(PathBuf::from));
        }
        roots
    }
}

// ── Mutations ────────────────────────────────────────────────────────────
//
// Each re-resolves and re-checks its target immediately before acting, on
// the thread that acts (spec §9: "re-validated at execution time"), and
// logs one info line per operation for audit.

fn exists_message(name: &str) -> String {
    format!("Something named “{name}” already exists here.")
}

fn missing_or(err: io::Error, what: &str) -> String {
    match err.kind() {
        io::ErrorKind::NotFound => "It doesn't exist anymore.".to_string(),
        io::ErrorKind::PermissionDenied => format!("You don't have permission to {what} it."),
        _ => format!("Couldn't {what} it: {err}"),
    }
}

/// `fs.rename`: a new name in the same folder. Never overwrites; a
/// case-only rename goes through a temporary name, because on a
/// case-insensitive filesystem the new name already "exists" (it is the
/// file itself) and some filesystems ignore a direct case-only rename
/// (spec §7.4).
pub fn rename(req: &FsRenameReq) -> Result<FsRenameResult, String> {
    let protect = ProtectedPaths::current();
    let src = resolve_entry_path(&req.path)?;
    validate_new_name(&req.new_name)?;
    std::fs::symlink_metadata(&src).map_err(|e| missing_or(e, "rename"))?;
    protect.check(&src)?;
    let parent = src.parent().ok_or_else(|| ROOT_REFUSAL.to_string())?.to_path_buf();
    let dest = parent.join(&req.new_name);
    protect.check(&dest)?;

    let old_name = file_name_string(&src);
    if old_name == req.new_name {
        return Ok(FsRenameResult { new_path: display_path(&dest) });
    }

    let result = if old_name.to_lowercase() == req.new_name.to_lowercase() {
        rename_case_only(&src, &dest, &parent, &req.new_name)
    } else {
        platform::rename_no_replace(&src, &dest).map_err(|e| match e.kind() {
            io::ErrorKind::AlreadyExists => exists_message(&req.new_name),
            _ => missing_or(e, "rename"),
        })
    };
    tracing::info!(
        op = "fs.rename",
        from = %display_path(&src),
        to = %display_path(&dest),
        outcome = if result.is_ok() { "ok" } else { "error" },
        error = result.as_ref().err().map(String::as_str).unwrap_or(""),
        "fs mutation"
    );
    result.map(|()| FsRenameResult { new_path: display_path(&dest) })
}

fn rename_case_only(src: &Path, dest: &Path, parent: &Path, new_name: &str) -> Result<(), String> {
    // On a case-sensitive filesystem a differently-cased name can be a
    // different, existing entry; only an exact match tells.
    let taken = std::fs::read_dir(parent)
        .map_err(|e| missing_or(e, "rename"))?
        .flatten()
        .any(|e| e.file_name() == std::ffi::OsStr::new(new_name));
    if taken {
        return Err(exists_message(new_name));
    }
    let tmp = parent.join(format!(".agentmux-rename-{}", uuid::Uuid::new_v4().simple()));
    platform::rename_no_replace(src, &tmp).map_err(|e| missing_or(e, "rename"))?;
    if let Err(e) = platform::rename_no_replace(&tmp, dest) {
        // Put it back under its old name rather than strand it under the
        // temporary one.
        let _ = platform::rename_no_replace(&tmp, src);
        return Err(match e.kind() {
            io::ErrorKind::AlreadyExists => exists_message(new_name),
            _ => missing_or(e, "rename"),
        });
    }
    Ok(())
}

/// `fs.create`: a new empty file or folder. Creation itself is the
/// existence check (`create_new` / `create_dir` fail if the name is taken),
/// so nothing can be overwritten.
pub fn create(req: &FsCreateReq) -> Result<FsCreateResult, String> {
    let protect = ProtectedPaths::current();
    validate_new_name(&req.name)?;
    let parent = resolve_request_path(&req.parent)?.canonicalize().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => "That folder doesn't exist anymore.".to_string(),
        _ => format!("Couldn't open that folder: {e}"),
    })?;
    if !parent.is_dir() {
        return Err("That isn't a folder.".to_string());
    }
    let target = parent.join(&req.name);
    protect.check(&target)?;

    let result = match req.kind {
        FsCreateKind::File => std::fs::OpenOptions::new().write(true).create_new(true).open(&target).map(|_| ()),
        FsCreateKind::Dir => std::fs::create_dir(&target),
    }
    .map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => exists_message(&req.name),
        io::ErrorKind::PermissionDenied => "You don't have permission to create something here.".to_string(),
        _ => format!("Couldn't create it: {e}"),
    });
    tracing::info!(
        op = "fs.create",
        kind = ?req.kind,
        path = %display_path(&target),
        outcome = if result.is_ok() { "ok" } else { "error" },
        error = result.as_ref().err().map(String::as_str).unwrap_or(""),
        "fs mutation"
    );
    result.map(|()| FsCreateResult { path: display_path(&target) })
}

/// `fs.delete`, one path: PERMANENT. A link is unlinked, never followed.
pub fn delete_permanently(raw: &str) -> Result<(), String> {
    let protect = ProtectedPaths::current();
    let target = resolve_entry_path(raw)?;
    std::fs::symlink_metadata(&target).map_err(|e| missing_or(e, "delete"))?;
    protect.check(&target)?;
    let result = platform::remove_entry_no_follow(&target).map_err(|e| missing_or(e, "delete"));
    tracing::info!(
        op = "fs.delete",
        path = %display_path(&target),
        outcome = if result.is_ok() { "ok" } else { "error" },
        error = result.as_ref().err().map(String::as_str).unwrap_or(""),
        "fs mutation"
    );
    result
}

/// `fs.trash`, one path. Must run on the trash thread
/// (`trash_worker::run`).
pub fn trash(raw: &str) -> Result<(), String> {
    let protect = ProtectedPaths::current();
    let target = resolve_entry_path(raw)?;
    std::fs::symlink_metadata(&target).map_err(|e| missing_or(e, "move"))?;
    protect.check(&target)?;
    let result = match platform::trash_unavailable_reason(&target) {
        Some(reason) => Err(reason.to_string()),
        None => trash_worker::trash_path(&target),
    };
    tracing::info!(
        op = "fs.trash",
        path = %display_path(&target),
        outcome = if result.is_ok() { "ok" } else { "error" },
        error = result.as_ref().err().map(String::as_str).unwrap_or(""),
        "fs mutation"
    );
    result
}

/// `fs.restore`: put each path back from the Trash. The paths are the
/// original locations. Must run on the trash thread (`trash_worker::run`).
/// One result per path, in order.
pub fn restore(raws: &[String]) -> Vec<Result<(), String>> {
    let protect = ProtectedPaths::current();
    // Validate every path first, then hand the valid ones to the trash
    // crate in one go, so the Trash is listed once per request.
    let checked: Vec<Result<PathBuf, String>> = raws
        .iter()
        .map(|raw| {
            let target = resolve_entry_path(raw)?;
            protect.check(&target)?;
            Ok(target)
        })
        .collect();
    let valid: Vec<PathBuf> = checked.iter().filter_map(|c| c.as_ref().ok().cloned()).collect();
    let mut restored = trash_worker::restore_paths(&valid).into_iter();
    checked
        .into_iter()
        .map(|c| {
            let target = c?;
            let result = restored.next().unwrap_or_else(|| Err("Couldn't put it back.".to_string()));
            tracing::info!(
                op = "fs.restore",
                path = %display_path(&target),
                outcome = if result.is_ok() { "ok" } else { "error" },
                error = result.as_ref().err().map(String::as_str).unwrap_or(""),
                "fs mutation"
            );
            result
        })
        .collect()
}

/// `fs.open` / `fs.reveal`: hand an existing path to the OS. Not limited to
/// home (unlike `openinshell`): opening and revealing change nothing.
pub fn open_or_reveal(raw: &str, reveal: bool) -> Result<(), String> {
    let path = resolve_request_path(raw)?;
    // A drive root has no parent to canonicalize; it is opened as it is.
    let target = if path.parent().is_none() { path } else { resolve_entry_path(raw)? };
    std::fs::symlink_metadata(&target).map_err(|e| missing_or(e, "open"))?;
    let cmd = if reveal { platform::reveal_command(&target) } else { platform::open_command(&target) };
    // `spawn_detached` applies the strict external-process policy (I7).
    let result = platform::spawn_detached(cmd).map_err(|e| format!("Couldn't open it: {e}"));
    tracing::info!(
        op = if reveal { "fs.reveal" } else { "fs.open" },
        path = %display_path(&target),
        outcome = if result.is_ok() { "ok" } else { "error" },
        "fs action"
    );
    result
}

#[cfg(test)]
mod tests;
