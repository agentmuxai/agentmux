// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Job templates: signed data a job is made from
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4, §5.5).
//!
//! A template comes compiled in (`templates/jobs/`, published by AgentMux),
//! installed from a folder or a zip, or from a private catalog. Installed
//! templates live in `~/.agentmux/job-templates/`, beside the widgets. Every
//! template waits for the operator's approval, which only the host carries,
//! like a widget's; a template that isn't approved, or changed since, can't
//! make a job ([`JobTemplates::resolve_approved`]). Publisher keys are pinned
//! on first approval, in `job-template-publishers.json` beside the widgets'
//! pins. Approving a template runs nothing.

pub mod catalog;
pub mod manifest;
pub mod package;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use manifest::{JobTemplateManifest, RenderedBrief};
use package::Package;

use crate::backend::widget_packages as wp;
use crate::backend::widget_signature::{self as sig, Pins, WidgetPublisherPin, WidgetSignatureInfo};

const APPROVALS_FILE: &str = "job-template-approvals.json";
const PUBLISHERS_FILE: &str = "job-template-publishers.json";
const CATALOGS_FILE: &str = "job-template-catalogs.json";

/// The publisher of the templates AgentMux ships; no other template may
/// use it.
pub const BUILTIN_PUBLISHER: &str = "agentmux";

/// A template compiled into srv from `templates/jobs/<id>/`.
pub struct Builtin {
    pub id: &'static str,
    pub files: &'static [(&'static str, &'static [u8])],
}

/// A built-in template: its id, and its two files from `templates/jobs/<id>/`.
macro_rules! builtin {
    ($name:literal) => {
        Builtin {
            id: concat!("agentmux.", $name),
            files: &[
                ("template.json", include_bytes!(concat!("../../../../../templates/jobs/agentmux.", $name, "/template.json"))),
                ("brief.md", include_bytes!(concat!("../../../../../templates/jobs/agentmux.", $name, "/brief.md"))),
            ],
        }
    };
}

/// The templates AgentMux ships (§5.5, §5.6). Each still needs the
/// operator's approval once, so they see its outputs.
pub static BUILTINS: &[Builtin] = &[
    builtin!("weekly-report"),
    builtin!("pr-review"),
    builtin!("nightly-build"),
    builtin!("daily-digest"),
    builtin!("dependency-scan"),
    builtin!("docs-drift"),
    builtin!("stale-sweep"),
    builtin!("backup-verify"),
    builtin!("on-event"),
    builtin!("weekly-status"),
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum JobTemplateSource {
    Builtin,
    Installed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum JobTemplateState {
    Approved,
    NeedsApproval,
    /// Its files or its signer changed since the operator approved it.
    Changed,
    Invalid,
}

/// A template as the UI lists it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateInfo {
    pub id: String,
    pub source: JobTemplateSource,
    pub state: JobTemplateState,
    pub error: Option<String>,
    /// The current content hash (empty when invalid).
    pub hash: String,
    pub manifest: Option<JobTemplateManifest>,
    /// Who signed it, against the pinned publishers. A built-in one is
    /// unsigned and published by AgentMux.
    pub signature: WidgetSignatureInfo,
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Approval {
    hash: String,
    version: String,
    signer: Option<String>,
    source: JobTemplateSource,
    approved_at: i64,
}

/// A template the operator approved, exactly as approved: what a job is
/// made from. A job records its id, version and hash.
#[allow(dead_code, reason = "the job definition reads it when it makes a job from a template")]
#[derive(Debug, Clone)]
pub struct ApprovedTemplate {
    pub id: String,
    pub version: String,
    pub hash: String,
    pub manifest: JobTemplateManifest,
    pub brief: Option<String>,
    pub source: JobTemplateSource,
    pub signer: Option<String>,
}

#[allow(dead_code, reason = "the job definition renders the brief when it makes a job")]
impl ApprovedTemplate {
    /// Its brief with `params` filled in.
    pub fn render(&self, params: &Map<String, Value>) -> Result<RenderedBrief, String> {
        manifest::render(&self.manifest, self.brief.as_deref().unwrap_or(""), params)
    }
}

/// A template found, before approval is applied.
struct Found {
    id: String,
    source: JobTemplateSource,
    dir: Option<PathBuf>,
    package: Result<Package, String>,
}

fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

// ── The service ─────────────────────────────────────────────────────────────

pub struct JobTemplates {
    pub templates_dir: PathBuf,
    approvals_path: PathBuf,
    pins_path: PathBuf,
    catalogs_path: PathBuf,
    builtins: &'static [Builtin],
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    approvals: BTreeMap<String, Approval>,
    pins: Pins,
    catalogs: Vec<catalog::Catalog>,
}

static SERVICE: OnceLock<Arc<JobTemplates>> = OnceLock::new();

pub fn service() -> Option<&'static Arc<JobTemplates>> {
    SERVICE.get()
}

/// `~/.agentmux/job-templates`, beside the widgets folder.
pub fn templates_dir() -> PathBuf {
    wp::widgets_dir().with_file_name("job-templates")
}

/// Start the service: this instance's approvals, pins and catalogs, from
/// `data_dir` beside the widgets'.
pub fn start(data_dir: &Path) {
    let data_dir = if data_dir.as_os_str().is_empty() { super::base::get_mux_data_dir() } else { data_dir.to_path_buf() };
    JobTemplates::new(templates_dir(), &data_dir, BUILTINS).install_global();
}

impl JobTemplates {
    pub fn new(templates_dir: PathBuf, data_dir: &Path, builtins: &'static [Builtin]) -> Self {
        let (approvals_path, pins_path, catalogs_path) =
            (data_dir.join(APPROVALS_FILE), data_dir.join(PUBLISHERS_FILE), data_dir.join(CATALOGS_FILE));
        let inner = Inner { approvals: read_json(&approvals_path), pins: sig::read_pins(&pins_path), catalogs: read_json(&catalogs_path) };
        Self { templates_dir, approvals_path, pins_path, catalogs_path, builtins, inner: Mutex::new(inner) }
    }

    pub fn install_global(self) -> &'static Arc<JobTemplates> {
        SERVICE.get_or_init(|| Arc::new(self))
    }

    /// The process-wide service for tests, in a temp folder that lives as
    /// long as the process; each test uses its own template ids.
    #[cfg(test)]
    pub fn for_tests() -> &'static Arc<JobTemplates> {
        static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
        let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
        JobTemplates::new(dir.path().join("job-templates"), &dir.path().join("data"), &[]).install_global()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Every template: the built-in ones, then each folder in the templates
    /// folder that holds a `template.json`, read now.
    fn scan(&self) -> Vec<Found> {
        let mut out: Vec<Found> = self
            .builtins
            .iter()
            .map(|b| Found {
                id: b.id.to_string(),
                source: JobTemplateSource::Builtin,
                dir: None,
                package: package::load_embedded(b.id, b.files),
            })
            .collect();
        let mut installed = Vec::new();
        for entry in std::fs::read_dir(&self.templates_dir).into_iter().flatten().flatten() {
            let dir = entry.path();
            if !entry.file_type().is_ok_and(|t| t.is_dir()) || !dir.join(package::TEMPLATE_FILE).exists() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            let package = if sig::publisher_of(&id) == BUILTIN_PUBLISHER {
                Err(format!("the publisher {BUILTIN_PUBLISHER:?} is kept for the templates AgentMux ships"))
            } else {
                package::load_dir(&dir, &id)
            };
            installed.push(Found { id, source: JobTemplateSource::Installed, dir: Some(dir), package });
        }
        installed.sort_by(|a, b| a.id.cmp(&b.id));
        out.extend(installed);
        out
    }

    fn find(&self, id: &str) -> Option<Found> {
        if let Some(b) = self.builtins.iter().find(|b| b.id == id) {
            return Some(Found {
                id: id.to_string(),
                source: JobTemplateSource::Builtin,
                dir: None,
                package: package::load_embedded(b.id, b.files),
            });
        }
        if !wp::valid_id(id) || sig::publisher_of(id) == BUILTIN_PUBLISHER {
            return None;
        }
        let dir = self.templates_dir.join(id);
        dir.join(package::TEMPLATE_FILE).exists().then(|| Found {
            id: id.to_string(),
            source: JobTemplateSource::Installed,
            package: package::load_dir(&dir, id),
            dir: Some(dir),
        })
    }

    fn describe(f: &Found, inner: &Inner) -> JobTemplateInfo {
        let folder = f.dir.as_ref().map(|d| d.display().to_string());
        let pkg = match &f.package {
            Ok(p) => p,
            Err(e) => {
                return JobTemplateInfo {
                    id: f.id.clone(),
                    source: f.source,
                    state: JobTemplateState::Invalid,
                    error: Some(e.clone()),
                    hash: String::new(),
                    manifest: None,
                    signature: sig::describe(&f.id, None, &inner.pins),
                    folder,
                }
            }
        };
        let state = match inner.approvals.get(&f.id) {
            None => JobTemplateState::NeedsApproval,
            Some(a) if a.hash != pkg.hash || a.signer != pkg.signer || a.source != f.source => JobTemplateState::Changed,
            Some(_) => JobTemplateState::Approved,
        };
        JobTemplateInfo {
            id: f.id.clone(),
            source: f.source,
            state,
            error: None,
            hash: pkg.hash.clone(),
            manifest: Some(pkg.manifest.clone()),
            signature: sig::describe(&f.id, pkg.signer.as_deref(), &inner.pins),
            folder,
        }
    }

    pub fn list(&self) -> Vec<JobTemplateInfo> {
        let found = self.scan();
        let inner = self.lock();
        found.iter().map(|f| Self::describe(f, &inner)).collect()
    }

    /// One template, and its brief when it is valid.
    pub fn get(&self, id: &str) -> Option<(JobTemplateInfo, Option<Package>)> {
        let f = self.find(id)?;
        let info = Self::describe(&f, &self.lock());
        Some((info, f.package.ok()))
    }

    /// Record the operator's approval of exactly `hash`, signed by the key
    /// whose fingerprint their prompt showed (`shown_signer`, "" for
    /// unsigned); only the host route calls this. Fails if the template or
    /// its signature changed since the prompt was shown.
    pub fn approve(&self, id: &str, hash: &str, shown_signer: &str) -> Result<(), String> {
        let f = self.find(id).ok_or("no such job template")?;
        let pkg = f.package?;
        if pkg.hash != hash {
            return Err("the template changed since you were asked; review it again".to_string());
        }
        if pkg.signer.as_deref().map(sig::fingerprint).unwrap_or_default() != shown_signer {
            return Err("the template's signature changed since you were asked; review it again".to_string());
        }
        let mut inner = self.lock();
        // The first signed template of a publisher pins it to its key; a
        // later one signed otherwise doesn't move the pin, and shows as
        // `key_changed` in its prompt.
        let publisher = sig::publisher_of(id).to_string();
        if let Some(key) = pkg.signer.clone().filter(|_| !inner.pins.contains_key(&publisher)) {
            inner.pins.insert(publisher, key);
            sig::write_pins(&self.pins_path, &inner.pins)?;
        }
        let approval = Approval {
            hash: pkg.hash,
            version: pkg.manifest.version,
            signer: pkg.signer,
            source: f.source,
            approved_at: chrono::Utc::now().timestamp(),
        };
        inner.approvals.insert(id.to_string(), approval);
        write_json(&self.approvals_path, &inner.approvals)
    }

    /// The approved template `id` at `version`, read from disk now: an
    /// error if it isn't installed at that version, isn't approved, or
    /// changed since the operator approved it.
    #[allow(dead_code, reason = "the job definition resolves a template when it makes a job")]
    pub fn resolve_approved(&self, id: &str, version: &str) -> Result<ApprovedTemplate, String> {
        let f = self.find(id).ok_or_else(|| format!("no job template {id}"))?;
        let pkg = f.package.map_err(|e| format!("{id} isn't a valid template: {e}"))?;
        if pkg.manifest.version != version {
            return Err(format!("{id} is at version {}, not {version}", pkg.manifest.version));
        }
        match self.lock().approvals.get(id) {
            None => return Err(format!("{id} isn't approved: the operator approves a template before a job can use it")),
            Some(a) if a.hash != pkg.hash || a.signer != pkg.signer || a.source != f.source => {
                return Err(format!("{id} changed since the operator approved it; it needs their approval again"))
            }
            Some(_) => {}
        }
        Ok(ApprovedTemplate {
            id: id.to_string(),
            version: pkg.manifest.version.clone(),
            hash: pkg.hash,
            manifest: pkg.manifest,
            brief: pkg.brief,
            source: f.source,
            signer: pkg.signer,
        })
    }

    /// Copy the template at `source` (a folder, its `template.json`, or a
    /// `.zip`) into the templates folder, waiting for approval.
    pub fn install(&self, source: &Path, replace: bool) -> Result<String, String> {
        wp::install_package(&self.templates_dir, source, replace, package::TEMPLATE_FILE, |root| {
            let raw = std::fs::read(root.join(package::TEMPLATE_FILE)).map_err(|e| format!("can't read template.json: {e}"))?;
            let m: JobTemplateManifest = serde_json::from_slice(&raw).map_err(|e| format!("template.json: {e}"))?;
            if sig::publisher_of(&m.id) == BUILTIN_PUBLISHER {
                return Err(format!("the publisher {BUILTIN_PUBLISHER:?} is kept for the templates AgentMux ships"));
            }
            package::load_dir(root, &m.id).map(|p| p.manifest.id)
        })
    }

    /// Delete an installed template's folder and its approval.
    pub fn uninstall(&self, id: &str) -> Result<(), String> {
        let f = self.find(id).ok_or("no such job template")?;
        let dir = f.dir.ok_or("a built-in template can't be uninstalled")?;
        std::fs::remove_dir_all(&dir).map_err(|e| format!("can't remove it: {e}"))?;
        let mut inner = self.lock();
        inner.approvals.remove(id);
        write_json(&self.approvals_path, &inner.approvals)
    }

    /// The publishers pinned to a key, for Settings.
    pub fn publishers(&self) -> Vec<WidgetPublisherPin> {
        let inner = self.lock();
        inner.pins.iter().map(|(publisher, key)| WidgetPublisherPin { publisher: publisher.clone(), fingerprint: sig::fingerprint(key) }).collect()
    }

    /// Forget a publisher's key (the operator's, through the host route).
    pub fn forget_publisher(&self, publisher: &str) -> Result<(), String> {
        let mut inner = self.lock();
        if inner.pins.remove(publisher).is_none() {
            return Err(format!("no key is kept for {publisher:?}"));
        }
        sig::write_pins(&self.pins_path, &inner.pins)
    }

    pub fn catalogs(&self) -> Vec<catalog::Catalog> {
        self.lock().catalogs.clone()
    }

    /// Add a private catalog, or change its key (the operator's, through the
    /// host route).
    pub fn add_catalog(&self, url: &str, public_key: &str) -> Result<(), String> {
        let c = catalog::Catalog::new(url, public_key)?;
        let mut inner = self.lock();
        inner.catalogs.retain(|x| x.url != c.url);
        inner.catalogs.push(c);
        write_json(&self.catalogs_path, &inner.catalogs)
    }

    pub fn remove_catalog(&self, url: &str) -> Result<(), String> {
        let mut inner = self.lock();
        let before = inner.catalogs.len();
        inner.catalogs.retain(|x| x.url != url);
        if inner.catalogs.len() == before {
            return Err(format!("no catalog at {url}"));
        }
        write_json(&self.catalogs_path, &inner.catalogs)
    }
}

#[cfg(test)]
mod starters_tests;
#[cfg(test)]
mod tests;
