// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Export to and import from the encrypted `.vault` file
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8).
//!
//! A `.vault` file is an age file encrypted to a passphrase (age's scrypt
//! recipient), whose plaintext is the JSON in `docs/api/VAULT_FILE_FORMAT.md`.
//! `age -d backup.vault` opens it without AgentMux.
//!
//! Only the person exports or imports: these functions are reached through the
//! host, which picks the file in its native dialog, so no agent chooses a path.
//! An import is two steps: `inspect` reads and decrypts the file into memory and
//! describes it with no values; `commit` writes it, in one transaction.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use age::secrecy::SecretString;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::import::{self, Incoming, Mapping, MappingNeeded, ReadError, Source};
use super::kinds::{self, VaultSection};
use super::seal::{self, Parts};
use super::{new_id, record, validate, PERSON};
use crate::backend::storage::store::Store;
use crate::backend::storage::vault::VaultRow;
use agentmux_common::time::now_ms;

pub const FORMAT: &str = "agentmux-vault";
pub const VERSION: u32 = 1;

/// §8.7: a file over this is refused before it is read.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
/// A passphrase shorter than this isn't accepted for an export.
pub const MIN_PASSPHRASE: usize = 8;
/// How long a staged import waits for the person to commit it.
const STAGED_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_STAGED: usize = 4;
const AGE_MAGIC: &[u8] = b"age-encryption.org/v1";

#[derive(Serialize, Deserialize)]
struct VaultFile {
    format: String,
    version: u32,
    exported_at: i64,
    #[serde(default)]
    app_version: String,
    entries: Vec<Incoming>,
}

fn passphrase_of(s: &str) -> SecretString {
    SecretString::from(s.to_string())
}

// ── Passphrase suggestion ────────────────────────────────────────────────────

/// Eight random words joined by hyphens: 64 bits, slowed down by scrypt.
pub fn suggest_passphrase() -> Result<String, String> {
    let mut bytes = [0u8; 8];
    SystemRandom::new().fill(&mut bytes).map_err(|_| "the system's random source failed".to_string())?;
    Ok(bytes.iter().map(|b| super::wordlist::WORDS[*b as usize]).collect::<Vec<_>>().join("-"))
}

// ── Export ───────────────────────────────────────────────────────────────────

/// What an export reported.
#[derive(Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ExportResult {
    pub count: usize,
    pub file_name: String,
}

/// Writes the chosen entries (all when `ids` is `None`) to `path`, encrypted to
/// `passphrase`.
pub fn export(store: &Store, ids: Option<&[String]>, passphrase: &str, path: &Path) -> Result<ExportResult, String> {
    if passphrase.chars().count() < MIN_PASSPHRASE {
        return Err(format!("a passphrase is at least {MIN_PASSPHRASE} characters"));
    }
    let rows = store.vault_list().map_err(|e| e.to_string())?;
    let rows: Vec<VaultRow> = match ids {
        Some(ids) => {
            let wanted: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
            rows.into_iter().filter(|r| wanted.contains(r.id.as_str())).collect()
        }
        None => rows,
    };
    if rows.is_empty() {
        return Err("there is nothing to export".into());
    }
    let key = seal::key(false)?;
    let mut entries = Vec::with_capacity(rows.len());
    for row in &rows {
        let parts = seal::open(&key, &row.id, &row.sealed)?;
        entries.push(Incoming {
            id: row.id.clone(),
            kind: row.kind.clone(),
            label: row.label.clone(),
            fields: serde_json::from_str(&row.fields).unwrap_or_default(),
            tags: serde_json::from_str(&row.tags).unwrap_or_default(),
            parts: parts.0.clone(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        });
    }
    let file = VaultFile { format: FORMAT.into(), version: VERSION, exported_at: now_ms(), app_version: env!("CARGO_PKG_VERSION").into(), entries };
    let plain = Zeroizing::new(serde_json::to_vec(&file).map_err(|e| e.to_string())?);
    // `file` holds every value too: wipe them.
    for mut e in file.entries {
        for v in e.parts.values_mut() {
            zeroize::Zeroize::zeroize(v);
        }
    }
    let mut out = Vec::new();
    let encryptor = age::Encryptor::with_user_passphrase(passphrase_of(passphrase));
    let mut w = encryptor.wrap_output(&mut out).map_err(|e| format!("couldn't start encrypting: {e}"))?;
    w.write_all(&plain).map_err(|e| format!("couldn't encrypt: {e}"))?;
    w.finish().map_err(|e| format!("couldn't finish encrypting: {e}"))?;
    write_atomically(path, &out)?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    record(store, "", "", PERSON, "exported", &file_name, &format!("{} entries", rows.len()));
    Ok(ExportResult { count: rows.len(), file_name })
}

/// The file appears whole or not at all: written beside it, then renamed.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, bytes).map_err(|e| {
        // A write that stopped part way (a full disk) leaves ciphertext beside the destination.
        let _ = std::fs::remove_file(&tmp);
        format!("couldn't write the file: {e}")
    })?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("couldn't save the file there: {e}")
    })
}

// ── Import: inspect ──────────────────────────────────────────────────────────

/// What `inspect` couldn't do yet.
#[derive(Debug)]
pub enum InspectError {
    /// The file is encrypted and no passphrase was given.
    NeedsPassphrase,
    /// A CSV no reader knows: which column is which is the person's to say.
    NeedsMapping(MappingNeeded),
    Other(String),
}

impl From<String> for InspectError {
    fn from(s: String) -> Self {
        InspectError::Other(s)
    }
}

struct Staged {
    file_name: String,
    entries: Vec<StagedEntry>,
    at: Instant,
}

struct StagedEntry {
    id: String,
    kind: &'static kinds::Kind,
    label: String,
    fields: BTreeMap<String, String>,
    tags: Vec<String>,
    parts: Parts,
    created_at: i64,
    updated_at: i64,
    /// The existing entry this one duplicates, if any.
    duplicate_of: Option<String>,
}

fn staged() -> &'static Mutex<HashMap<String, Staged>> {
    static STAGED: std::sync::OnceLock<Mutex<HashMap<String, Staged>>> = std::sync::OnceLock::new();
    STAGED.get_or_init(Mutex::default)
}

#[derive(Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PreviewEntry {
    pub label: String,
    pub kind: String,
    pub duplicate: bool,
}

#[derive(Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct PreviewSection {
    pub section: VaultSection,
    pub count: usize,
    /// The first few entries: labels, never values.
    pub first: Vec<PreviewEntry>,
}

#[derive(Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct Skipped {
    pub label: String,
    pub reason: String,
}

#[derive(Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ImportPreview {
    pub token: String,
    pub file_name: String,
    pub total: usize,
    pub duplicates: usize,
    pub sections: Vec<PreviewSection>,
    pub skipped: Vec<Skipped>,
    /// What the file is, found from its contents.
    pub source: Source,
    pub source_label: String,
    /// The file holds every secret in it unencrypted: the person is offered to delete it.
    pub plaintext_file: bool,
}

/// How many entries of each section the preview lists.
const PREVIEW_FIRST: usize = 5;

/// What a duplicate is: the same site and username for a login, the same kind
/// and main value for anything else.
fn fingerprint(kind: &kinds::Kind, fields: &BTreeMap<String, String>, parts: &Parts) -> Option<String> {
    if kind.name == "login" {
        let site = fields.get("site")?.trim().trim_end_matches('/').to_lowercase();
        let user = fields.get("username").map(|u| u.trim().to_lowercase()).unwrap_or_default();
        return Some(format!("login|{site}|{user}"));
    }
    Some(format!("{}|{}", kind.name, parts.get(kind.parts[0])?))
}

/// Reads `path`, decrypts it with `passphrase`, and stages it for `commit`.
pub fn inspect(store: &Store, path: &Path, passphrase: Option<&str>) -> Result<ImportPreview, InspectError> {
    inspect_with(store, path, passphrase, None)
}

/// [`inspect`], with the person's column choices for a CSV no reader recognises.
pub fn inspect_with(store: &Store, path: &Path, passphrase: Option<&str>, mapping: Option<&Mapping>) -> Result<ImportPreview, InspectError> {
    let meta = std::fs::metadata(path).map_err(|e| format!("couldn't read that file: {e}"))?;
    if !meta.is_file() {
        return Err("that isn't a file".to_string().into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("that file is over {} MiB, more than the Vault imports", MAX_FILE_BYTES / 1024 / 1024).into());
    }
    let mut head = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(MAX_FILE_BYTES).read_to_end(&mut head)).map_err(|e| format!("couldn't read that file: {e}"))?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // An age file is a Vault export; anything else is another service's plaintext export.
    let (source, incoming, mut skipped, plaintext_file) = if head.starts_with(AGE_MAGIC) {
        let plain = decrypt(&head, passphrase)?;
        let file: VaultFile = serde_json::from_slice(&plain).map_err(|_| "this isn't a Vault file: its contents aren't in the expected shape".to_string())?;
        drop(plain);
        if file.format != FORMAT {
            return Err(format!("this isn't a Vault file (its format is {:?})", file.format).into());
        }
        if file.version > VERSION {
            return Err(format!("this file was made by a newer AgentMux (format version {}); update AgentMux to import it", file.version).into());
        }
        (Source::Vault, file.entries, Vec::new(), false)
    } else {
        match import::read(&head, &file_name, mapping) {
            Ok(i) => (i.source, i.entries, i.skipped.into_iter().map(|(label, reason)| Skipped { label, reason }).collect(), true),
            Err(ReadError::NeedsMapping(m)) => return Err(InspectError::NeedsMapping(m)),
            Err(ReadError::Other(e)) => return Err(e.into()),
        }
    };
    drop(head);
    if incoming.len() > MAX_ENTRIES {
        return Err(format!("this file has more than {MAX_ENTRIES} entries, more than the Vault imports").into());
    }
    let existing = existing_fingerprints(store);
    let mut entries = Vec::new();
    for e in incoming {
        let Some(kind) = kinds::kind(&e.kind) else {
            skipped.push(Skipped { label: e.label, reason: format!("unknown kind {:?}", e.kind) });
            continue;
        };
        let mut parts = Parts(e.parts);
        parts.0.retain(|_, v| !v.is_empty());
        let fields: BTreeMap<String, String> = e.fields.into_iter().map(|(k, v)| (k, v.trim().to_string())).filter(|(_, v)| !v.is_empty()).collect();
        if let Err(why) = validate(kind, &e.label, &fields, &e.tags, &parts) {
            skipped.push(Skipped { label: e.label, reason: why });
            continue;
        }
        if parts.get(kind.parts[0]).is_none() {
            skipped.push(Skipped { label: e.label, reason: format!("it has no {}", kind.parts[0]) });
            continue;
        }
        let duplicate_of = fingerprint(kind, &fields, &parts).and_then(|f| existing.get(&f).cloned());
        entries.push(StagedEntry {
            id: e.id,
            kind,
            label: e.label.trim().to_string(),
            fields,
            tags: e.tags,
            parts,
            created_at: e.created_at,
            updated_at: e.updated_at,
            duplicate_of,
        });
    }
    let mut sections: Vec<PreviewSection> = Vec::new();
    for e in &entries {
        let section = e.kind.section;
        let slot = match sections.iter().position(|s| s.section == section) {
            Some(i) => i,
            None => {
                sections.push(PreviewSection { section, count: 0, first: Vec::new() });
                sections.len() - 1
            }
        };
        sections[slot].count += 1;
        if sections[slot].first.len() < PREVIEW_FIRST {
            sections[slot].first.push(PreviewEntry { label: e.label.clone(), kind: e.kind.name.into(), duplicate: e.duplicate_of.is_some() });
        }
    }
    let preview = ImportPreview {
        token: uuid::Uuid::new_v4().to_string(),
        file_name: file_name.clone(),
        total: entries.len(),
        duplicates: entries.iter().filter(|e| e.duplicate_of.is_some()).count(),
        sections,
        skipped,
        source,
        source_label: source.label().to_string(),
        plaintext_file,
    };
    let mut held = staged().lock().unwrap_or_else(|p| p.into_inner());
    held.retain(|_, s| s.at.elapsed() < STAGED_TTL);
    while held.len() >= MAX_STAGED {
        let Some(oldest) = held.iter().min_by_key(|(_, s)| s.at).map(|(k, _)| k.clone()) else { break };
        held.remove(&oldest);
    }
    held.insert(preview.token.clone(), Staged { file_name, entries, at: Instant::now() });
    Ok(preview)
}

/// Decrypts an age file with a passphrase; the plaintext is wiped on drop.
fn decrypt(bytes: &[u8], passphrase: Option<&str>) -> Result<Zeroizing<Vec<u8>>, InspectError> {
    if !bytes.starts_with(AGE_MAGIC) {
        return Err("this isn't a Vault file: it isn't an age-encrypted file".to_string().into());
    }
    let decryptor = age::Decryptor::new(bytes).map_err(|e| format!("this isn't a readable Vault file: {e}"))?;
    if !decryptor.is_scrypt() {
        return Err("this file isn't encrypted to a passphrase, so AgentMux can't open it".to_string().into());
    }
    let Some(passphrase) = passphrase.filter(|p| !p.is_empty()) else {
        return Err(InspectError::NeedsPassphrase);
    };
    let mut identity = age::scrypt::Identity::new(passphrase_of(passphrase));
    // §8.7: a file can't make the import hang on a huge work factor.
    identity.set_max_work_factor(22);
    let mut reader = decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity)).map_err(|e| match e {
        age::DecryptError::DecryptionFailed | age::DecryptError::NoMatchingKeys => "that passphrase didn't open the file".to_string(),
        age::DecryptError::ExcessiveWork { .. } => "this file's key-stretching is too heavy for AgentMux to open safely".to_string(),
        other => format!("couldn't decrypt the file: {other}"),
    })?;
    let mut plain = Zeroizing::new(Vec::new());
    reader.by_ref().take(MAX_FILE_BYTES * 2).read_to_end(&mut plain).map_err(|_| "the file is damaged or was changed: it didn't decrypt cleanly".to_string())?;
    Ok(plain)
}

/// The fingerprint of each existing entry, mapped to its id. Entries that can't
/// be opened aren't compared.
fn existing_fingerprints(store: &Store) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(rows) = store.vault_list() else { return out };
    if rows.is_empty() {
        return out;
    }
    let Ok(key) = seal::key(false) else { return out };
    for row in rows {
        let (Some(kind), Ok(parts)) = (kinds::kind(&row.kind), seal::open(&key, &row.id, &row.sealed)) else { continue };
        let fields: BTreeMap<String, String> = serde_json::from_str(&row.fields).unwrap_or_default();
        if let Some(f) = fingerprint(kind, &fields, &parts) {
            out.insert(f, row.id);
        }
    }
    out
}

// ── Import: commit ───────────────────────────────────────────────────────────

/// What to do with an incoming entry that duplicates one already in the Vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Duplicates {
    Skip,
    Replace,
    KeepBoth,
}

#[derive(Debug, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ImportResult {
    pub added: usize,
    pub replaced: usize,
    pub skipped: usize,
}

/// Writes a staged import: all of it or none of it.
pub fn commit(store: &Store, token: &str, duplicates: Duplicates) -> Result<ImportResult, String> {
    let taken = {
        let mut held = staged().lock().unwrap_or_else(|p| p.into_inner());
        held.remove(token)
    };
    let Some(staged_import) = taken.filter(|s| s.at.elapsed() < STAGED_TTL) else {
        return Err("this import has expired: choose the file again".into());
    };
    match write_staged(store, &staged_import, duplicates) {
        Ok(result) => Ok(result),
        Err(e) => {
            // Nothing was written: the import stays staged, so Import can be pressed again.
            staged().lock().unwrap_or_else(|p| p.into_inner()).insert(token.to_string(), staged_import);
            Err(e)
        }
    }
}

/// Writes `incoming`, all of it or none of it.
fn write_staged(store: &Store, incoming: &Staged, duplicates: Duplicates) -> Result<ImportResult, String> {
    let key = seal::key(store.vault_count().map_err(|e| e.to_string())? == 0)?;
    let now = now_ms();
    let mut inserts: Vec<VaultRow> = Vec::new();
    let mut updates: Vec<VaultRow> = Vec::new();
    let mut skipped = 0;
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    for e in &incoming.entries {
        if e.duplicate_of.is_some() && duplicates == Duplicates::Skip {
            skipped += 1;
            continue;
        }
        let created = if e.created_at > 0 { e.created_at } else { now };
        let fields = serde_json::to_string(&e.fields).map_err(|err| err.to_string())?;
        let tags = serde_json::to_string(&e.tags).map_err(|err| err.to_string())?;
        if let (Some(existing), Duplicates::Replace) = (&e.duplicate_of, duplicates) {
            updates.push(VaultRow {
                id: existing.clone(),
                kind: e.kind.name.into(),
                label: e.label.clone(),
                fields,
                tags,
                sealed: seal::seal(&key, existing, &e.parts)?,
                created_at: created,
                updated_at: now,
                last_used_at: 0,
            });
            continue;
        }
        // The file's own id when it is free, so an agent's reference survives
        // a round trip; else a new one.
        let wanted = e.id.trim();
        let id = if !wanted.is_empty()
            && wanted.len() <= 48
            && wanted.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !taken.contains(wanted)
            && store.vault_get(wanted).map_err(|err| err.to_string())?.is_none()
        {
            wanted.to_string()
        } else {
            let base = new_id(store, e.kind.name, &e.label)?;
            let mut id = base.clone();
            let mut n = 2;
            while taken.contains(&id) || store.vault_get(&id).map_err(|err| err.to_string())?.is_some() {
                id = format!("{base}-{n}");
                n += 1;
            }
            id
        };
        taken.insert(id.clone());
        inserts.push(VaultRow {
            sealed: seal::seal(&key, &id, &e.parts)?,
            id,
            kind: e.kind.name.into(),
            label: e.label.clone(),
            fields,
            tags,
            created_at: created,
            updated_at: now,
            last_used_at: 0,
        });
    }
    store.vault_write_many(&inserts, &updates).map_err(|e| e.to_string())?;
    let result = ImportResult { added: inserts.len(), replaced: updates.len(), skipped };
    record(store, "", "", PERSON, "imported", &incoming.file_name, &format!("{} added, {} replaced, {} skipped", result.added, result.replaced, result.skipped));
    Ok(result)
}

/// Drops a staged import (the person cancelled).
pub fn cancel(token: &str) {
    staged().lock().unwrap_or_else(|p| p.into_inner()).remove(token);
}
