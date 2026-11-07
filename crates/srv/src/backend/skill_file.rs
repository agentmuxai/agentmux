// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A computer-use skill as one file (docs/specs/SPEC_PACKS_2026_11_05.md §5 and §10.5): a zip with the
//! extension `.skill` that holds the skill's folder, named `<publisher>-<id>`, and nothing else. Exporting
//! writes the folder the catalog holds; importing unpacks it into a scratch folder and reads it like any
//! other folder (`computer_use_skill::read_folder`), so every check a folder gets, a `.skill` gets.
//!
//! The reader trusts nothing in the zip: names are checked before anything is written, and the sizes are
//! counted as they are read, not taken from the headers. Scripts always arrive unapproved, so a `.skill`
//! installs, never approves.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Component, Path};

use crate::backend::computer_use_skill::{content_hash, MANIFEST, SIG_FILE, VERIFIED_FILE};
use crate::backend::skill_import::parse_skill_md;
use crate::backend::storage::skill_app::SkillApp;
use crate::backend::storage::store::Store;
use crate::backend::storage::Skill;

/// The extension of the file.
pub const EXTENSION: &str = "skill";

const MAX_ENTRIES: usize = 220;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 5 * 1024 * 1024;
/// The zip itself: the files compress, so this is a little over the uncompressed total.
pub(crate) const MAX_ZIP_BYTES: u64 = 6 * 1024 * 1024;

/// An exported skill, ready to write.
pub struct Exported {
    /// `<publisher>-<id>-<version>.skill`.
    pub filename: String,
    pub bytes: Vec<u8>,
    /// Whether the file carries the publisher's signature. It does only while the skill is exactly what was signed.
    pub signed: bool,
}

/// `SKILL.md` for a skill whose installed text is no longer what it came with (the person edited it in Memory),
/// or that was installed before the original was kept. The description is written as a JSON string, which is a
/// valid double-quoted value in the frontmatter whatever it holds.
fn rebuilt_skill_md(folder: &str, skill: &Skill) -> String {
    let description = serde_json::to_string(&skill.description).unwrap_or_else(|_| "\"\"".to_string());
    format!("---\nname: {folder}\ndescription: {description}\n---\n\n{}", skill.content)
}

/// Whether the `SKILL.md` kept from the install still says what the catalog skill says now.
fn original_still_applies(folder: &str, original: &str, skill: &Skill) -> bool {
    let (name, description, content) = parse_skill_md(original, folder);
    name == folder && description == skill.description && content == skill.content
}

fn verified_json(app: &SkillApp) -> Result<Option<Vec<u8>>, String> {
    if app.verified.is_empty() {
        return Ok(None);
    }
    serde_json::to_vec_pretty(&app.verified).map(Some).map_err(|e| format!("{VERIFIED_FILE}: {e}"))
}

/// The catalog skill `skill_id` as a `.skill` file.
pub fn export(catalog: &Store, skill_id: &str) -> Result<Exported, String> {
    let skill = catalog.skill_get(skill_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no skill {skill_id}"))?;
    let app = catalog.skill_app_get(skill_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("{} is not a computer-use skill", skill.name))?;
    let manifest = catalog.skill_manifest_get(skill_id).map_err(|e| e.to_string())?.ok_or_else(|| format!("{} has no manifest", skill.name))?;
    let origin = catalog.skill_origin_get(skill_id).map_err(|e| e.to_string())?;
    let folder = format!("{}-{}", app.info.publisher, app.info.id);

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for f in catalog.skill_files(skill_id).map_err(|e| e.to_string())? {
        files.insert(f.path, f.image.unwrap_or_else(|| f.content.into_bytes()));
    }
    files.insert(MANIFEST.to_string(), manifest.into_bytes());
    let skill_md = match &origin {
        Some((md, _)) if original_still_applies(&folder, md, &skill) => md.clone(),
        _ => rebuilt_skill_md(&folder, &skill),
    };
    files.insert("SKILL.md".to_string(), skill_md.into_bytes());

    // The publisher's signature goes along only when what is being written hashes to what was signed.
    let signature = origin.and_then(|(_, sig)| sig).filter(|_| content_hash(&files) == app.content_hash);
    let signed = signature.is_some();
    if let Some(sig) = signature {
        files.insert(SIG_FILE.to_string(), sig.into_bytes());
    }
    if let Some(v) = verified_json(&app)? {
        files.insert(VERIFIED_FILE.to_string(), v);
    }

    let total: u64 = files.values().map(|b| b.len() as u64).sum();
    if total > MAX_TOTAL_BYTES {
        return Err(format!("{} is over {} MB: too large to share as one file", skill.name, MAX_TOTAL_BYTES / 1024 / 1024));
    }
    Ok(Exported { filename: file_name(&folder, &app.info.version), bytes: zip_folder(&folder, &files)?, signed })
}

/// `<folder>-<version>.skill`, with the version cut down to the characters a version has: it comes from a manifest, and a
/// file name is no place for whatever else it might hold.
fn file_name(folder: &str, version: &str) -> String {
    let mut version: String = version.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')).collect();
    while version.contains("..") {
        version = version.replace("..", ".");
    }
    format!("{folder}-{version}.{EXTENSION}")
}

fn zip_folder(folder: &str, files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (path, bytes) in files {
        zip.start_file(format!("{folder}/{path}"), options).map_err(|e| format!("{path}: {e}"))?;
        zip.write_all(bytes).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Unpack the `.skill` at `zip_path` into `dest`, which is left holding one folder. Every entry is checked and
/// read before anything is written, so a file that fails a check leaves `dest` as it was.
pub fn extract(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let size = std::fs::metadata(zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?.len();
    if size > MAX_ZIP_BYTES {
        return Err(format!("{} is over {} MB: not a skill file this build reads", zip_path.display(), MAX_ZIP_BYTES / 1024 / 1024));
    }
    let file = std::fs::File::open(zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a .{EXTENSION} file (a zip of one skill folder): {e}"))?;
    if zip.len() > MAX_ENTRIES {
        return Err(format!("the file holds {} entries; a skill has at most {MAX_ENTRIES}", zip.len()));
    }
    let mut top: Option<String> = None;
    let mut staged: Vec<(String, Vec<u8>)> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(format!("{} is a symbolic link", entry.name()));
        }
        let rel = entry.enclosed_name().ok_or_else(|| format!("{} is not a path inside the folder", entry.name()))?;
        let mut parts = Vec::new();
        for c in rel.components() {
            match c {
                Component::Normal(p) => parts.push(p.to_string_lossy().to_string()),
                _ => return Err(format!("{} is not a path inside the folder", entry.name())),
            }
        }
        let Some(first) = parts.first().cloned() else { continue };
        match &top {
            None => top = Some(first.clone()),
            Some(t) if *t != first => return Err("a .skill file holds one folder; this one holds more".to_string()),
            Some(_) => {}
        }
        if entry.is_dir() {
            continue;
        }
        if parts.len() < 2 {
            return Err(format!("{} sits outside the skill's folder", entry.name()));
        }
        if !seen.insert(parts.join("/").to_lowercase()) {
            return Err(format!("{} appears twice (or differs from another entry only by letter case)", entry.name()));
        }
        // Counted as read: a header's size is the sender's word.
        let mut bytes = Vec::new();
        entry.by_ref().take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", entry.name()))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("{} is over {} MB", entry.name(), MAX_FILE_BYTES / 1024 / 1024));
        }
        total += bytes.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(format!("the files add up to over {} MB", MAX_TOTAL_BYTES / 1024 / 1024));
        }
        staged.push((parts.join("/"), bytes));
    }
    if top.is_none() {
        return Err("the file is empty".to_string());
    }
    for (path, bytes) in staged {
        let target = dest.join(&path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "skill_file_tests.rs"]
mod tests;
