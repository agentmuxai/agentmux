// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Private job template catalogs (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §5.5): an `index.json` at a URL the operator added, signed by the key they
//! pinned for it (`index.json.sig`, the widget catalog's scheme).
//!
//! Each entry names its zip, the zip's SHA-256, the template's content hash
//! and its publisher's key. srv downloads a template only from the catalog's
//! own origin, checks all three, and installs it as `needs_approval`: the
//! catalog is a second check, never an approval.

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::package::{self, MAX_BYTES, TEMPLATE_FILE};
use super::BUILTIN_PUBLISHER;
use crate::backend::widget_catalog as wc;
use crate::backend::widget_packages as wp;
use crate::backend::widget_signature as sig;

const MAX_INDEX_BYTES: usize = 1024 * 1024;
const CATALOG_VERSION: u32 = 1;

/// A catalog the operator added: where its index is, and the key it is
/// signed with (base64 Ed25519).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Catalog {
    pub url: String,
    pub public_key: String,
}

impl Catalog {
    pub fn new(url: &str, public_key: &str) -> Result<Self, String> {
        let url = url.trim();
        wc::origin_of(url).ok_or("a catalog's URL must be an https:// URL")?;
        let public_key = public_key.trim();
        let key = base64::engine::general_purpose::STANDARD.decode(public_key).ok().and_then(|k| <[u8; 32]>::try_from(k).ok());
        key.and_then(|k| ed25519_dalek::VerifyingKey::from_bytes(&k).ok())
            .ok_or("a catalog's key must be a 32-byte Ed25519 public key in base64")?;
        Ok(Self { url: url.to_string(), public_key: public_key.to_string() })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobTemplateCatalogEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    /// The template's content hash.
    pub hash: String,
    /// Its publisher's key (base64), which signed it.
    pub publisher_key: String,
    pub zip: String,
    pub zip_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Index {
    catalog_version: u32,
    #[serde(default)]
    templates: Vec<JobTemplateCatalogEntry>,
}

/// Checks the index `raw` against its signature `sig_b64` and the catalog's
/// pinned key, and each entry against the catalog's rules.
pub fn verify_index(raw: &[u8], sig_b64: &str, catalog: &Catalog) -> Result<Vec<JobTemplateCatalogEntry>, String> {
    let signature: [u8; 64] = base64::engine::general_purpose::STANDARD
        .decode(sig_b64.trim())
        .ok()
        .and_then(|s| s.try_into().ok())
        .ok_or("the catalog's signature isn't 64 bytes of base64")?;
    if !wc::signed_by_any(raw, &signature, &[catalog.public_key.as_str()]) {
        return Err(format!("{}'s signature doesn't verify with the key you pinned for it", catalog.url));
    }
    let index: Index = serde_json::from_slice(raw).map_err(|e| format!("the catalog index: {e}"))?;
    if index.catalog_version != CATALOG_VERSION {
        return Err(format!("the catalog is version {}, this AgentMux reads {CATALOG_VERSION}", index.catalog_version));
    }
    let origin = wc::origin_of(&catalog.url).ok_or("the catalog URL isn't an https:// URL")?;
    for e in &index.templates {
        if !wp::valid_id(&e.id) || sig::publisher_of(&e.id) == BUILTIN_PUBLISHER {
            return Err(format!("the catalog lists {:?}, which isn't a template id it may use", e.id));
        }
        if wc::origin_of(&e.zip).as_deref() != Some(origin.as_str()) {
            return Err(format!("the catalog's {} isn't served from the catalog itself", e.id));
        }
    }
    Ok(index.templates)
}

/// The catalog's entries, fetched and checked.
pub async fn fetch(catalog: &Catalog) -> Result<Vec<JobTemplateCatalogEntry>, String> {
    let raw = wc::get(&catalog.url, MAX_INDEX_BYTES).await?;
    let sig_text = String::from_utf8(wc::get(&format!("{}.sig", catalog.url), 1024).await?).map_err(|_| "the catalog's signature isn't text")?;
    verify_index(&raw, &sig_text, catalog)
}

/// Checks a downloaded zip against its entry: the zip's SHA-256, then,
/// unpacked, the template's content hash, id, version and its publisher's
/// signature. Returns the unpacked template's folder (in the temp dir).
pub fn check_download(zip: &[u8], entry: &JobTemplateCatalogEntry) -> Result<(tempfile::TempDir, PathBuf), String> {
    if hex::encode(Sha256::digest(zip)) != entry.zip_sha256 {
        return Err(format!("{}'s download doesn't match the catalog", entry.id));
    }
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let zip_path = tmp.path().join("pkg.zip");
    std::fs::write(&zip_path, zip).map_err(|e| e.to_string())?;
    let stage = tmp.path().join("pkg");
    wp::unzip(&zip_path, &stage)?;
    let root = wp::package_root(&stage, TEMPLATE_FILE)?;
    let pkg = package::load_dir(&root, &entry.id)?;
    if pkg.hash != entry.hash {
        return Err(format!("{}'s files don't match the catalog", entry.id));
    }
    if pkg.manifest.version != entry.version {
        return Err(format!("{}'s template.json doesn't match the catalog", entry.id));
    }
    if pkg.signer.as_deref() != Some(entry.publisher_key.as_str()) {
        return Err(format!("{} isn't signed by the publisher the catalog lists", entry.id));
    }
    std::fs::remove_file(&zip_path).ok();
    Ok((tmp, root))
}

/// Downloads `entry` from its catalog, checks it, and installs it
/// (replacing an installed version), waiting for approval.
pub async fn install(templates: Arc<super::JobTemplates>, entry: &JobTemplateCatalogEntry) -> Result<String, String> {
    let zip = wc::get(&entry.zip, (MAX_BYTES as usize) * 4).await?;
    let entry = entry.clone();
    tokio::task::spawn_blocking(move || {
        let (_tmp, root) = check_download(&zip, &entry)?;
        templates.install(&root, true)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
