// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The runtime layer of the provider registry: custom harnesses registered by
//! a manifest (`harness_manifest.rs`), asked after the built-ins.
//!
//! Every caller of the registry takes a `&'static ProviderConfig`, so a custom
//! harness is turned into one by leaking its strings once. The leak is keyed by
//! the manifest's content hash, so reading the same file again leaks nothing,
//! and two caps bound what a machine can ever leak: live harnesses
//! ([`MAX_LIVE`]) and distinct manifests seen since start ([`MAX_LEAKED`]).
//! SPEC_BYO_HARNESS_CUSTOM_ACP_MANIFEST_2026_11_12.md §4.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

use sha2::{Digest, Sha256};

use super::harness_manifest::{Auth, HarnessManifest, Install, ManifestError};
use super::providers::{ControllerType, ProviderConfig, Tier};

/// Custom harnesses registered at once on one machine.
pub const MAX_LIVE: usize = 32;
/// Distinct manifests that may be turned into a config in one process: a
/// manifest edited over and over cannot grow the leak without bound.
pub const MAX_LEAKED: usize = 128;
/// A harness folder larger than this is refused (it is a manifest and an icon).
pub const MAX_FOLDER_BYTES: u64 = 1024 * 1024;
/// An icon image larger than this is refused.
pub const MAX_ICON_BYTES: u64 = 64 * 1024;
/// The manifest's file name inside its folder.
pub const MANIFEST_FILE: &str = "harness.json";

/// Why a registration was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// A manifest rule is broken.
    Manifest(ManifestError),
    /// [`MAX_LIVE`] harnesses are registered already.
    TooManyLive,
    /// [`MAX_LEAKED`] distinct manifests were read since start.
    TooManyRevisions,
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::Manifest(e) => write!(f, "{e}"),
            RegisterError::TooManyLive => write!(f, "at most {MAX_LIVE} custom harnesses; remove one first"),
            RegisterError::TooManyRevisions => {
                write!(f, "too many different manifests since AgentMux started; restart AgentMux, then register it")
            }
        }
    }
}

impl std::error::Error for RegisterError {}

impl From<ManifestError> for RegisterError {
    fn from(e: ManifestError) -> Self {
        RegisterError::Manifest(e)
    }
}

/// One folder's outcome when a directory of harnesses is read.
#[derive(Debug, Default)]
pub struct LoadReport {
    /// Ids now registered from this directory.
    pub loaded: Vec<String>,
    /// Folders refused, with the reason.
    pub refused: Vec<(String, String)>,
}

#[derive(Default)]
struct Entry {
    hash: String,
    config: Option<&'static ProviderConfig>,
}

/// The harnesses registered and every config ever built, by content hash.
pub struct CustomLayer {
    max_live: usize,
    max_leaked: usize,
    live: HashMap<String, Entry>,
    leaked: HashMap<String, &'static ProviderConfig>,
}

impl CustomLayer {
    pub fn new(max_live: usize, max_leaked: usize) -> Self {
        CustomLayer { max_live, max_leaked, live: HashMap::new(), leaked: HashMap::new() }
    }

    /// Register a manifest, replacing the harness of the same id. Returns the
    /// config and whether the manifest's content changed (or is new).
    pub fn register(&mut self, manifest: &HarnessManifest) -> Result<(&'static ProviderConfig, bool), RegisterError> {
        manifest.validate()?;
        let hash = manifest_hash(manifest);
        let replacing = self.live.contains_key(&manifest.id);
        if !replacing && self.live.len() >= self.max_live {
            return Err(RegisterError::TooManyLive);
        }
        let config = match self.leaked.get(&hash) {
            Some(config) => *config,
            None => {
                if self.leaked.len() >= self.max_leaked {
                    return Err(RegisterError::TooManyRevisions);
                }
                let config = leak_config(manifest);
                self.leaked.insert(hash.clone(), config);
                config
            }
        };
        let changed = self.live.get(&manifest.id).is_none_or(|e| e.hash != hash);
        self.live.insert(manifest.id.clone(), Entry { hash, config: Some(config) });
        Ok((config, changed))
    }

    /// Take a harness out. The leaked config stays (it is never freed), but
    /// nothing can look it up any more.
    pub fn remove(&mut self, id: &str) -> bool {
        self.live.remove(id).is_some()
    }

    pub fn get(&self, id: &str) -> Option<&'static ProviderConfig> {
        self.live.get(id).and_then(|e| e.config)
    }

    /// Every registered harness, by id.
    pub fn all(&self) -> Vec<&'static ProviderConfig> {
        let mut all: Vec<_> = self.live.values().filter_map(|e| e.config).collect();
        all.sort_by_key(|c| c.id);
        all
    }

    /// The content hash a registered harness was built from.
    pub fn hash_of(&self, id: &str) -> Option<&str> {
        self.live.get(id).map(|e| e.hash.as_str())
    }

    /// Read every harness folder under `dir`, replacing what the layer holds
    /// for those ids. A folder that is refused is reported and the rest load.
    pub fn load_dir(&mut self, dir: &Path) -> LoadReport {
        let mut report = LoadReport::default();
        let Ok(entries) = std::fs::read_dir(dir) else { return report };
        let mut folders: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
        folders.sort_by_key(|e| e.file_name());
        for folder in folders {
            let name = folder.file_name().to_string_lossy().into_owned();
            match read_folder(&folder.path(), &name).and_then(|m| self.register(&m).map_err(|e| e.to_string())) {
                Ok((config, _)) => report.loaded.push(config.id.to_string()),
                Err(reason) => report.refused.push((name, reason)),
            }
        }
        report
    }
}

/// A folder's manifest, after the folder rules: named as its id, under
/// [`MAX_FOLDER_BYTES`], an icon file under [`MAX_ICON_BYTES`].
fn read_folder(path: &Path, folder_name: &str) -> Result<HarnessManifest, String> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(path).map_err(|e| format!("cannot read the folder: {e}"))?.flatten() {
        let meta = entry.metadata().map_err(|e| format!("cannot read the folder: {e}"))?;
        if meta.is_file() {
            total += meta.len();
        }
    }
    if total > MAX_FOLDER_BYTES {
        return Err(format!("the folder is larger than {MAX_FOLDER_BYTES} bytes"));
    }
    let text = std::fs::read_to_string(path.join(MANIFEST_FILE)).map_err(|e| format!("cannot read {MANIFEST_FILE}: {e}"))?;
    let manifest = HarnessManifest::parse(&text).map_err(|e| e.to_string())?;
    if manifest.id != folder_name {
        return Err(format!("the folder is named {folder_name} but the id is {}", manifest.id));
    }
    if let Some(icon) = manifest.icon.as_deref().filter(|i| i.contains('.')) {
        let size = std::fs::metadata(path.join(icon)).map_err(|e| format!("icon: cannot read {icon}: {e}"))?.len();
        if size > MAX_ICON_BYTES {
            return Err(format!("icon: {icon} is larger than {MAX_ICON_BYTES} bytes"));
        }
    }
    Ok(manifest)
}

/// SHA-256 of the manifest's canonical JSON (a map is ordered, so equal
/// content hashes equal).
pub fn manifest_hash(manifest: &HarnessManifest) -> String {
    let canonical = serde_json::to_string(manifest).unwrap_or_default();
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn leak_strs(items: &[String]) -> &'static [&'static str] {
    Box::leak(items.iter().map(|s| leak_str(s)).collect::<Vec<_>>().into_boxed_slice())
}

/// The built-in shape of an ACP harness, filled from the manifest. A custom
/// harness gets the generic ACP defaults for everything the manifest omits.
fn leak_config(m: &HarnessManifest) -> &'static ProviderConfig {
    let (npm_package, pinned_version) = match &m.install {
        Install::Npm { package, version } => (leak_str(package), leak_str(version)),
        _ => ("", ""),
    };
    let auth_config_dir_env_var = match &m.auth {
        Auth::ConfigDir { env_var } => leak_str(env_var),
        _ => "",
    };
    let extra_env: Vec<(&'static str, &'static str)> = m.env.iter().map(|(k, v)| (leak_str(k), leak_str(v))).collect();
    let startup = m.startup_instructions.as_deref().map(leak_str);
    let sources: Vec<&'static str> = startup.into_iter().collect();
    Box::leak(Box::new(ProviderConfig {
        id: leak_str(&m.id),
        tier: Tier::Custom,
        cli_command: leak_str(&m.command),
        controller_type: ControllerType::Acp,
        app_server: None,
        launch_args: leak_strs(&m.args),
        persistent_launch_args: None,
        resume_flag: None,
        session_id_field: "sessionId",
        styled_output_format: "acp",
        auth_config_dir_env_var,
        auth_dir_name: leak_str(&m.id),
        history_native_subdir: None,
        auth_extra_env: Box::leak(extra_env.into_boxed_slice()),
        unset_env: &[],
        npm_package,
        pinned_version,
        base_url_env_var: m.base_url_env_var.as_deref().map(leak_str),
        supported_vendors: leak_strs(&m.vendors),
        startup_instructions_filename: startup,
        native_instruction_sources: Box::leak(sources.into_boxed_slice()),
        native_instruction_dirs: &[],
    }))
}

// ─── The process-wide layer ──────────────────────────────────────────────────

static LAYER: LazyLock<Mutex<CustomLayer>> = LazyLock::new(|| Mutex::new(CustomLayer::new(MAX_LIVE, MAX_LEAKED)));
static LOADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn layer() -> std::sync::MutexGuard<'static, CustomLayer> {
    LAYER.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Read the machine's harness folder once, the first time a custom id is
/// asked for. While the shared folder is not this srv's to use, nothing is
/// read and the next ask tries again.
fn ensure_loaded() {
    use std::sync::atomic::Ordering;
    if LOADED.load(Ordering::Acquire) {
        return;
    }
    let Some(dir) = crate::registry::shared_guard::shared_dir().map(|d| d.join("harnesses")) else { return };
    let mut layer = layer();
    if LOADED.swap(true, Ordering::AcqRel) {
        return;
    }
    let report = layer.load_dir(&dir);
    for (folder, reason) in &report.refused {
        tracing::warn!(folder = %folder, "custom harness refused: {reason}");
    }
    if !report.loaded.is_empty() {
        tracing::info!(count = report.loaded.len(), "custom harnesses loaded");
    }
}

/// A registered custom harness by id. Built-ins are asked first by the caller.
pub fn get_custom_provider(id: &str) -> Option<&'static ProviderConfig> {
    if !id.starts_with(super::harness_manifest::ID_PREFIX) {
        return None;
    }
    ensure_loaded();
    layer().get(id)
}

/// Every registered custom harness, by id.
pub fn all_custom_providers() -> Vec<&'static ProviderConfig> {
    ensure_loaded();
    layer().all()
}

/// Register a manifest in the process-wide layer.
pub fn register_custom(manifest: &HarnessManifest) -> Result<&'static ProviderConfig, RegisterError> {
    ensure_loaded();
    layer().register(manifest).map(|(config, _)| config)
}

/// Take a custom harness out of the process-wide layer.
pub fn remove_custom(id: &str) -> bool {
    layer().remove(id)
}

#[cfg(test)]
#[path = "providers_custom_tests.rs"]
mod tests;
