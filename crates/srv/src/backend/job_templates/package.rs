// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A template package, in a folder or compiled in: its files, its content
//! hash and its signature (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md
//! §5.4).
//!
//! A template is data, so a package holds only its known files, at its top
//! level. The content hash is the widgets' (`widget_packages::package_hash`)
//! over every file except `template.sig`, which signs it.

use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::manifest::{self, JobTemplateManifest};
use crate::backend::widget_packages as wp;
use crate::backend::widget_signature as sig;

pub const TEMPLATE_FILE: &str = "template.json";
pub const BRIEF_FILE: &str = "brief.md";
const GRAPH_FILE: &str = "graph.json";
const README_FILE: &str = "README.md";
const FILES: &[&str] = &[TEMPLATE_FILE, BRIEF_FILE, GRAPH_FILE, README_FILE];
pub const MAX_BYTES: u64 = 256 * 1024;

/// A template, read and checked.
#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: JobTemplateManifest,
    pub brief: Option<String>,
    pub hash: String,
    /// The key (base64) whose `template.sig` verifies, if it is signed.
    pub signer: Option<String>,
}

/// Parse and check a template from its files (path → bytes), its signature
/// left out; `folder` is the name of the folder it is in.
pub fn parse(folder: &str, files: &BTreeMap<String, Vec<u8>>) -> Result<Package, String> {
    if let Some(other) = files.keys().find(|p| !FILES.contains(&p.as_str())) {
        return Err(format!(
            "a template holds only {}, and {}; {other:?} isn't one of them",
            FILES.join(", "),
            sig::JOB_TEMPLATE.sig_file
        ));
    }
    if files.contains_key(GRAPH_FILE) {
        return Err("graph.json (a Drone template) needs a later version of AgentMux".to_string());
    }
    if files.values().map(|b| b.len() as u64).sum::<u64>() > MAX_BYTES {
        return Err(format!("a template is at most {} KB", MAX_BYTES / 1024));
    }
    let raw = files.get(TEMPLATE_FILE).ok_or("no template.json")?;
    let m: JobTemplateManifest = serde_json::from_slice(raw).map_err(|e| format!("template.json: {e}"))?;
    let brief = files
        .get(BRIEF_FILE)
        .map(|b| String::from_utf8(b.clone()).map_err(|_| "brief.md isn't UTF-8 text".to_string()))
        .transpose()?;
    manifest::validate(&m, folder, brief.as_deref())?;
    let hashes = files.iter().map(|(p, b)| (p.clone(), hex::encode(Sha256::digest(b)))).collect();
    Ok(Package { manifest: m, brief, hash: wp::package_hash(&hashes), signer: None })
}

/// The files of the template folder `dir`, without its signature: no links,
/// no folders, within the size limit.
pub fn read_files(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut out = BTreeMap::new();
    let mut total = 0u64;
    let entries = std::fs::read_dir(dir).map_err(|e| format!("can't read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let ft = entry.file_type().map_err(|e| e.to_string())?;
        if ft.is_symlink() {
            return Err(format!("{name} is a link; a template can't contain links"));
        }
        if ft.is_dir() {
            return Err(format!("{name}/ is a folder; a template holds only files"));
        }
        if name == sig::JOB_TEMPLATE.sig_file {
            continue;
        }
        total += entry.metadata().map_err(|e| e.to_string())?.len();
        if total > MAX_BYTES {
            return Err(format!("a template is at most {} KB", MAX_BYTES / 1024));
        }
        out.insert(name.clone(), std::fs::read(entry.path()).map_err(|e| format!("can't read {name}: {e}"))?);
    }
    Ok(out)
}

/// The template in `dir`, checked as if its folder were named `folder`,
/// with its signature verified.
pub fn load_dir(dir: &Path, folder: &str) -> Result<Package, String> {
    let mut pkg = parse(folder, &read_files(dir)?)?;
    pkg.signer = match sig::check(&sig::JOB_TEMPLATE, dir, &pkg.manifest.id, &pkg.manifest.version, &pkg.hash) {
        None => None,
        Some(Ok(key)) => Some(key),
        Some(Err(e)) => return Err(e),
    };
    Ok(pkg)
}

/// A compiled-in template: never signed, it is part of the app.
pub fn load_embedded(id: &str, files: &[(&str, &[u8])]) -> Result<Package, String> {
    parse(id, &files.iter().map(|(p, b)| (p.to_string(), b.to_vec())).collect())
}
