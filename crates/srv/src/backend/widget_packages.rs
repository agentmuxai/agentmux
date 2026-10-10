// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Widget packages: user widgets as folders with a `widget.json` manifest,
//! which run only once the user approves them
//! (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §5, §8).
//!
//! srv scans `~/.agentmux/widgets/` (shared by every channel), validates each
//! package's manifest, hashes every file, and keeps this instance's approval
//! record. A package's files are served (`/agentmux/widget-files/…`) only while
//! they still hash to what the user approved, checked per file on every read,
//! so an edit after approval can't run until it is approved again — whether
//! or not the folder watcher saw it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::wconfig::WidgetConfigType;

pub const MANIFEST_FILE: &str = "widget.json";
pub const MANIFEST_VERSION: u32 = 1;
pub const MAX_PACKAGE_BYTES: u64 = 50 * 1024 * 1024;
pub const MAX_PACKAGE_FILES: usize = 2000;
const APPROVALS_FILE: &str = "widget-approvals.json";

/// Every permission a manifest may ask for (spec §6.4). `net:<origin>` is
/// checked separately.
pub const PERMISSIONS: &[&str] = &["storage", "files", "clipboard:write", "panes", "agents:read", "agents:send"];

// ── The manifest (spec §5.3) ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum WidgetKind {
    Sandboxed,
    Trusted,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestPane {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub default_meta: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub singleton: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestContributes {
    #[serde(default)]
    pub panes: Vec<ManifestPane>,
    /// Reserved for W5; accepted and ignored.
    #[serde(default)]
    pub commands: Option<serde_json::Value>,
    #[serde(default, rename = "statusItems")]
    pub status_items: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub default_hue: Option<i64>,
    #[serde(default)]
    pub kind: Option<WidgetKind>,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub min_agent_mux: Option<String>,
    pub contributes: ManifestContributes,
}

impl Manifest {
    pub fn kind(&self) -> WidgetKind {
        self.kind.clone().unwrap_or(WidgetKind::Sandboxed)
    }

    fn default_entry(&self) -> String {
        self.entry.clone().unwrap_or_else(|| match self.kind() {
            WidgetKind::Sandboxed => "index.html".to_string(),
            WidgetKind::Trusted => "index.js".to_string(),
        })
    }
}

fn is_slug(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn valid_id(id: &str) -> bool {
    id.len() <= 64 && matches!(id.split_once('.'), Some((a, b)) if is_slug(a) && is_slug(b))
}

fn valid_semver(v: &str) -> bool {
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// A permission is a known name, or `net:<origin>`: `http(s)://host[:port]`,
/// where host may start with `*.` (subdomains), never a bare `*`.
pub fn valid_permission(p: &str) -> bool {
    if PERMISSIONS.contains(&p) {
        return true;
    }
    let Some(origin) = p.strip_prefix("net:") else {
        return false;
    };
    let Some(rest) = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://")) else {
        return false;
    };
    if rest.is_empty() || rest.contains('/') || rest.contains('?') || rest.contains('#') || rest.contains('@') {
        return false;
    }
    let host = rest.rsplit_once(':').map(|(h, port)| (h, port)).map_or(rest, |(h, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) && !port.is_empty() {
            h
        } else {
            "\u{0}"
        }
    });
    let bare = host.strip_prefix("*.").unwrap_or(host);
    !bare.is_empty() && !bare.contains('*') && bare.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'[' || b == b']' || b == b':')
}

/// A path inside a package: relative, `/`-separated, no `..`.
fn valid_rel_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && Path::new(p).components().all(|c| matches!(c, Component::Normal(_)))
}

/// Check a manifest; the first problem, in words the user can act on.
pub fn validate(m: &Manifest, folder: &str) -> Result<(), String> {
    if m.manifest_version != MANIFEST_VERSION {
        return Err(format!(
            "manifestVersion {} isn't one this AgentMux reads (it reads {MANIFEST_VERSION})",
            m.manifest_version
        ));
    }
    if !valid_id(&m.id) {
        return Err(format!("id {:?} must look like publisher.name (lowercase letters, digits, -)", m.id));
    }
    if m.id != folder {
        return Err(format!("id {:?} must match its folder name {:?}", m.id, folder));
    }
    if m.name.trim().is_empty() || m.name.chars().count() > 60 {
        return Err("name must be 1–60 characters".to_string());
    }
    if !valid_semver(&m.version) {
        return Err(format!("version {:?} isn't semver (like 1.2.0)", m.version));
    }
    if m.description.as_deref().is_some_and(|d| d.chars().count() > 300) {
        return Err("description is longer than 300 characters".to_string());
    }
    if let Some(h) = &m.homepage {
        if !h.starts_with("https://") {
            return Err("homepage must be an https:// link".to_string());
        }
    }
    if let Some(h) = m.default_hue {
        if !(0..=359).contains(&h) {
            return Err("defaultHue must be 0–359".to_string());
        }
    }
    for p in &m.permissions {
        if !valid_permission(p) {
            return Err(format!("unknown permission {p:?}"));
        }
    }
    if m.contributes.panes.is_empty() {
        return Err("contributes.panes must list at least one pane".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for pane in &m.contributes.panes {
        if !is_slug(&pane.name) {
            return Err(format!("pane name {:?} must be lowercase letters, digits and -", pane.name));
        }
        if !seen.insert(pane.name.as_str()) {
            return Err(format!("two panes are named {:?}", pane.name));
        }
        let entry = pane.entry.clone().unwrap_or_else(|| m.default_entry());
        if !valid_rel_path(&entry) {
            return Err(format!("entry {entry:?} must be a path inside the package"));
        }
        if let Some(meta) = &pane.default_meta {
            if let Some(k) = meta.keys().find(|k| !k.starts_with("widget:")) {
                return Err(format!("defaultMeta key {k:?} must start with \"widget:\""));
            }
        }
    }
    Ok(())
}

// ── Files and the content hash (spec §5.1, §8.2) ────────────────────────────

/// Every file in `dir` (relative path → SHA-256 hex), within the limits, with
/// no symlinks.
pub fn hash_files(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut total: u64 = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("can't read {}: {e}", d.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let ft = entry.file_type().map_err(|e| e.to_string())?;
            let path = entry.path();
            if ft.is_symlink() {
                return Err(format!("{} is a link; a package can't contain links", path.display()));
            }
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(dir)
                .map_err(|e| e.to_string())?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let bytes = std::fs::read(&path).map_err(|e| format!("can't read {rel}: {e}"))?;
            total += bytes.len() as u64;
            if total > MAX_PACKAGE_BYTES {
                return Err(format!("the package is over {} MB", MAX_PACKAGE_BYTES / (1024 * 1024)));
            }
            out.insert(rel, hex::encode(Sha256::digest(&bytes)));
            if out.len() > MAX_PACKAGE_FILES {
                return Err(format!("the package has over {MAX_PACKAGE_FILES} files"));
            }
        }
    }
    Ok(out)
}

/// The package's content hash: SHA-256 over each file's path and hash, in
/// path order. Any change to any file changes it.
pub fn package_hash(files: &BTreeMap<String, String>) -> String {
    let mut h = Sha256::new();
    for (path, file_hash) in files {
        h.update(path.as_bytes());
        h.update([0]);
        h.update(file_hash.as_bytes());
        h.update([b'\n']);
    }
    hex::encode(h.finalize())
}

// ── Approvals (spec §8.3) ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Approval {
    pub hash: String,
    pub kind: WidgetKind,
    pub permissions: Vec<String>,
    /// Per-file hashes at approval: what the files route checks every read
    /// against.
    pub files: BTreeMap<String, String>,
    pub approved_at: i64,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

pub type Approvals = BTreeMap<String, Approval>;

pub fn read_approvals(path: &Path) -> Approvals {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn write_approvals(path: &Path, approvals: &Approvals) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(approvals).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

// ── What the frontend sees (spec §8.1) ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum WidgetState {
    Approved,
    NeedsApproval,
    Changed,
    Invalid,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetPaneInfo {
    /// `ext:<id>/<name>` (or a v1 widget's own `ext:<name>`).
    pub view: String,
    pub name: String,
    pub label: String,
    pub icon: String,
    pub entry: String,
    pub singleton: bool,
    #[ts(type = "Record<string, unknown>")]
    pub default_meta: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetPackageInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    pub icon: String,
    #[ts(type = "number | null")]
    pub default_hue: Option<i64>,
    pub kind: WidgetKind,
    pub permissions: Vec<String>,
    /// Permissions the user granted, when approved (an update that asks for
    /// more is `changed` until approved again).
    pub granted: Vec<String>,
    pub state: WidgetState,
    pub error: Option<String>,
    /// The current content hash (empty when the files couldn't be read).
    pub hash: String,
    pub panes: Vec<WidgetPaneInfo>,
    /// Where its files are served, ending in `/`, while it is approved and
    /// enabled; relative to srv's web endpoint.
    pub files_url: Option<String>,
    /// A v1 `widgets.json` module entry, shown as an implied package.
    pub implied: bool,
    pub folder: String,
}

// ── The scan ────────────────────────────────────────────────────────────────

/// A package found on disk, before approval is applied.
#[derive(Debug, Clone)]
pub struct Found {
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Result<Manifest, String>,
    pub files: Result<BTreeMap<String, String>, String>,
    pub implied: bool,
}

/// Every folder in `widgets_dir` (a missing folder is none).
pub fn scan(widgets_dir: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(widgets_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let dir = entry.path();
        let folder = entry.file_name().to_string_lossy().into_owned();
        let manifest_path = dir.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            // A v1 widget's module folder (e.g. hello/ with only index.js) is
            // not a package; the v1 entry in widgets.json names it instead.
            continue;
        }
        let manifest = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("can't read widget.json: {e}"))
            .and_then(|s| serde_json::from_str::<Manifest>(&s).map_err(|e| format!("widget.json: {e}")))
            .and_then(|m| validate(&m, &folder).map(|()| m));
        out.push(Found { id: folder, files: hash_files(&dir), dir, manifest, implied: false });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// A v1 `widgets.json` entry with a `module` as an implied trusted package
/// (spec §5.5): its one module file is the package.
pub fn implied_from_v1(key: &str, entry: &WidgetConfigType, widgets_dir: &Path) -> Option<Found> {
    let module = entry.module.as_str();
    if module.is_empty() {
        return None;
    }
    let view = entry.block_def.meta.get("view").and_then(|v| v.as_str())?.to_string();
    let name = view.strip_prefix("ext:")?;
    if name.contains('/') {
        return None;
    }
    let raw = Path::new(module);
    let path = if raw.is_absolute() { raw.to_path_buf() } else { widgets_dir.join(raw) };
    let dir = path.parent()?.to_path_buf();
    let file = path.file_name()?.to_string_lossy().into_owned();
    let slug: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let id = format!("local.{slug}");
    let files = std::fs::read(&path)
        .map_err(|e| format!("can't read {}: {e}", path.display()))
        .map(|bytes| BTreeMap::from([(file.clone(), hex::encode(Sha256::digest(&bytes)))]));
    let manifest = Manifest {
        manifest_version: MANIFEST_VERSION,
        id: id.clone(),
        name: if entry.label.is_empty() { name.to_string() } else { entry.label.clone() },
        version: "0.0.0".to_string(),
        description: Some(format!("From widgets.json ({key})")),
        author: None,
        homepage: None,
        icon: Some(entry.icon.clone()).filter(|i| !i.is_empty()),
        default_hue: None,
        kind: Some(WidgetKind::Trusted),
        entry: Some(file),
        permissions: vec![],
        min_agent_mux: None,
        contributes: ManifestContributes {
            panes: vec![ManifestPane { name: slug, label: None, icon: None, entry: None, default_meta: None, singleton: false }],
            commands: None,
            status_items: None,
        },
    };
    let _ = view;
    Some(Found { id, dir, manifest: Ok(manifest), files, implied: true })
}

/// The view a pane opens as: `ext:<id>/<name>`, or a v1 widget's own name.
fn pane_view(found: &Found, m: &Manifest, pane: &ManifestPane, v1_view: Option<&str>) -> String {
    match (found.implied, v1_view) {
        (true, Some(v)) => v.to_string(),
        _ => format!("ext:{}/{}", m.id, pane.name),
    }
}

/// The files URL's unguessable part: HMAC(instance secret, id, hash), so a web
/// page outside AgentMux can't name a package's files (they aren't secret,
/// but nothing else needs them).
pub fn files_key(secret: &str, id: &str, hash: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(b"agentmux-widget-files\0");
    mac.update(id.as_bytes());
    mac.update(b"\0");
    mac.update(hash.as_bytes());
    hex::encode(&mac.finalize().into_bytes()[..16])
}

/// Combine what's on disk with this instance's approvals.
pub fn describe(found: &Found, approvals: &Approvals, secret: &str, v1_view: Option<&str>) -> WidgetPackageInfo {
    let folder = found.dir.display().to_string();
    let invalid = |error: String| WidgetPackageInfo {
        id: found.id.clone(),
        name: found.id.clone(),
        version: String::new(),
        description: None,
        author: None,
        homepage: None,
        icon: "puzzle-piece".to_string(),
        default_hue: None,
        kind: WidgetKind::Sandboxed,
        permissions: vec![],
        granted: vec![],
        state: WidgetState::Invalid,
        error: Some(error),
        hash: String::new(),
        panes: vec![],
        files_url: None,
        implied: found.implied,
        folder: folder.clone(),
    };
    let m = match &found.manifest {
        Ok(m) => m,
        Err(e) => return invalid(e.clone()),
    };
    let files = match &found.files {
        Ok(f) => f,
        Err(e) => return invalid(e.clone()),
    };
    let entry_missing = m.contributes.panes.iter().find_map(|p| {
        let entry = p.entry.clone().unwrap_or_else(|| m.default_entry());
        (!files.contains_key(&entry)).then_some(entry)
    });
    if let Some(entry) = entry_missing {
        return invalid(format!("its entry {entry:?} isn't in the package"));
    }
    let hash = package_hash(files);
    let kind = m.kind();
    let approval = approvals.get(&m.id);
    let state = match approval {
        None => WidgetState::NeedsApproval,
        Some(a) if a.hash != hash || a.kind != kind => WidgetState::Changed,
        Some(a) if !a.enabled => WidgetState::Disabled,
        Some(_) => WidgetState::Approved,
    };
    let icon = m.icon.clone().unwrap_or_else(|| "puzzle-piece".to_string());
    let panes = m
        .contributes
        .panes
        .iter()
        .map(|p| WidgetPaneInfo {
            view: pane_view(found, m, p, v1_view),
            name: p.name.clone(),
            label: p.label.clone().unwrap_or_else(|| m.name.clone()),
            icon: p.icon.clone().unwrap_or_else(|| icon.clone()),
            entry: p.entry.clone().unwrap_or_else(|| m.default_entry()),
            singleton: p.singleton,
            default_meta: p.default_meta.clone().unwrap_or_default(),
        })
        .collect();
    let files_url = (state == WidgetState::Approved)
        .then(|| format!("/agentmux/widget-files/{}/{}/{}/", m.id, hash, files_key(secret, &m.id, &hash)));
    WidgetPackageInfo {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.clone(),
        description: m.description.clone(),
        author: m.author.clone(),
        homepage: m.homepage.clone(),
        icon,
        default_hue: m.default_hue,
        kind: kind.clone(),
        permissions: if kind == WidgetKind::Trusted { vec![] } else { m.permissions.clone() },
        granted: approval.filter(|a| a.hash == hash).map(|a| a.permissions.clone()).unwrap_or_default(),
        state,
        error: None,
        hash,
        panes,
        files_url,
        implied: found.implied,
        folder,
    }
}

/// The widget-bar entries an approved, enabled package adds, keyed
/// `ext@<id>/<name>` (spec §5.4). v1 widgets already have theirs.
pub fn widget_entries(packages: &[WidgetPackageInfo]) -> HashMap<String, WidgetConfigType> {
    let mut out = HashMap::new();
    for p in packages.iter().filter(|p| p.state == WidgetState::Approved && !p.implied) {
        for pane in &p.panes {
            let mut entry = WidgetConfigType {
                label: pane.label.clone(),
                icon: pane.icon.clone(),
                description: p.description.clone().unwrap_or_default(),
                ..Default::default()
            };
            entry.block_def.meta.insert("view".to_string(), serde_json::Value::String(pane.view.clone()));
            for (k, v) in &pane.default_meta {
                entry.block_def.meta.insert(k.clone(), v.clone());
            }
            out.insert(format!("ext@{}/{}", p.id, pane.name), entry);
        }
    }
    out
}

// ── The service ─────────────────────────────────────────────────────────────

/// srv's view of the widgets folder and this instance's approvals.
pub struct WidgetPackages {
    pub widgets_dir: PathBuf,
    approvals_path: PathBuf,
    secret: String,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    found: Vec<Found>,
    v1_views: HashMap<String, String>,
    packages: Vec<WidgetPackageInfo>,
    approvals: Approvals,
}

static SERVICE: OnceLock<Arc<WidgetPackages>> = OnceLock::new();

pub fn service() -> Option<&'static Arc<WidgetPackages>> {
    SERVICE.get()
}

/// `~/.agentmux/widgets`, shared by every channel (spec §3, decision 5).
pub fn widgets_dir() -> PathBuf {
    crate::srv_info::agentmux_home()
        .unwrap_or_else(crate::backend::base::get_mux_data_dir)
        .join("widgets")
}

impl WidgetPackages {
    pub fn new(widgets_dir: PathBuf, data_dir: &Path, secret: String) -> Self {
        let approvals_path = data_dir.join(APPROVALS_FILE);
        let approvals = read_approvals(&approvals_path);
        Self { widgets_dir, approvals_path, secret, inner: Mutex::new(Inner { approvals, ..Default::default() }) }
    }

    pub fn install_global(self) -> &'static Arc<WidgetPackages> {
        SERVICE.get_or_init(|| Arc::new(self))
    }

    /// Scan again, with the v1 entries currently in the user's widgets.json.
    pub fn rescan(&self, v1_entries: &HashMap<String, WidgetConfigType>) -> Vec<WidgetPackageInfo> {
        let mut found = scan(&self.widgets_dir);
        let mut v1_views = HashMap::new();
        let mut keys: Vec<&String> = v1_entries.keys().collect();
        keys.sort();
        for key in keys {
            if let Some(f) = implied_from_v1(key, &v1_entries[key], &self.widgets_dir) {
                if found.iter().any(|x| x.id == f.id) {
                    continue;
                }
                if let Some(v) = v1_entries[key].block_def.meta.get("view").and_then(|v| v.as_str()) {
                    v1_views.insert(f.id.clone(), v.to_string());
                }
                found.push(f);
            }
        }
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let packages = found
            .iter()
            .map(|f| describe(f, &inner.approvals, &self.secret, v1_views.get(&f.id).map(String::as_str)))
            .collect::<Vec<_>>();
        inner.found = found;
        inner.v1_views = v1_views;
        inner.packages = packages.clone();
        packages
    }

    pub fn list(&self) -> Vec<WidgetPackageInfo> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).packages.clone()
    }

    /// Record the user's approval of exactly `hash` (the host-only route
    /// calls this). Fails if the package changed since the prompt was shown.
    pub fn approve(&self, id: &str, hash: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let found = inner.found.iter().find(|f| f.id == id).ok_or("no such widget")?;
        let m = found.manifest.as_ref().map_err(|e| e.clone())?;
        let files = found.files.as_ref().map_err(|e| e.clone())?;
        if package_hash(files) != hash {
            return Err("the widget changed since you were asked; review it again".to_string());
        }
        let approval = Approval {
            hash: hash.to_string(),
            kind: m.kind(),
            permissions: if m.kind() == WidgetKind::Trusted { vec![] } else { m.permissions.clone() },
            files: files.clone(),
            approved_at: chrono::Utc::now().timestamp(),
            enabled: true,
        };
        inner.approvals.insert(id.to_string(), approval);
        write_approvals(&self.approvals_path, &inner.approvals)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let a = inner.approvals.get_mut(id).ok_or("that widget isn't approved")?;
        a.enabled = enabled;
        write_approvals(&self.approvals_path, &inner.approvals)
    }

    /// Forget a package's approval (uninstall does this; so does a v1 entry
    /// leaving widgets.json, at the next rescan's caller's choice).
    pub fn forget(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.approvals.remove(id);
        write_approvals(&self.approvals_path, &inner.approvals)
    }

    pub fn package_dir(&self, id: &str) -> Option<(PathBuf, bool)> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.found.iter().find(|f| f.id == id).map(|f| (f.dir.clone(), f.implied))
    }

    /// The bytes of `path` in package `id`, if `hash` and `key` name its
    /// approved version and the file on disk still matches that approval
    /// (hashed now, not trusted from the last scan).
    pub fn read_file(&self, id: &str, hash: &str, key: &str, path: &str) -> Result<Vec<u8>, FileError> {
        if !valid_rel_path(path) {
            return Err(FileError::NotFound);
        }
        if !agentmux_common::secret_eq::secret_eq(key.as_bytes(), files_key(&self.secret, id, hash).as_bytes()) {
            return Err(FileError::NotFound);
        }
        let (dir, file_hash) = {
            let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let approval = inner.approvals.get(id).filter(|a| a.hash == hash && a.enabled).ok_or(FileError::NotFound)?;
            let file_hash = approval.files.get(path).cloned().ok_or(FileError::NotFound)?;
            let dir = inner.found.iter().find(|f| f.id == id).map(|f| f.dir.clone()).ok_or(FileError::NotFound)?;
            (dir, file_hash)
        };
        let full = dir.join(path);
        // No links anywhere on the way: the approved bytes are the package's
        // own, not wherever a link points now.
        let mut p = dir.clone();
        for c in Path::new(path).components() {
            p.push(c);
            if std::fs::symlink_metadata(&p).map(|m| m.file_type().is_symlink()).unwrap_or(true) {
                return Err(FileError::NotFound);
            }
        }
        let bytes = std::fs::read(&full).map_err(|_| FileError::NotFound)?;
        if hex::encode(Sha256::digest(&bytes)) != file_hash {
            return Err(FileError::Changed);
        }
        Ok(bytes)
    }
}

// ── Keeping everyone current ────────────────────────────────────────────────

/// Scan again, merge the widget bar again, and tell every UI.
pub fn refresh(
    config_watcher: &Arc<super::wconfig::ConfigState>,
    event_bus: &Arc<super::eventbus::EventBus>,
    broker: &super::mps::Broker,
) -> Vec<WidgetPackageInfo> {
    let Some(svc) = service() else {
        return vec![];
    };
    let packages = svc.rescan(&super::user_widgets::last_v1_entries());
    super::user_widgets::recompute(config_watcher);
    super::config_watcher_fs::broadcast_full_config(config_watcher, event_bus);
    broker.publish(super::mps::MuxEvent {
        event: super::mps::EVENT_WIDGET_PACKAGES.to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: Some(serde_json::json!({ "packages": packages })),
    });
    packages
}

/// Start the service: scan, merge, and watch the widgets folder.
pub fn start(
    data_dir: &Path,
    secret: String,
    pool: Arc<super::fs_watch::FsWatchPool>,
    config_watcher: Arc<super::wconfig::ConfigState>,
    event_bus: Arc<super::eventbus::EventBus>,
    broker: Arc<super::mps::Broker>,
) {
    let data_dir = if data_dir.as_os_str().is_empty() { super::base::get_mux_data_dir() } else { data_dir.to_path_buf() };
    let data_dir = data_dir.as_path();
    let dir = widgets_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(dir = %dir.display(), error = %e, "can't create the widgets folder; widget packages off");
        return;
    }
    WidgetPackages::new(dir.clone(), data_dir, secret).install_global();
    let packages = refresh(&config_watcher, &event_bus, &broker);
    tracing::info!(dir = %dir.display(), count = packages.len(), "widget packages scanned");

    let mut events = pool.events();
    let _sub = pool.subscribe_dir_recursive(&dir);
    let watched = dir.canonicalize().unwrap_or(dir);
    tokio::spawn(async move {
        let _sub = _sub;
        loop {
            match events.recv().await {
                Ok(ev) if ev.path.starts_with(&watched) => {}
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            }
            // Coalesce a burst (an editor's save, a copy of a whole folder).
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            while events.try_recv().is_ok() {}
            refresh(&config_watcher, &event_bus, &broker);
        }
    });
}

#[derive(Debug, PartialEq)]
pub enum FileError {
    NotFound,
    /// The file no longer matches what the user approved.
    Changed,
}

/// The content type a widget file is served with.
pub fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// The policy every widget file is served with (spec §6.1).
pub const WIDGET_CSP: &str = "default-src 'none'; script-src 'self' 'unsafe-inline' 'unsafe-eval' blob:; \
style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; media-src 'self' blob:; \
connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'";

// ── Install and uninstall (spec §8.4) ───────────────────────────────────────

/// Copy the package at `source` (a folder, a `widget.json` in one, or a
/// `.zip`) into the widgets folder; returns its id. Validates first; refuses
/// to replace an existing package unless `replace`.
pub fn install(widgets_dir: &Path, source: &Path, replace: bool) -> Result<String, String> {
    let is_zip = source.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    let staging = tempfile::tempdir_in(widgets_dir.parent().unwrap_or(widgets_dir))
        .or_else(|_| tempfile::tempdir())
        .map_err(|e| format!("can't make a staging folder: {e}"))?;
    let stage = staging.path().join("pkg");
    if is_zip {
        unzip(source, &stage)?;
    } else {
        let folder = if source.file_name().is_some_and(|n| n == MANIFEST_FILE) { source.parent().unwrap_or(source) } else { source };
        copy_tree(folder, &stage)?;
    }
    // A zip may hold the package at its root or in one top-level folder.
    let root = if stage.join(MANIFEST_FILE).exists() {
        stage.clone()
    } else {
        let mut dirs = std::fs::read_dir(&stage).map_err(|e| e.to_string())?.flatten().filter(|e| e.path().is_dir());
        match (dirs.next(), dirs.next()) {
            (Some(d), None) if d.path().join(MANIFEST_FILE).exists() => d.path(),
            _ => return Err("no widget.json at the package's top level".to_string()),
        }
    };
    let manifest: Manifest = serde_json::from_str(
        &std::fs::read_to_string(root.join(MANIFEST_FILE)).map_err(|e| format!("can't read widget.json: {e}"))?,
    )
    .map_err(|e| format!("widget.json: {e}"))?;
    validate(&manifest, &manifest.id)?;
    hash_files(&root)?;
    std::fs::create_dir_all(widgets_dir).map_err(|e| e.to_string())?;
    let dest = widgets_dir.join(&manifest.id);
    if dest.exists() {
        if !replace {
            return Err(format!("{} is already installed", manifest.id));
        }
        std::fs::remove_dir_all(&dest).map_err(|e| format!("can't replace the installed version: {e}"))?;
    }
    copy_tree(&root, &dest)?;
    Ok(manifest.id)
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("can't read {}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ft = entry.file_type().map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if ft.is_symlink() {
            return Err(format!("{} is a link; a package can't contain links", entry.path().display()));
        } else if ft.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn unzip(zip_path: &Path, to: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("can't open {}: {e}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("not a zip file: {e}"))?;
    if archive.len() > MAX_PACKAGE_FILES {
        return Err(format!("the package has over {MAX_PACKAGE_FILES} files"));
    }
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name refuses `..` and absolute names.
        let rel = f.enclosed_name().ok_or_else(|| format!("unsafe path in the zip: {}", f.name()))?;
        if f.is_symlink() {
            return Err(format!("{} is a link; a package can't contain links", f.name()));
        }
        let out = to.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        total += f.size();
        if total > MAX_PACKAGE_BYTES {
            return Err(format!("the package is over {} MB", MAX_PACKAGE_BYTES / (1024 * 1024)));
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut (&mut f).take(MAX_PACKAGE_BYTES + 1), &mut w).map_err(|e| e.to_string())?;
    }
    Ok(())
}

use std::io::Read as _;

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str) -> serde_json::Value {
        serde_json::json!({
            "manifestVersion": 1,
            "id": id,
            "name": "Test",
            "version": "1.0.0",
            "permissions": ["storage", "net:https://api.github.com"],
            "contributes": { "panes": [{ "name": "main" }] }
        })
    }

    fn package(root: &Path, id: &str) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), manifest(id).to_string()).unwrap();
        std::fs::write(dir.join("index.html"), "<p>hi</p>").unwrap();
        dir
    }

    fn service(root: &Path) -> WidgetPackages {
        WidgetPackages::new(root.join("widgets"), &root.join("data"), "secret".into())
    }

    #[test]
    fn ids_versions_and_permissions_are_checked() {
        assert!(valid_id("acme.pr-dashboard"));
        assert!(!valid_id("Acme.x") && !valid_id("acme") && !valid_id("a.b.c") && !valid_id("acme."));
        assert!(valid_semver("1.2.0") && valid_semver("1.2.0-beta.1") && !valid_semver("1.2"));
        assert!(valid_permission("storage") && valid_permission("net:https://api.github.com"));
        assert!(valid_permission("net:https://*.example.com") && valid_permission("net:http://127.0.0.1:8188"));
        assert!(!valid_permission("net:https://*") && !valid_permission("net:https://a.com/path"));
        assert!(!valid_permission("net:ftp://a.com") && !valid_permission("everything"));
        assert!(!valid_permission("net:https://user@a.com"));
    }

    #[test]
    fn a_manifest_must_match_its_folder_and_keep_meta_namespaced() {
        let mut m: Manifest = serde_json::from_value(manifest("acme.test")).unwrap();
        assert!(validate(&m, "acme.test").is_ok());
        assert!(validate(&m, "acme.other").unwrap_err().contains("folder"));
        m.contributes.panes[0].default_meta = Some(serde_json::Map::from_iter([("view".into(), "term".into())]));
        assert!(validate(&m, "acme.test").unwrap_err().contains("widget:"));
        m.contributes.panes[0].default_meta = None;
        m.contributes.panes[0].entry = Some("../escape.html".into());
        assert!(validate(&m, "acme.test").unwrap_err().contains("inside the package"));
    }

    #[test]
    fn a_new_package_needs_approval_and_approval_names_its_exact_files() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path());
        package(&svc.widgets_dir, "acme.test");
        let p = &svc.rescan(&HashMap::new())[0];
        assert_eq!(p.state, WidgetState::NeedsApproval);
        assert_eq!(p.files_url, None);
        assert_eq!(p.panes[0].view, "ext:acme.test/main");

        svc.approve("acme.test", &p.hash).unwrap();
        let p = svc.rescan(&HashMap::new())[0].clone();
        assert_eq!(p.state, WidgetState::Approved);
        assert_eq!(p.granted, vec!["storage".to_string(), "net:https://api.github.com".to_string()]);
        let url = p.files_url.clone().unwrap();
        let key = url.trim_end_matches('/').rsplit('/').next().unwrap();
        assert_eq!(svc.read_file("acme.test", &p.hash, key, "index.html").unwrap(), b"<p>hi</p>");

        // An approval of a stale hash is refused.
        assert!(svc.approve("acme.test", "0000").is_err());
    }

    #[test]
    fn an_edited_file_is_never_served_and_the_package_asks_again() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path());
        let dir = package(&svc.widgets_dir, "acme.test");
        let hash = svc.rescan(&HashMap::new())[0].hash.clone();
        svc.approve("acme.test", &hash).unwrap();
        let key = files_key("secret", "acme.test", &hash);

        // Edited after approval, before any rescan: the read itself catches it.
        std::fs::write(dir.join("index.html"), "<script>evil()</script>").unwrap();
        assert_eq!(svc.read_file("acme.test", &hash, &key, "index.html"), Err(FileError::Changed));
        // A file added after approval isn't served either.
        std::fs::write(dir.join("extra.js"), "x").unwrap();
        assert_eq!(svc.read_file("acme.test", &hash, &key, "extra.js"), Err(FileError::NotFound));
        // And the rescan marks it changed, with no files URL.
        let p = svc.rescan(&HashMap::new())[0].clone();
        assert_eq!(p.state, WidgetState::Changed);
        assert_eq!(p.files_url, None);
    }

    #[test]
    fn files_need_the_right_key_and_stay_inside_the_package() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path());
        package(&svc.widgets_dir, "acme.test");
        std::fs::write(svc.widgets_dir.join("secret.txt"), "nope").unwrap();
        let hash = svc.rescan(&HashMap::new())[0].hash.clone();
        svc.approve("acme.test", &hash).unwrap();
        let key = files_key("secret", "acme.test", &hash);
        assert_eq!(svc.read_file("acme.test", &hash, "guess", "index.html"), Err(FileError::NotFound));
        assert_eq!(svc.read_file("acme.test", &hash, &key, "../secret.txt"), Err(FileError::NotFound));
        assert_eq!(svc.read_file("acme.test", &hash, &key, "/etc/passwd"), Err(FileError::NotFound));
    }

    #[test]
    fn disabling_stops_serving_and_removes_the_widget_bar_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path());
        package(&svc.widgets_dir, "acme.test");
        let hash = svc.rescan(&HashMap::new())[0].hash.clone();
        svc.approve("acme.test", &hash).unwrap();
        let packages = svc.rescan(&HashMap::new());
        let entries = widget_entries(&packages);
        assert_eq!(entries["ext@acme.test/main"].block_def.meta["view"], "ext:acme.test/main");

        svc.set_enabled("acme.test", false).unwrap();
        let packages = svc.rescan(&HashMap::new());
        assert_eq!(packages[0].state, WidgetState::Disabled);
        assert!(widget_entries(&packages).is_empty());
        let key = files_key("secret", "acme.test", &hash);
        assert_eq!(svc.read_file("acme.test", &hash, &key, "index.html"), Err(FileError::NotFound));
    }

    #[test]
    fn approvals_are_kept_per_instance_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path());
        package(&svc.widgets_dir, "acme.test");
        let hash = svc.rescan(&HashMap::new())[0].hash.clone();
        svc.approve("acme.test", &hash).unwrap();
        // Same widgets folder, another instance's data dir: asks again.
        let other = WidgetPackages::new(tmp.path().join("widgets"), &tmp.path().join("other-data"), "s2".into());
        assert_eq!(other.rescan(&HashMap::new())[0].state, WidgetState::NeedsApproval);
        // The same instance after a restart remembers.
        let again = service(tmp.path());
        assert_eq!(again.rescan(&HashMap::new())[0].state, WidgetState::Approved);
    }

    #[test]
    fn a_v1_module_entry_is_an_implied_trusted_package() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path());
        let hello = svc.widgets_dir.join("hello");
        std::fs::create_dir_all(&hello).unwrap();
        std::fs::write(hello.join("index.js"), "export default {}").unwrap();
        let mut entry = WidgetConfigType { label: "Hello".into(), module: "hello/index.js".into(), ..Default::default() };
        entry.block_def.meta.insert("view".into(), "ext:hello".into());
        let v1 = HashMap::from([("ext@hello".to_string(), entry)]);
        let p = svc.rescan(&v1)[0].clone();
        assert_eq!(p.id, "local.hello");
        assert_eq!(p.kind, WidgetKind::Trusted);
        assert!(p.implied);
        assert_eq!(p.state, WidgetState::NeedsApproval);
        assert_eq!(p.panes[0].view, "ext:hello");
        assert_eq!(p.panes[0].entry, "index.js");
        // A v1 widget keeps its own widget-bar entry; none is added.
        svc.approve("local.hello", &p.hash).unwrap();
        assert!(widget_entries(&svc.rescan(&v1)).is_empty());
    }

    #[test]
    fn install_copies_a_folder_or_a_zip_and_refuses_a_bad_one() {
        let tmp = tempfile::tempdir().unwrap();
        let widgets = tmp.path().join("widgets");
        let src = package(&tmp.path().join("src"), "acme.test");
        assert_eq!(install(&widgets, &src, false).unwrap(), "acme.test");
        assert!(widgets.join("acme.test/index.html").exists());
        assert!(install(&widgets, &src.join(MANIFEST_FILE), false).unwrap_err().contains("already installed"));
        assert_eq!(install(&widgets, &src.join(MANIFEST_FILE), true).unwrap(), "acme.test");

        // A zip with the package in one top-level folder.
        let zip_path = tmp.path().join("pkg.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("acme.zipped/widget.json", opts).unwrap();
            std::io::Write::write_all(&mut z, manifest("acme.zipped").to_string().as_bytes()).unwrap();
            z.start_file("acme.zipped/index.html", opts).unwrap();
            std::io::Write::write_all(&mut z, b"<p>z</p>").unwrap();
            z.finish().unwrap();
        }
        assert_eq!(install(&widgets, &zip_path, false).unwrap(), "acme.zipped");

        let bad = tmp.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join(MANIFEST_FILE), "{\"manifestVersion\": 9}").unwrap();
        assert!(install(&widgets, &bad, false).is_err());
    }
}
