// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Readers for the files other password managers export
//! (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.4).
//!
//! Each reader turns a file's bytes into [`Incoming`] entries (the same shape a
//! `.vault` file holds) and says what it couldn't bring in and why. They're
//! attack surface (§8.7): they run on files a person was given, so each caps
//! what it reads and takes no value as trusted. Nothing here touches the Vault:
//! `transfer::inspect` stages what they return, and the person previews it.
//!
//! Everything here is a plaintext export (the encrypted sources, KeePass and
//! password-protected exports, come with their own passwords); `Imported`
//! says so, and the pane then offers to delete the file.

mod bitwarden;
mod bitwarden_encrypted;
mod dashlane;
mod env;
mod kdbx;
mod keeper;
pub use kdbx::{MAX_ARGON_ITERATIONS, MAX_ARGON_MEMORY, MAX_ARGON_PARALLELISM, MAX_KEY_FILE};
mod logins;
mod nordpass;
mod onepux;
mod roboform;
mod shared;
mod zoho;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Where an import came from, for the preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum Source {
    Vault,
    LastPass,
    OnePassword,
    Bitwarden,
    Chrome,
    Firefox,
    Safari,
    KeePass,
    Env,
    Csv,
    Dashlane,
    Keeper,
    NordPass,
    RoboForm,
    ZohoVault,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Vault => "an AgentMux Vault file",
            Source::LastPass => "LastPass",
            Source::OnePassword => "1Password",
            Source::Bitwarden => "Bitwarden",
            Source::Chrome => "a browser's password export (Chrome, Edge, Brave)",
            Source::Firefox => "Firefox",
            Source::Safari => "Safari or Apple Passwords",
            Source::KeePass => "a KeePass database",
            Source::Env => "a .env file",
            Source::Csv => "a CSV file",
            Source::Dashlane => "Dashlane",
            Source::Keeper => "Keeper",
            Source::NordPass => "NordPass",
            Source::RoboForm => "RoboForm",
            Source::ZohoVault => "Zoho Vault",
        }
    }
}

/// An entry as a file holds it, before it is checked against its kind.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Incoming {
    #[serde(default)]
    pub id: String,
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub parts: BTreeMap<String, String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

impl Incoming {
    pub fn new(kind: &str, label: &str) -> Self {
        Self { kind: kind.into(), label: label.trim().into(), ..Self::default() }
    }

    pub fn part(mut self, name: &str, value: &str) -> Self {
        self.parts.insert(name.into(), value.to_string());
        self
    }

    pub fn field(mut self, name: &str, value: &str) -> Self {
        let v = value.trim();
        if !v.is_empty() {
            self.fields.insert(name.into(), v.to_string());
        }
        self
    }

    pub fn tag(mut self, tag: &str) -> Self {
        let t = tag.trim();
        if !t.is_empty() && !self.tags.iter().any(|x| x == t) {
            self.tags.push(t.to_string());
        }
        self
    }
}

/// What a reader found.
pub struct Imported {
    pub source: Source,
    pub entries: Vec<Incoming>,
    /// (label, why) for what couldn't come in.
    pub skipped: Vec<(String, String)>,
    /// The file held every secret unencrypted (a CSV or JSON export): the person is
    /// offered to delete it. A KeePass database is encrypted, so it isn't one.
    pub plaintext: bool,
}

/// Which column of a CSV is what, chosen by the person for a file no reader knows.
#[derive(Debug, Clone, Deserialize)]
pub struct Mapping {
    /// The kind of entry every row becomes: `login`, `api-key`, `note`, ….
    pub kind: String,
    pub label: usize,
    /// The main value: a login's password, a key, a note's text.
    pub value: usize,
    #[serde(default)]
    pub site: Option<usize>,
    #[serde(default)]
    pub username: Option<usize>,
}

/// A CSV no reader recognised: its columns, for the person to say which is which.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct MappingNeeded {
    pub columns: Vec<String>,
    pub rows: usize,
}

pub enum ReadError {
    NeedsMapping(MappingNeeded),
    /// An encrypted database: its master password is needed.
    NeedsPassphrase,
    Other(String),
}

impl From<String> for ReadError {
    fn from(s: String) -> Self {
        ReadError::Other(s)
    }
}

impl From<&str> for ReadError {
    fn from(s: &str) -> Self {
        ReadError::Other(s.to_string())
    }
}

/// The most rows or items one file may hold (§8.7).
pub const MAX_ITEMS: usize = 100_000;

/// Reads an export that needs no password, finding out what it is from its contents.
pub fn read(bytes: &[u8], file_name: &str, mapping: Option<&Mapping>) -> Result<Imported, ReadError> {
    read_with(bytes, file_name, mapping, None)
}

/// [`read`], with the password an encrypted database (KeePass) needs.
pub fn read_with(bytes: &[u8], file_name: &str, mapping: Option<&Mapping>, passphrase: Option<&str>) -> Result<Imported, ReadError> {
    read_keyed(bytes, file_name, mapping, passphrase, None)
}

/// [`read_with`], and the key file a KeePass database may need as well as, or instead of, its password.
pub fn read_keyed(bytes: &[u8], file_name: &str, mapping: Option<&Mapping>, passphrase: Option<&str>, keyfile: Option<&[u8]>) -> Result<Imported, ReadError> {
    if bytes.starts_with(&kdbx::MAGIC) {
        return kdbx::read(bytes, passphrase.unwrap_or(""), keyfile);
    }
    if bytes.starts_with(b"PK\x03\x04") {
        if onepux::looks_like(bytes) {
            return onepux::read(bytes);
        }
        return Err("this is a zip file that isn't a 1Password export (a Proton Pass export is a zip too, which AgentMux can't import yet); export a CSV or JSON from that app instead".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "this isn't a text file AgentMux knows how to read: its export formats are CSV, JSON and .env".to_string())?;
    let text = text.trim_start_matches('\u{feff}');
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') {
        if let Some(own) = read_own_json(text) {
            return own;
        }
        return bitwarden::read(text, passphrase);
    }
    if file_name.to_ascii_lowercase().ends_with(".env") || env::looks_like_env(text) {
        return env::read(text);
    }
    if mapping.is_none() {
        if let Some(read) = read_service_csv(text) {
            return read;
        }
    }
    match logins::read(text, mapping) {
        // A table with no header row and no column the generic reader knows: Keeper writes those.
        Err(ReadError::NeedsMapping(needed)) if mapping.is_none() => keeper::read(text).unwrap_or(Err(ReadError::NeedsMapping(needed))),
        other => other,
    }
}

/// A CSV that one of the second-wave services wrote, found by its column names; `None` for any other.
fn read_service_csv(text: &str) -> Option<Result<Imported, ReadError>> {
    let names = shared::header_names(text);
    if nordpass::looks_like(&names) {
        return Some(nordpass::read(text));
    }
    if roboform::looks_like(&names) {
        return Some(roboform::read(text));
    }
    if zoho::looks_like(&names) {
        return Some(zoho::read(text));
    }
    dashlane::read(text)
}

/// AgentMux's own plain JSON export (a `.vault` file's contents, unencrypted), or `None` when the
/// text is some other JSON.
fn read_own_json(text: &str) -> Option<Result<Imported, ReadError>> {
    /// Just enough to tell whose JSON it is, without reading the entries.
    #[derive(Deserialize)]
    struct Probe {
        format: Option<String>,
    }
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        version: u32,
        entries: Vec<Incoming>,
    }
    let probe: Probe = serde_json::from_str(text).ok()?;
    if probe.format.as_deref() != Some(super::transfer::FORMAT) {
        return None;
    }
    let file: File = match serde_json::from_str(text) {
        Ok(f) => f,
        Err(_) => return Some(Err("this isn't a Vault export: its contents aren't in the expected shape".into())),
    };
    if file.version > super::transfer::VERSION {
        return Some(Err(format!("this file was made by a newer AgentMux (format version {}); update AgentMux to import it", file.version).into()));
    }
    if file.entries.len() > MAX_ITEMS {
        return Some(Err(format!("this file has more than {MAX_ITEMS} entries, more than the Vault imports").into()));
    }
    Some(Ok(Imported { source: Source::Vault, entries: file.entries, skipped: Vec::new(), plaintext: true }))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod fuzz_tests;
#[cfg(test)]
mod second_wave_tests;
