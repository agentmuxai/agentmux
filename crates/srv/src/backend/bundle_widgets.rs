// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Widgets in agent bundles (docs/specs/SPEC_WIDGET_SHARING_2026_10_10.md §3).
//!
//! An ABF v0.4 bundle may carry whole widget packages under `widgets/<id>/`,
//! binary files included, and list them in `bundle.json` as
//! `components.widgets: [{ id, version, hash }]`. Importing the bundle offers
//! them; it never approves one: each is copied into the widgets folder and
//! asks for the user's approval like a widget installed from a folder.
//! Bundles carry sandboxed widgets only.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use super::widget_packages::{self as wp, Manifest, WidgetKind, WidgetPackageInfo, WidgetState};
use super::widget_signature::{self as sig, WidgetSignatureInfo};

/// Where a bundle keeps its widgets, relative to the bundle's root.
pub const WIDGETS_DIR: &str = "widgets";

/// Is `path` (bundle-relative, as the text importer sees it) part of a widget?
pub fn is_widget_path(path: &str) -> bool {
    path.strip_prefix(WIDGETS_DIR).is_some_and(|rest| rest.starts_with('/'))
}

/// One widget package read out of a bundle.
#[derive(Debug, Clone, Default)]
pub struct BundleWidget {
    pub id: String,
    /// Package-relative path → bytes.
    pub files: BTreeMap<String, Vec<u8>>,
}

/// The widgets in a bundle zip: every entry under `[<wrapper>/]widgets/<id>/`.
/// The text importer has already bounded the archive (entry count, sizes);
/// this holds each widget to the widget limits too.
pub fn read_from_zip(zip_bytes: &[u8]) -> Result<Vec<BundleWidget>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| format!("not a valid zip archive: {e}"))?;
    let names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().filter(|e| !e.is_dir()).map(|e| e.name().to_string()))
        .collect();
    // The same wrapper rule as the text importer: strip a first folder only
    // when every entry shares it.
    let wrapper = names.first().and_then(|n| n.split_once('/').map(|(w, _)| w.to_string())).filter(|w| {
        names.iter().all(|n| n.split_once('/').is_some_and(|(r, _)| r == w))
    });
    let mut out: BTreeMap<String, BundleWidget> = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("zip archive: entry {i}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let rel = match &wrapper {
            Some(w) => name.strip_prefix(w.as_str()).and_then(|r| r.strip_prefix('/')).unwrap_or(&name).to_string(),
            None => name.clone(),
        };
        let Some(rest) = rel.strip_prefix(WIDGETS_DIR).and_then(|r| r.strip_prefix('/')) else { continue };
        let Some((id, path)) = rest.split_once('/') else {
            return Err(format!("{name}: a widget's files go in widgets/<id>/"));
        };
        if !wp::valid_id(id) {
            return Err(format!("{name}: {id:?} isn't a widget id (publisher.name)"));
        }
        if path.is_empty() || path.contains('\\') || !Path::new(path).components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(format!("{name}: not a path inside the widget"));
        }
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(format!("{name} is a link; a widget can't contain links"));
        }
        let w = out.entry(id.to_string()).or_insert_with(|| BundleWidget { id: id.to_string(), ..Default::default() });
        let used: u64 = w.files.values().map(|b| b.len() as u64).sum();
        let mut bytes = Vec::new();
        (&mut entry)
            .take(wp::MAX_PACKAGE_BYTES - used + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{name}: {e}"))?;
        if used + bytes.len() as u64 > wp::MAX_PACKAGE_BYTES {
            return Err(format!("widget {id} is over {} MB", wp::MAX_PACKAGE_BYTES / (1024 * 1024)));
        }
        w.files.insert(path.to_string(), bytes);
        if w.files.len() > wp::MAX_PACKAGE_FILES {
            return Err(format!("widget {id} has over {} files", wp::MAX_PACKAGE_FILES));
        }
    }
    Ok(out.into_values().collect())
}

/// `w` written out as a package folder `<tmp>/<id>/`, for checking and
/// installing with the widget code's own rules.
pub fn stage(w: &BundleWidget) -> Result<(tempfile::TempDir, std::path::PathBuf), String> {
    let tmp = tempfile::tempdir().map_err(|e| format!("can't make a staging folder: {e}"))?;
    let root = tmp.path().join(&w.id);
    for (path, bytes) in &w.files {
        let full = root.join(path);
        if let Some(dir) = full.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&full, bytes).map_err(|e| format!("can't stage {path}: {e}"))?;
    }
    Ok((tmp, root))
}

/// What the import preview shows for one widget.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BundleImportWidgetPreview {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub kind: WidgetKind,
    pub permissions: Vec<String>,
    pub hash: String,
    pub signature: Option<WidgetSignatureInfo>,
    /// The version installed here now, if any, and its state.
    pub installed_version: Option<String>,
    pub installed_state: Option<WidgetState>,
    /// Installed here at exactly these files: importing changes nothing.
    pub same_as_installed: bool,
    /// Why it can't be imported; it is then not offered.
    pub error: Option<String>,
}

/// `components.widgets` in `bundle.json`: id → declared hash.
pub fn declared(manifest_json: Option<&str>) -> BTreeMap<String, String> {
    #[derive(Deserialize)]
    struct Entry {
        id: String,
        #[serde(default)]
        hash: String,
    }
    manifest_json
        .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
        .and_then(|v| v.pointer("/components/widgets").cloned())
        .and_then(|v| serde_json::from_value::<Vec<Entry>>(v).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id, e.hash))
        .collect()
}

/// Checks `w` as the widget code would, against what the bundle declares
/// and what is installed here.
pub fn preview(
    w: &BundleWidget,
    declared: &BTreeMap<String, String>,
    installed: &[WidgetPackageInfo],
    describe_signature: impl Fn(&str, Option<&str>) -> WidgetSignatureInfo,
) -> BundleImportWidgetPreview {
    let mut p = BundleImportWidgetPreview {
        id: w.id.clone(),
        name: w.id.clone(),
        version: String::new(),
        description: None,
        author: None,
        kind: WidgetKind::Sandboxed,
        permissions: vec![],
        hash: String::new(),
        signature: None,
        installed_version: None,
        installed_state: None,
        same_as_installed: false,
        error: None,
    };
    let checked = (|| -> Result<(), String> {
        let (_tmp, root) = stage(w)?;
        let m: Manifest = serde_json::from_slice(w.files.get(wp::MANIFEST_FILE).ok_or("it has no widget.json")?)
            .map_err(|e| format!("widget.json: {e}"))?;
        wp::validate(&m, &w.id)?;
        p.name = m.name.clone();
        p.version = m.version.clone();
        p.description = m.description.clone();
        p.author = m.author.clone();
        p.kind = m.kind();
        p.permissions = m.permissions.clone();
        let hash = wp::package_hash(&wp::hash_files(&root)?);
        p.hash = hash.clone();
        if m.kind() != WidgetKind::Sandboxed {
            return Err("bundles can carry sandboxed widgets only".to_string());
        }
        match declared.get(&w.id) {
            None => return Err("bundle.json doesn't list it in components.widgets".to_string()),
            Some(h) if *h != hash => return Err("its files don't match the hash bundle.json lists for it".to_string()),
            _ => {}
        }
        let signer = match sig::check(&root, &m.id, &m.version, &hash) {
            None => None,
            Some(Ok(k)) => Some(k),
            Some(Err(e)) => return Err(e),
        };
        p.signature = Some(describe_signature(&m.id, signer.as_deref()));
        Ok(())
    })();
    if let Some(i) = installed.iter().find(|i| i.id == w.id) {
        p.installed_version = Some(i.version.clone());
        p.installed_state = Some(i.state.clone());
        p.same_as_installed = !p.hash.is_empty()
            && i.hash == p.hash
            && i.signature.fingerprint == p.signature.as_ref().and_then(|s| s.fingerprint.clone());
    }
    p.error = checked.err();
    p
}

/// Copies `w` into the widgets folder, replacing an installed version.
pub fn install(widgets_dir: &Path, w: &BundleWidget) -> Result<String, String> {
    let (_tmp, root) = stage(w)?;
    wp::install(widgets_dir, &root, true)
}

/// A bundle export as a zip, with `widgets` (each an approved package's id,
/// content hash and files) under `widgets/<id>/` and listed in
/// `bundle.json`'s `components.widgets` (SPEC_WIDGET_SHARING_2026_10_10.md §3.4).
pub fn zip_export_with_widgets(
    export: &super::bundle_export::BundleExport,
    widgets: &[(String, String, BTreeMap<String, Vec<u8>>)],
) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let mut buf = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buf);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let listed: Vec<serde_json::Value> = widgets
        .iter()
        .map(|(id, hash, files)| {
            let version = files
                .get(wp::MANIFEST_FILE)
                .and_then(|m| serde_json::from_slice::<serde_json::Value>(m).ok())
                .and_then(|m| m.get("version").and_then(|v| v.as_str()).map(str::to_string))
                .unwrap_or_default();
            serde_json::json!({ "id": id, "version": version, "hash": hash })
        })
        .collect();
    let mut put = |path: String, bytes: &[u8]| -> Result<(), String> {
        writer.start_file(path, options).map_err(|e| format!("zip: {e}"))?;
        writer.write_all(bytes).map_err(|e| format!("zip: {e}"))
    };
    for file in &export.files {
        let path = format!("{}/{}", export.root_slug, file.path);
        if file.path == super::bundle_import::MANIFEST_FILE {
            let mut manifest: serde_json::Value = serde_json::from_str(&file.content).map_err(|e| format!("bundle.json: {e}"))?;
            manifest["components"]["widgets"] = serde_json::Value::Array(listed.clone());
            put(path, serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?.as_bytes())?;
        } else {
            put(path, file.content.as_bytes())?;
        }
    }
    for (id, _, files) in widgets {
        for (path, bytes) in files {
            put(format!("{}/{WIDGETS_DIR}/{id}/{path}", export.root_slug), bytes)?;
        }
    }
    drop(put);
    writer.finish().map_err(|e| format!("zip: {e}"))?;
    Ok(buf.into_inner())
}

#[cfg(test)]
#[path = "bundle_widgets_tests.rs"]
mod tests;
