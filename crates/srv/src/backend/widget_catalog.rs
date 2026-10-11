// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The widget catalog (docs/specs/SPEC_WIDGET_SHARING_2026_10_10.md §4): a
//! public list of sandboxed widgets, built and signed by the
//! `agentmuxai/widgets` repository's CI, that Settings → Widgets browses.
//!
//! The catalog is a convenience and a second check, never an approval. srv
//! checks the index's signature against a key pinned here, downloads a
//! widget only from the catalog's own origin, checks the zip's SHA-256, the
//! package's content hash and its author's signature against the index, and
//! installs it as `needs_approval`, like any widget: the user's prompt
//! decides.

use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::widget_packages::{self as wp, WidgetState};
use super::widget_signature as sig;

/// Where the official catalog is served.
pub const DEFAULT_CATALOG_URL: &str = "https://agentmuxai.github.io/widgets/index.json";

/// The catalog keys AgentMux trusts (base64 Ed25519 public keys). The
/// private half lives only in the catalog repository's CI. More than one,
/// for a rotation.
pub const CATALOG_KEYS: &[&str] = &[];

const MAX_INDEX_BYTES: usize = 5 * 1024 * 1024;
const CATALOG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../frontend/types/rpc/", rename = "WidgetCatalogEntry")]
pub struct CatalogEntry {
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
    pub permissions: Vec<String>,
    /// The package's content hash.
    pub hash: String,
    /// Its publisher's key (base64), which signed it.
    pub publisher_key: String,
    pub zip: String,
    pub zip_sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogIndex {
    catalog_version: u32,
    #[serde(default)]
    widgets: Vec<CatalogEntry>,
}

/// An entry as Settings shows it: the entry, its publisher's fingerprint,
/// and what is installed here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetCatalogItem {
    pub entry: CatalogEntry,
    pub fingerprint: String,
    pub installed_version: Option<String>,
    pub installed_state: Option<WidgetState>,
    /// Installed here at exactly the catalog's files.
    pub current: bool,
}

/// Checks the index `raw` against its signature `sig_b64` with `keys`, and
/// each entry against the catalog's rules.
pub fn verify_index(raw: &[u8], sig_b64: &str, keys: &[&str], index_url: &str) -> Result<Vec<CatalogEntry>, String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let signature: [u8; 64] = b64
        .decode(sig_b64.trim())
        .ok()
        .and_then(|s| s.try_into().ok())
        .ok_or("the catalog's signature isn't 64 bytes of base64")?;
    let signature = Signature::from_bytes(&signature);
    let signed = keys.iter().any(|k| {
        b64.decode(k)
            .ok()
            .and_then(|k| <[u8; 32]>::try_from(k).ok())
            .and_then(|k| VerifyingKey::from_bytes(&k).ok())
            .is_some_and(|k| k.verify(raw, &signature).is_ok())
    });
    if !signed {
        return Err("the catalog's signature doesn't verify: it isn't the AgentMux catalog, or it was changed".to_string());
    }
    let index: CatalogIndex = serde_json::from_slice(raw).map_err(|e| format!("the catalog index: {e}"))?;
    if index.catalog_version != CATALOG_VERSION {
        return Err(format!("the catalog is version {}, this AgentMux reads {CATALOG_VERSION}", index.catalog_version));
    }
    let origin = origin_of(index_url).ok_or("the catalog URL isn't an https:// URL")?;
    for e in &index.widgets {
        if !wp::valid_id(&e.id) {
            return Err(format!("the catalog lists {:?}, which isn't a widget id", e.id));
        }
        if origin_of(&e.zip).as_deref() != Some(origin.as_str()) {
            return Err(format!("the catalog's {} isn't served from the catalog itself", e.id));
        }
    }
    Ok(index.widgets)
}

/// `https://host[:port]` of an https URL.
fn origin_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty() && !host.contains('@')).then(|| format!("https://{host}"))
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

async fn get(url: &str, max: usize) -> Result<Vec<u8>, String> {
    let resp = client()?.get(url).send().await.map_err(|e| format!("can't reach the catalog: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("the catalog answered {} for {url}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("the catalog: {e}"))?;
    if bytes.len() > max {
        return Err(format!("{url} is over {} MB", max / (1024 * 1024)));
    }
    Ok(bytes.to_vec())
}

/// The catalog at `url`, checked.
pub async fn fetch(url: &str) -> Result<Vec<CatalogEntry>, String> {
    let raw = get(url, MAX_INDEX_BYTES).await?;
    let sig_text = String::from_utf8(get(&format!("{url}.sig"), 1024).await?).map_err(|_| "the catalog's signature isn't text")?;
    verify_index(&raw, &sig_text, CATALOG_KEYS, url)
}

/// What Settings lists: each entry with what is installed here.
pub fn items(entries: Vec<CatalogEntry>, installed: &[wp::WidgetPackageInfo]) -> Vec<WidgetCatalogItem> {
    entries
        .into_iter()
        .map(|entry| {
            let i = installed.iter().find(|p| p.id == entry.id);
            WidgetCatalogItem {
                fingerprint: sig::fingerprint(&entry.publisher_key),
                installed_version: i.map(|p| p.version.clone()),
                installed_state: i.map(|p| p.state.clone()),
                current: i.is_some_and(|p| p.hash == entry.hash),
                entry,
            }
        })
        .collect()
}

/// Checks a downloaded zip against its entry: the zip's SHA-256, then,
/// unpacked, the package's content hash and its author's signature.
/// Returns the unpacked package's folder (in the returned temp dir).
pub fn check_download(zip: &[u8], entry: &CatalogEntry) -> Result<(tempfile::TempDir, PathBuf), String> {
    if hex::encode(Sha256::digest(zip)) != entry.zip_sha256 {
        return Err(format!("{}'s download doesn't match the catalog", entry.id));
    }
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let zip_path = tmp.path().join("pkg.zip");
    std::fs::write(&zip_path, zip).map_err(|e| e.to_string())?;
    let stage = tmp.path().join("pkg");
    wp::unzip(&zip_path, &stage)?;
    let root = package_root(&stage)?;
    let hash = wp::package_hash(&wp::hash_files(&root)?);
    if hash != entry.hash {
        return Err(format!("{}'s files don't match the catalog", entry.id));
    }
    let manifest: wp::Manifest = serde_json::from_slice(&std::fs::read(root.join(wp::MANIFEST_FILE)).map_err(|e| e.to_string())?)
        .map_err(|e| format!("widget.json: {e}"))?;
    if manifest.id != entry.id || manifest.version != entry.version {
        return Err(format!("{}'s widget.json doesn't match the catalog", entry.id));
    }
    if manifest.kind() != wp::WidgetKind::Sandboxed {
        return Err("the catalog lists sandboxed widgets only".to_string());
    }
    match sig::check(&root, &entry.id, &entry.version, &hash) {
        Some(Ok(key)) if key == entry.publisher_key => {}
        Some(Ok(_)) | None => return Err(format!("{} isn't signed by the publisher the catalog lists", entry.id)),
        Some(Err(e)) => return Err(e),
    }
    std::fs::remove_file(&zip_path).ok();
    Ok((tmp, root))
}

/// The package folder in an unpacked zip: its root, or its one top folder.
fn package_root(stage: &Path) -> Result<PathBuf, String> {
    if stage.join(wp::MANIFEST_FILE).exists() {
        return Ok(stage.to_path_buf());
    }
    let mut dirs = std::fs::read_dir(stage).map_err(|e| e.to_string())?.flatten().filter(|e| e.path().is_dir());
    match (dirs.next(), dirs.next()) {
        (Some(d), None) if d.path().join(wp::MANIFEST_FILE).exists() => Ok(d.path()),
        _ => Err("no widget.json at the package's top level".to_string()),
    }
}

/// Downloads `entry` from the catalog, checks it, and copies it into
/// `widgets_dir` (replacing an installed version), waiting for approval.
pub async fn install(widgets_dir: &Path, entry: &CatalogEntry) -> Result<String, String> {
    let zip = get(&entry.zip, (wp::MAX_PACKAGE_BYTES + 1024 * 1024) as usize).await?;
    let (entry, dir) = (entry.clone(), widgets_dir.to_path_buf());
    tokio::task::spawn_blocking(move || {
        let (_tmp, root) = check_download(&zip, &entry)?;
        wp::install(&dir, &root, true)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
#[path = "widget_catalog_tests.rs"]
mod tests;
