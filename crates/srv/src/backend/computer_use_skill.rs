// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading a computer-use skill folder (docs/specs/SPEC_PACKS_2026_11_05.md §3): `SKILL.md`, the
//! manifest `computer-use.json`, and the text files beside them. It only reads and checks; the caller
//! shows the person what was found and writes only what they approve.
//!
//! The checks are the public catalog's (the agentmuxai/skills repo, in its lib module), so a folder that passes
//! there reads here: the folder name is `<publisher>-<id>` and equals the `SKILL.md` name, the version
//! is semver, the last changelog entry is the current version, the ladder names only `api`, `mcp`,
//! `shortcuts` and `desktop`, and an MCP pointer names a catalog entry, never a command. The content
//! hash is the catalog's too: SHA-256 over `path\0filehash\n` in byte order of the path, leaving out
//! `skill.sig` and `verified.json`.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::backend::agent_config::AgentConfigFile;
use crate::backend::skill_import::parse_skill_md;
use crate::backend::skill_slugs::agent_skill_slugs;
use crate::backend::storage::skill_app::{LadderStep, ReferenceInfo, SkillAppInfo, SkillFileInput, VerifiedRecord};
use crate::backend::storage::store::Store;
use crate::backend::storage::AgentSkill;

pub const MANIFEST: &str = "computer-use.json";
pub const SIG_FILE: &str = "skill.sig";
pub const VERIFIED_FILE: &str = "verified.json";
/// The manifest `schema` this build reads.
pub const SCHEMA: &str = "computer-use-skill/v1";

const MAX_FILES: usize = 200;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 5 * 1024 * 1024;
const MAX_SKILL_MD_LINES: usize = 500;
/// Reference crops (SPEC_PACKS §3.1): at most this many, this large, this many pixels across.
const MAX_REFERENCES: usize = 40;
const MAX_REFERENCE_BYTES: usize = 100 * 1024;
const MAX_REFERENCE_W: u32 = 800;
const MAX_REFERENCE_H: u32 = 600;
/// PNG chunks a crop may carry. Text, time, EXIF and physical-size chunks can hold a name or a machine, so they are refused.
const PNG_CHUNKS: &[&str] = &["IHDR", "PLTE", "tRNS", "IDAT", "IEND", "sRGB", "gAMA", "cHRM"];
const TEXT_EXT: &[&str] = &["md", "json", "txt", "py", "ps1", "js", "mjs", "vbs", "jsx", "bat", "sh"];
const SCRIPT_EXT: &[&str] = &["py", "ps1", "js", "mjs", "vbs", "jsx", "bat", "sh"];
const PLATFORMS: &[&str] = &["windows", "macos", "linux"];
const LADDER_KINDS: &[&str] = &["api", "mcp", "shortcuts", "desktop"];

/// A skill folder, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The `SKILL.md` name, which is also the folder name.
    pub name: String,
    pub description: String,
    /// `SKILL.md` as written, frontmatter included: the content hash covers it.
    pub skill_md: String,
    /// The `SKILL.md` body without its frontmatter: what `db_skills.content` holds.
    pub content: String,
    pub info: SkillAppInfo,
    /// The manifest as written, kept so a later build can read fields this one ignores.
    pub manifest_json: String,
    pub content_hash: String,
    /// Every file but `SKILL.md`, the manifest and the signature: reference files, the test, scripts.
    pub files: Vec<SkillFileInput>,
    /// The publisher's signature (`skill.sig`) as written, if the folder carries one.
    pub signature: Option<String>,
    /// The pass records in `verified.json`, if the folder carries one.
    pub verified: Vec<VerifiedRecord>,
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.split('-').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

/// `1.2.3`, with an optional `-prerelease` and `+build`, each of letters, digits, dots and hyphens: the version ends up in
/// file names and a pass record's key, so nothing else may ride along after the numbers.
fn is_semver(s: &str) -> bool {
    let core = s.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    let tail = &s[core.len()..];
    parts.len() == 3
        && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && tail.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
}

fn ext_of(path: &str) -> String {
    path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// A path beside `SKILL.md` that can be stored and written under an agent's skill folder: relative,
/// forward slashes, no `.`/`..`/empty part, no drive or stream marker, at most four parts.
pub fn valid_rel_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 200 || path.starts_with('/') || path.contains('\\') || path.contains(':') || path.contains('\0') {
        return false;
    }
    let parts: Vec<&str> = path.split('/').collect();
    parts.len() <= 4 && parts.iter().all(|p| !p.is_empty() && *p != "." && *p != ".." && !p.ends_with('.') && !p.ends_with(' '))
}

/// A name the folder itself uses (`SKILL.md`, the manifest, the signature, the pass records), in any letter case.
/// On a case-insensitive disk a sibling called `skill.md` would overwrite the `SKILL.md` the person reviewed.
fn is_reserved_name(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    ["skill.md", MANIFEST, SIG_FILE, VERIFIED_FILE].contains(&lower.as_str())
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// The manifest's own fields. Strict about what an installed skill needs, quiet about extra fields.
pub fn info_from_manifest(text: &str) -> Result<SkillAppInfo, String> {
    let m: Value = serde_json::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let s = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    if m.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(format!("{MANIFEST}: schema must be {SCHEMA}"));
    }
    let publisher = s(&m, "publisher").filter(|p| is_name(p)).ok_or_else(|| format!("{MANIFEST}: publisher must be lowercase letters, digits and hyphens"))?;
    let id = s(&m, "id").filter(|p| is_name(p)).ok_or_else(|| format!("{MANIFEST}: id must be lowercase letters, digits and hyphens"))?;
    let name = s(&m, "name").filter(|n| !n.trim().is_empty()).ok_or_else(|| format!("{MANIFEST}: needs a name"))?;
    let version = s(&m, "version").filter(|v| is_semver(v)).ok_or_else(|| format!("{MANIFEST}: version must be semver"))?;
    let last = m.get("changelog").and_then(Value::as_array).and_then(|a| a.last()).and_then(|e| s(e, "version"));
    if last.as_deref() != Some(version.as_str()) {
        return Err(format!("{MANIFEST}: changelog must end with an entry for version {version}"));
    }
    let app = m.get("app").ok_or_else(|| format!("{MANIFEST}: app.vendor and app.executables are required"))?;
    let vendor = s(app, "vendor").ok_or_else(|| format!("{MANIFEST}: app.vendor and app.executables are required"))?;
    let exes = app.get("executables").and_then(Value::as_object).ok_or_else(|| format!("{MANIFEST}: app.vendor and app.executables are required"))?;
    let mut executables = BTreeMap::new();
    for (platform, list) in exes {
        if !PLATFORMS.contains(&platform.as_str()) {
            return Err(format!("{MANIFEST}: app.executables has an unknown platform {platform:?}"));
        }
        executables.insert(platform.clone(), str_list(Some(list)));
    }
    let platforms = str_list(m.get("platforms"));
    if platforms.is_empty() || platforms.iter().any(|p| !PLATFORMS.contains(&p.as_str())) {
        return Err(format!("{MANIFEST}: platforms must list windows, macos or linux"));
    }
    let mut ladder = Vec::new();
    for step in m.get("ladder").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
        match (s(step, "kind"), s(step, "note")) {
            (Some(kind), Some(note)) if LADDER_KINDS.contains(&kind.as_str()) => ladder.push(LadderStep { kind, note }),
            _ => return Err(format!("{MANIFEST}: ladder must list {{ kind: api | mcp | shortcuts | desktop, note }}")),
        }
    }
    if ladder.is_empty() {
        return Err(format!("{MANIFEST}: ladder must list {{ kind: api | mcp | shortcuts | desktop, note }}"));
    }
    // A catalog id, as a bare string or `{ catalogId, note }`; anything with a command in it is refused.
    let mcp = match m.get("mcp") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(v @ Value::Object(_)) => {
            if v.get("command").is_some() || v.get("args").is_some() {
                return Err(format!("{MANIFEST}: mcp is a catalog id, never a command line"));
            }
            // `catalogId: null` says the app has no entry in the MCP catalog yet.
            match v.get("catalogId") {
                None | Some(Value::Null) => None,
                Some(Value::String(id)) => Some(id.clone()),
                Some(_) => return Err(format!("{MANIFEST}: mcp.catalogId is a string or null")),
            }
        }
        Some(_) => return Err(format!("{MANIFEST}: mcp is a catalog id, never a command line")),
    };
    if mcp.as_deref().is_some_and(|id| id.chars().any(char::is_whitespace)) {
        return Err(format!("{MANIFEST}: mcp is a catalog id, never a command line"));
    }
    let versions = app.get("versions");
    let references = references_from(m.get("references"))?;
    Ok(SkillAppInfo {
        publisher,
        id,
        name,
        version,
        vendor,
        executables,
        title_hints: str_list(app.get("titleHints")),
        platforms,
        ladder,
        mcp,
        supported: versions.and_then(|v| s(v, "supported")),
        tested: str_list(versions.and_then(|v| v.get("tested"))),
        references,
    })
}

/// The manifest's `references`: every crop needs all its tags, or it is refused.
fn references_from(v: Option<&Value>) -> Result<Vec<ReferenceInfo>, String> {
    let Some(list) = v.and_then(Value::as_array) else { return Ok(Vec::new()) };
    if list.len() > MAX_REFERENCES {
        return Err(format!("{MANIFEST}: a skill may have at most {MAX_REFERENCES} reference crops"));
    }
    let mut out = Vec::new();
    for r in list {
        let s = |key: &str| r.get(key).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string);
        let path = s("path").unwrap_or_default();
        let bad = |what: &str| format!("{MANIFEST}: reference {path:?} needs {what}");
        if !path.starts_with("references/") || !path.ends_with(".png") || !valid_rel_path(&path) {
            return Err(format!("{MANIFEST}: a reference is a PNG under references/, not {path:?}"));
        }
        let platform = s("platform").filter(|p| PLATFORMS.contains(&p.as_str())).ok_or_else(|| bad("a platform (windows, macos or linux)"))?;
        let scale = r.get("scale").and_then(Value::as_u64).filter(|n| (1..=400).contains(n)).ok_or_else(|| bad("a scale in percent"))? as u32;
        let sha256 = s("sha256").filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).ok_or_else(|| bad("its sha256"))?.to_ascii_lowercase();
        out.push(ReferenceInfo {
            label: s("label").ok_or_else(|| bad("a label"))?,
            shows: s("shows").unwrap_or_default(),
            app_version: s("appVersion").ok_or_else(|| bad("the app version it was taken at"))?,
            theme: s("theme").ok_or_else(|| bad("a theme"))?,
            captured_by: s("capturedBy").ok_or_else(|| bad("capturedBy"))?,
            date: s("date").ok_or_else(|| bad("a date"))?,
            platform,
            scale,
            sha256,
            path,
        });
    }
    Ok(out)
}

/// Check a reference crop's bytes: a PNG within the size and pixel limits that carries no metadata chunk.
pub fn check_png(path: &str, bytes: &[u8]) -> Result<(), String> {
    const SIG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() > MAX_REFERENCE_BYTES {
        return Err(format!("{path}: over {} KB", MAX_REFERENCE_BYTES / 1024));
    }
    if !bytes.starts_with(SIG) {
        return Err(format!("{path}: not a PNG"));
    }
    let (mut at, mut first, mut ended) = (SIG.len(), true, false);
    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let kind = std::str::from_utf8(&bytes[at + 4..at + 8]).unwrap_or("????");
        let end = at.checked_add(12).and_then(|n| n.checked_add(len)).filter(|e| *e <= bytes.len()).ok_or_else(|| format!("{path}: truncated PNG"))?;
        if !PNG_CHUNKS.contains(&kind) {
            return Err(format!("{path}: carries a {kind} chunk; re-save the crop without metadata"));
        }
        if first {
            if kind != "IHDR" || len != 13 {
                return Err(format!("{path}: not a PNG"));
            }
            let w = u32::from_be_bytes(bytes[at + 8..at + 12].try_into().unwrap());
            let h = u32::from_be_bytes(bytes[at + 12..at + 16].try_into().unwrap());
            if w == 0 || h == 0 || w > MAX_REFERENCE_W || h > MAX_REFERENCE_H {
                return Err(format!("{path}: {w}x{h} pixels; a crop is at most {MAX_REFERENCE_W}x{MAX_REFERENCE_H}"));
            }
            first = false;
        }
        at = end;
        if kind == "IEND" {
            ended = true;
            break;
        }
    }
    if !ended || at != bytes.len() {
        return Err(format!("{path}: not a complete PNG"));
    }
    Ok(())
}

/// The scripts a manifest lists: path, and the SHA-256 it states.
fn listed_scripts(manifest: &Value) -> BTreeMap<String, String> {
    manifest
        .get("scripts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| Some((s.get("path")?.as_str()?.to_string(), s.get("sha256")?.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// What each listed script is for and needs, for the approval prompt.
pub fn script_infos(manifest_json: &str) -> Vec<crate::backend::storage::skill_app::ScriptInfo> {
    let Ok(m) = serde_json::from_str::<Value>(manifest_json) else { return Vec::new() };
    m.get("scripts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(crate::backend::storage::skill_app::ScriptInfo {
                        path: s.get("path")?.as_str()?.to_string(),
                        purpose: s.get("purpose").and_then(Value::as_str).unwrap_or("").to_string(),
                        needs: str_list(s.get("needs")),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The catalog's content hash of `files` (path to bytes), without `skill.sig` and `verified.json`.
pub fn content_hash(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut h = Sha256::new();
    // BTreeMap<String, _> iterates in byte order of the key, the order the catalog signs in.
    for (path, bytes) in files.iter().filter(|(p, _)| p.as_str() != SIG_FILE && p.as_str() != VERIFIED_FILE) {
        h.update(path.as_bytes());
        h.update(b"\0");
        h.update(sha256_hex(bytes).as_bytes());
        h.update(b"\n");
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn read_files(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    fn walk(abs: &Path, rel: &str, out: &mut BTreeMap<String, Vec<u8>>, total: &mut u64) -> Result<(), String> {
        let mut names: Vec<_> = std::fs::read_dir(abs).map_err(|e| format!("{}: {e}", abs.display()))?.flatten().collect();
        names.sort_by_key(|e| e.file_name());
        for entry in names {
            let name = entry.file_name().to_string_lossy().to_string();
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let meta = std::fs::symlink_metadata(entry.path()).map_err(|e| format!("{r}: {e}"))?;
            if meta.file_type().is_symlink() {
                return Err(format!("{r} is a link; a skill can't contain links"));
            }
            if meta.is_dir() {
                walk(&entry.path(), &r, out, total)?;
                continue;
            }
            if meta.len() > MAX_FILE_BYTES {
                return Err(format!("{r} is over 1 MB"));
            }
            *total += meta.len();
            if *total > MAX_TOTAL_BYTES {
                return Err("the skill is over 5 MB".to_string());
            }
            out.insert(r.clone(), std::fs::read(entry.path()).map_err(|e| format!("{r}: {e}"))?);
            if out.len() > MAX_FILES {
                return Err(format!("the skill has over {MAX_FILES} files"));
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(dir, "", &mut out, &mut 0)?;
    Ok(out)
}

/// Read and check the skill folder `dir`. The folder's name must be `<publisher>-<id>`.
pub fn read_folder(dir: &Path) -> Result<Bundle, String> {
    let folder = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let all = read_files(dir)?;
    let text = |name: &str| -> Result<Option<String>, String> {
        all.get(name).map(|b| String::from_utf8(b.clone()).map_err(|_| format!("{name} is not UTF-8 text"))).transpose()
    };
    for path in all.keys() {
        if !valid_rel_path(path) {
            return Err(format!("{path}: not a path a skill can hold"));
        }
        if is_reserved_name(path) && !matches!(path.as_str(), "SKILL.md" | MANIFEST | SIG_FILE | VERIFIED_FILE) {
            return Err(format!("{path}: a name the skill folder uses for itself, in other letters; rename it"));
        }
        let is_crop = path.starts_with("references/") && ext_of(path) == "png";
        if !is_crop && !TEXT_EXT.contains(&ext_of(path).as_str()) && path != SIG_FILE {
            return Err(format!("{path}: only text and script files are accepted ({}), and PNG crops under references/", TEXT_EXT.join(" ")));
        }
    }
    let skill_md = text("SKILL.md")?.ok_or("no SKILL.md")?;
    let (name, description, content) = parse_skill_md(&skill_md, &folder);
    if name != folder {
        return Err(format!("SKILL.md name ({name}) must match the folder ({folder})"));
    }
    if description.is_empty() || description.chars().count() > 1024 {
        return Err("SKILL.md needs a description of 1 to 1024 characters".to_string());
    }
    if skill_md.lines().count() > MAX_SKILL_MD_LINES {
        return Err(format!("SKILL.md is over {MAX_SKILL_MD_LINES} lines: move detail into reference files"));
    }

    let manifest_json = text(MANIFEST)?.ok_or_else(|| format!("no {MANIFEST}"))?;
    let info = info_from_manifest(&manifest_json)?;
    if format!("{}-{}", info.publisher, info.id) != folder {
        return Err(format!("the folder must be {}-{}", info.publisher, info.id));
    }
    let manifest: Value = serde_json::from_str(&manifest_json).map_err(|e| format!("{MANIFEST}: {e}"))?;

    // Every script is listed with the hash it has; a listed file must be there.
    let listed = listed_scripts(&manifest);
    let mut files = Vec::new();
    for (path, bytes) in &all {
        if matches!(path.as_str(), "SKILL.md" | MANIFEST | SIG_FILE | VERIFIED_FILE) {
            continue;
        }
        if path.starts_with("references/") && ext_of(path) == "png" {
            check_png(path, bytes)?;
            let tagged = info.references.iter().find(|r| &r.path == path).ok_or_else(|| format!("{path} is a crop: list it in {MANIFEST} \"references\" with its tags"))?;
            if tagged.sha256 != sha256_hex(bytes) {
                return Err(format!("{path}: sha256 in {MANIFEST} doesn't match the file"));
            }
            files.push(SkillFileInput { path: path.clone(), content: String::new(), image: Some(bytes.clone()), script: false });
            continue;
        }
        let script = SCRIPT_EXT.contains(&ext_of(path).as_str());
        if script {
            match listed.get(path) {
                None => return Err(format!("{path} is a script: list it in {MANIFEST} \"scripts\" with its sha256, purpose and needs")),
                Some(h) if *h != sha256_hex(bytes) => return Err(format!("{path}: sha256 in {MANIFEST} doesn't match the file")),
                Some(_) => {}
            }
        }
        let content = String::from_utf8(bytes.clone()).map_err(|_| format!("{path} is not UTF-8 text"))?;
        files.push(SkillFileInput { path: path.clone(), content, image: None, script });
    }
    for r in &info.references {
        if !all.contains_key(&r.path) {
            return Err(format!("{MANIFEST} lists the reference {}, which isn't in the folder", r.path));
        }
    }
    for path in listed.keys() {
        if !all.contains_key(path) {
            return Err(format!("{MANIFEST} lists {path}, which isn't in the folder"));
        }
    }

    let signature = text(SIG_FILE)?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let verified = match text(VERIFIED_FILE)? {
        None => Vec::new(),
        Some(t) => serde_json::from_str::<Vec<VerifiedRecord>>(&t).map_err(|e| format!("{VERIFIED_FILE}: {e}"))?,
    };
    Ok(Bundle { name, description, skill_md, content, info, manifest_json, content_hash: content_hash(&all), files, signature, verified })
}

/// Whether `exe` (an executable name as `Desktop apps` lists it) is one this skill's app runs as on `platform`.
/// Compared by file name, ignoring case and a folder part, never by window title alone.
pub fn matches_executable(info: &SkillAppInfo, platform: &str, exe: &str) -> bool {
    // `Desktop` may name an app with or without `.exe`; the manifest lists it either way.
    let base = |s: &str| {
        let name = s.rsplit(['/', '\\']).next().unwrap_or(s).to_ascii_lowercase();
        name.strip_suffix(".exe").map(str::to_string).unwrap_or(name)
    };
    let want = base(exe);
    !want.is_empty() && info.executables.get(platform).is_some_and(|list| list.iter().any(|e| base(e) == want))
}

/// What an import preview read, kept until the person approves it, so the commit writes exactly what they saw
/// (by content hash) and not a second read of a source that may have changed. At most 16, each for 30 minutes.
static STASH: std::sync::Mutex<Vec<(std::time::Instant, Bundle)>> = std::sync::Mutex::new(Vec::new());
const STASH_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const STASH_MAX: usize = 16;

/// Keep `bundle` for the commit that follows its preview.
pub fn stash(bundle: &Bundle) {
    let mut s = STASH.lock().unwrap();
    s.retain(|(at, b)| at.elapsed() < STASH_TTL && b.content_hash != bundle.content_hash);
    s.push((std::time::Instant::now(), bundle.clone()));
    while s.len() > STASH_MAX {
        s.remove(0);
    }
}

/// The bundle a preview read, by its content hash, if it is still kept. It stays, so a failed commit can be retried.
pub fn stashed(content_hash: &str) -> Option<Bundle> {
    let s = STASH.lock().unwrap();
    s.iter().find(|(at, b)| at.elapsed() < STASH_TTL && b.content_hash == content_hash).map(|(_, b)| b.clone())
}

/// The key that signed this bundle: `Ok(None)` for an unsigned one, an error for a signature that does not match
/// its files. A publisher pinned to another key, or pinned and now unsigned, is refused (`SPEC_PACKS` §5): a
/// different key than the one the person approved a script of is not an update, it is a different author.
pub fn check_signature(catalog: &Store, b: &Bundle) -> Result<Option<String>, String> {
    use crate::backend::widget::signature as sig;
    let id = format!("{}.{}", b.info.publisher, b.info.id);
    let signer = match &b.signature {
        Some(text) => Some(sig::verify_text(&sig::COMPUTER_USE_SKILL, text, &id, &b.info.version, &b.content_hash).map_err(|e| format!("{id}: {e}"))?),
        None => None,
    };
    let pins = catalog.skill_publisher_pins().map_err(|e| e.to_string())?;
    if let Some(pinned) = pins.get(&b.info.publisher) {
        if signer.as_deref() != Some(pinned.as_str()) {
            let said = signer.as_deref().map_or("not signed".to_string(), |k| format!("signed by {}", sig::fingerprint(k)));
            return Err(format!("{id} is {said}, but {} is pinned to {}: that is a different author, not an update", b.info.publisher, sig::fingerprint(pinned)));
        }
    }
    Ok(signer)
}

/// Write an approved bundle's computer-use part for the catalog skill `skill_id`: the manifest, the files, and the pass
/// records its folder carried. A script lands unapproved. Call after the skill row itself is written.
/// `signer` is what `check_signature` returned for this bundle: the caller checks first and a refusal never gets here.
pub fn install(
    catalog: &Store,
    skill_id: &str,
    b: &Bundle,
    signer: Option<&str>,
    now_ms: i64,
) -> Result<(), crate::backend::storage::error::StoreError> {
    catalog.skill_app_set(skill_id, &b.info, &b.manifest_json, &b.content_hash, &b.files, now_ms)?;
    catalog.skill_signer_set(skill_id, signer)?;
    catalog.skill_origin_set(skill_id, &b.skill_md, b.signature.as_deref())?;
    for r in &b.verified {
        catalog.skill_verified_append(skill_id, r, "import", now_ms)?;
    }
    Ok(())
}

/// The computer-use skill for a window, as `Desktop apps` points at it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SkillPointer {
    pub id: String,
    pub name: String,
    pub version: String,
    /// At least one test of this installed version passed on this platform.
    pub verified_here: bool,
}

/// The skill whose app runs as `exe` on `platform`, if the catalog has one. No grant is implied by the pointer.
pub fn skill_for_executable(catalog: &Store, platform: &str, exe: &str) -> Option<SkillPointer> {
    let apps = catalog.skill_app_map().ok()?;
    let (id, app) = apps.into_iter().find(|(_, a)| matches_executable(&a.info, platform, exe))?;
    let name = catalog.skill_get(&id).ok().flatten()?.name;
    let verified_here = app.verified.iter().any(|r| r.platform == platform && r.result == "pass" && r.skill_version == app.info.version);
    Some(SkillPointer { id, name, version: app.info.version, verified_here })
}

/// The files an agent's launch writes beside each computer-use skill's `SKILL.md`
/// (`.claude/skills/<slug>/<path>`), for the skills `build_config_files` gave a folder. A script is
/// written only once a person has approved it. A reference crop is not written at all: an image is read
/// with `Skills get` and `file`, so it costs nothing until a step points at it. Raw text: unlike `SKILL.md`, no `{{VARIABLE}}` is expanded,
/// since a script or a reference file may hold `{{` of its own. The `.claude/skills/` prefix makes them
/// managed files, so the launch's stale-file cleanup removes the ones a skill no longer has.
pub fn sibling_files(skills: &[AgentSkill], catalog: &Store) -> Vec<AgentConfigFile> {
    let mut out = Vec::new();
    for (skill, slug) in skills.iter().zip(agent_skill_slugs(skills)) {
        let Some(slug) = slug else { continue };
        for f in catalog.skill_files(&skill.id).unwrap_or_default() {
            if (f.script && !f.approved) || f.image.is_some() || !valid_rel_path(&f.path) || is_reserved_name(&f.path) {
                continue;
            }
            out.push(AgentConfigFile { filename: format!(".claude/skills/{slug}/{}", f.path), content: f.content });
        }
    }
    out
}

#[cfg(test)]
#[path = "computer_use_skill_tests.rs"]
pub(crate) mod tests;
