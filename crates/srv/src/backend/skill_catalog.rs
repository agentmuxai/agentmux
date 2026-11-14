// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The public catalog of computer-use skills (docs/specs/SPEC_SKILLS_REPOSITORY_2026_11_05.md §2): fetch its signed
//! index, say what is installed here, and install or update one skill. It reuses the widget catalog's fetch and
//! signature check with its own key list: a leaked secret in one repository cannot sign the other's index.
//!
//! Three checks stand between the network and the catalog, each against the entry in the signed index: the zip's
//! SHA-256, the skill's content hash, and the author's signature. A skill installs unapproved, the same as one
//! imported from a folder: its scripts are never given to an agent until a person approves them, and its MCP
//! pointer is only shown, never installed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::backend::computer_use_skill::{check_signature, install, read_folder, Bundle};
use crate::backend::memory::record::sha256_hex;
use crate::backend::skill_file;
use crate::backend::storage::skill_app::SkillApp;
use crate::backend::storage::store::Store;
use crate::backend::storage::Skill;
use crate::backend::widget::catalog::{get, origin_of, signed_by_any};
use crate::backend::widget::signature::fingerprint;

/// Where the catalog is served.
pub const CATALOG_URL: &str = "https://agentmuxai.github.io/skills/index.json";

/// The catalog keys AgentMux trusts for this index (base64 Ed25519 public keys), more than one for a rotation. The
/// private half is a secret of the catalog's own repository; the widget catalog has a different key.
pub const PACK_CATALOG_KEYS: &[&str] = &[
    // 2026-11-06, the first skills catalog key.
    "VBSn+oh9qB6bnv+BoGifqfwqDXTh3BPIsQb/8RMg+Do=",
];

const MAX_INDEX_BYTES: usize = 5 * 1024 * 1024;
const CATALOG_VERSION: u32 = 1;
/// A catalog zip is a `.skill` file, which is at most this, plus some slack for the zip's own headers.
const MAX_ZIP_BYTES: usize = skill_file::MAX_ZIP_BYTES as usize + 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "AppSkillCatalogVersions")]
pub struct CatalogVersions {
    #[serde(default)]
    pub supported: Option<String>,
    #[serde(default)]
    pub tested: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "AppSkillCatalogApp")]
pub struct CatalogApp {
    #[serde(default)]
    pub vendor: String,
    /// Per platform, the executable names the app runs as: how its version is found on this computer.
    #[serde(default)]
    pub executables: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub versions: CatalogVersions,
}

/// One test the catalog records for a version of a skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "AppSkillCatalogRecord")]
pub struct CatalogRecord {
    pub skill_version: String,
    pub app_version: String,
    pub platform: String,
    pub date: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "AppSkillCatalogEntry")]
pub struct CatalogEntry {
    /// `<publisher>.<id>`.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub publisher: String,
    #[serde(default)]
    pub app: CatalogApp,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub routes: Vec<String>,
    /// Whether it carries a script that does something (not only a test).
    #[serde(default)]
    pub has_scripts: bool,
    /// The MCP catalog entry it points at, which is installed only through the MCP tab.
    #[serde(default)]
    pub mcp: Option<String>,
    /// The skill's content hash.
    pub hash: String,
    /// Its author's key (base64), which signed it.
    pub publisher_key: String,
    #[serde(default)]
    pub verification: Vec<CatalogRecord>,
    pub zip: String,
    pub zip_sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogIndex {
    catalog_version: u32,
    #[serde(default)]
    skills: Vec<CatalogEntry>,
}

/// An entry as Browse shows it: the entry, its author's fingerprint and badge, and what is installed here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct AppSkillCatalogItem {
    pub entry: CatalogEntry,
    pub fingerprint: String,
    /// `agentmux` when the AgentMux catalog key signed it, else `community`.
    pub badge: String,
    /// `not_installed`, `current` (the same files), `update` (installed here is older), `newer_here` (installed here is
    /// newer than the catalog's), or `changed` (the same version, other files).
    pub state: String,
    pub installed_version: Option<String>,
    pub skill_id: Option<String>,
    /// The application itself is on this computer (found by the executable names the entry lists for this platform).
    pub app_found: bool,
    /// The application's version here, when it is found and says one.
    pub app_version: Option<String>,
}

fn is_part(s: &str) -> bool {
    !s.is_empty() && s.split('-').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

/// Checks the index `raw` against its signature with `keys`, and each entry against the catalog's rules.
pub fn verify_index(raw: &[u8], sig_b64: &str, keys: &[&str], index_url: &str) -> Result<Vec<CatalogEntry>, String> {
    let signature: [u8; 64] = base64::engine::general_purpose::STANDARD
        .decode(sig_b64.trim())
        .ok()
        .and_then(|s| s.try_into().ok())
        .ok_or("the catalog's signature isn't 64 bytes of base64")?;
    if !signed_by_any(raw, &signature, keys) {
        return Err("the catalog's signature doesn't verify: it isn't the AgentMux skill catalog, or it was changed".to_string());
    }
    let index: CatalogIndex = serde_json::from_slice(raw).map_err(|e| format!("the catalog index: {e}"))?;
    if index.catalog_version != CATALOG_VERSION {
        return Err(format!("the catalog is version {}, this AgentMux reads {CATALOG_VERSION}", index.catalog_version));
    }
    let origin = origin_of(index_url).ok_or("the catalog URL isn't an https:// URL")?;
    for e in &index.skills {
        let named = e.id.split_once('.').is_some_and(|(p, i)| is_part(p) && is_part(i) && p == e.publisher);
        if !named {
            return Err(format!("the catalog lists {:?}, which isn't a <publisher>.<id> skill", e.id));
        }
        if origin_of(&e.zip).as_deref() != Some(origin.as_str()) {
            return Err(format!("the catalog's {} isn't served from the catalog itself", e.id));
        }
    }
    Ok(index.skills)
}

/// The catalog at `url`, checked.
pub async fn fetch(url: &str) -> Result<Vec<CatalogEntry>, String> {
    let raw = get(url, MAX_INDEX_BYTES).await?;
    let sig_text = String::from_utf8(get(&format!("{url}.sig"), 1024).await?).map_err(|_| "the catalog's signature isn't text")?;
    verify_index(&raw, &sig_text, PACK_CATALOG_KEYS, url)
}

/// The zip an entry points at, from the catalog's own origin (no redirects).
pub async fn download(entry: &CatalogEntry) -> Result<Vec<u8>, String> {
    get(&entry.zip, MAX_ZIP_BYTES).await
}

/// `1.2.3[-pre][+build]` ordered as semver does: the numbers (a missing one is 0), then a pre-release sorts before
/// the same release and pre-releases compare part by part (numbers before words); `+build` is ignored.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn split(v: &str) -> (Vec<u64>, Option<&str>) {
        let core = v.split('+').next().unwrap_or("");
        let (nums, pre) = core.split_once('-').map_or((core, None), |(n, p)| (n, Some(p)));
        (nums.split('.').map(|p| p.parse().unwrap_or(0)).collect(), pre)
    }
    let ((mut an, ap), (mut bn, bp)) = (split(a), split(b));
    let len = an.len().max(bn.len());
    an.resize(len, 0);
    bn.resize(len, 0);
    an.cmp(&bn).then_with(|| match (ap, bp) {
        (None, None) => Ordering::Equal,
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(x), Some(y)) => {
            let part = |p: &str| p.parse::<u64>().map_or((1, 0, p.to_string()), |n| (0, n, String::new()));
            x.split('.').map(part).cmp(y.split('.').map(part))
        }
    })
}

/// What Browse lists: each entry with what is installed here (`installed` is `Store::skill_app_map`).
pub fn items(entries: Vec<CatalogEntry>, installed: &BTreeMap<String, SkillApp>) -> Vec<AppSkillCatalogItem> {
    entries
        .into_iter()
        .map(|entry| {
            let here = installed.iter().find(|(_, a)| format!("{}.{}", a.info.publisher, a.info.id) == entry.id);
            let state = match here {
                None => "not_installed",
                Some((_, a)) if a.content_hash == entry.hash => "current",
                Some((_, a)) => match version_cmp(&entry.version, &a.info.version) {
                    std::cmp::Ordering::Greater => "update",
                    std::cmp::Ordering::Less => "newer_here",
                    std::cmp::Ordering::Equal => "changed",
                },
            };
            AppSkillCatalogItem {
                fingerprint: fingerprint(&entry.publisher_key),
                badge: if PACK_CATALOG_KEYS.contains(&entry.publisher_key.as_str()) { "agentmux" } else { "community" }.to_string(),
                state: state.to_string(),
                installed_version: here.map(|(_, a)| a.info.version.clone()),
                skill_id: here.map(|(id, _)| id.clone()),
                app_found: false,
                app_version: None,
                entry,
            }
        })
        .collect()
}

/// Says for each entry whether its application is on this computer, and which version, by the executable names it lists
/// for `platform` (`find` is `app_version::find`). An entry that lists none for this platform is left as it is.
pub fn add_app_versions(items: &mut [AppSkillCatalogItem], platform: &str, find: impl Fn(&[String]) -> Option<crate::backend::app_version::Found>) {
    for item in items {
        let Some(names) = item.entry.app.executables.get(platform).filter(|n| !n.is_empty()) else { continue };
        if let Some(found) = find(names) {
            item.app_found = true;
            item.app_version = found.version;
        }
    }
}

/// The platform name skills use for the computer this runs on.
pub fn this_platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// Checks a downloaded zip against its entry: the zip's SHA-256, then, unpacked and read like any skill folder, the
/// content hash, the id and version, and the author's signature (which must be the key the catalog lists, and not
/// another key than the one this publisher is pinned to here). Returns the skill and the key that signed it.
pub fn check_download(catalog: &Store, zip: &[u8], entry: &CatalogEntry) -> Result<(Bundle, String), String> {
    let id = &entry.id;
    if sha256_hex(zip) != entry.zip_sha256 {
        return Err(format!("{id}'s download doesn't match the catalog"));
    }
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let zip_path = tmp.path().join("skill.zip");
    std::fs::write(&zip_path, zip).map_err(|e| e.to_string())?;
    let stage = tmp.path().join("skill");
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    skill_file::extract(&zip_path, &stage).map_err(|e| format!("{id}: {e}"))?;
    let folder: PathBuf = std::fs::read_dir(&stage)
        .map_err(|e| e.to_string())?
        .flatten()
        .find(|e| e.path().is_dir())
        .map(|e| e.path())
        .ok_or_else(|| format!("{id}'s download holds no folder"))?;
    let bundle = read_folder(&folder).map_err(|e| format!("{id}: {e}"))?;
    if bundle.content_hash != entry.hash {
        return Err(format!("{id}'s files don't match the catalog"));
    }
    if format!("{}.{}", bundle.info.publisher, bundle.info.id) != *id || bundle.info.version != entry.version {
        return Err(format!("{id}'s manifest doesn't match the catalog"));
    }
    match check_signature(catalog, &bundle)? {
        Some(key) if key == entry.publisher_key => Ok((bundle, key)),
        _ => Err(format!("{id} isn't signed by the author the catalog lists")),
    }
}

/// What `install_download` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub skill_id: String,
    pub name: String,
    pub version: String,
    /// An earlier version was replaced.
    pub updated: bool,
}

/// Check `zip` against `entry` and add it to the catalog as a computer-use skill, or replace the version installed
/// here. Scripts arrive unapproved (a script whose text is unchanged keeps the approval a person gave it). A new
/// skill is global, bound to nobody and outside "new agents get it"; an update keeps its binds and reach.
pub fn install_download(catalog: &Store, zip: &[u8], entry: &CatalogEntry, now_ms: i64) -> Result<Installed, String> {
    let (bundle, signer) = check_download(catalog, zip, entry)?;
    let existing = catalog
        .skill_app_map()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|(_, a)| format!("{}.{}", a.info.publisher, a.info.id) == entry.id)
        .map(|(id, _)| id);
    let current = match &existing {
        Some(id) => catalog.skill_get(id).map_err(|e| e.to_string())?,
        None => None,
    };
    let skill = Skill {
        id: current.as_ref().map_or_else(|| uuid::Uuid::new_v4().to_string(), |s| s.id.clone()),
        name: bundle.name.clone(),
        trigger: current.as_ref().map_or_else(|| bundle.name.clone(), |s| s.trigger.clone()),
        skill_type: current.as_ref().map_or_else(|| "agent-skill".to_string(), |s| s.skill_type.clone()),
        description: bundle.description.clone(),
        content: bundle.content.clone(),
        is_global: true,
        created_at: current.as_ref().map_or(now_ms, |s| s.created_at),
        updated_at: now_ms,
    };
    catalog.skill_upsert_unique_global(&skill).map_err(|e| e.to_string())?;
    if current.is_none() {
        catalog.skill_set_reach(&skill.id, false, false, now_ms).map_err(|e| e.to_string())?;
    }
    install(catalog, &skill.id, &bundle, Some(&signer), now_ms).map_err(|e| e.to_string())?;
    Ok(Installed { skill_id: skill.id, name: skill.name, version: bundle.info.version, updated: current.is_some() })
}

#[cfg(test)]
#[path = "skill_catalog_tests.rs"]
mod tests;
