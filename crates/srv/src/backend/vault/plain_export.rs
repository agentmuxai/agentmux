// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Export the Vault as a plain, unencrypted CSV or JSON file, for a person leaving for another
//! manager (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.3).
//!
//! The file holds every secret in the clear, so everything around it is careful: only the host
//! asks for it, after the person typed a confirmation and the OS confirmed them (like Reveal), to
//! a path they chose in the native dialog; the file is private to its owner where the OS has such
//! a thing; the bytes are wiped after the write; and the Activity log says it happened.
//!
//! - **CSV** uses Bitwarden's column layout, which nearly every manager reads. A login is a row
//!   with its address, user name, password and one-time-code seed. Every other kind is a secure
//!   note whose text lists its parts by their names (`SSN: 123-45-6789`), because a column layout
//!   made for logins has nowhere else to put a card or an identity.
//! - **JSON** is the `.vault` file's own contents, unencrypted: every kind, part, detail and tag.
//!   AgentMux imports it back.

use std::io::Write as _;
use std::path::Path;

use zeroize::Zeroizing;

use super::import::Incoming;
use super::kdbx_names::{self, COMPANIONS, DETAILS};
use super::transfer::{gather, plain_json, wipe, ExportResult};
use super::{record, PERSON};
use crate::backend::storage::store::Store;

/// Bitwarden's CSV columns, in its order.
const HEADER: [&str; 11] = ["folder", "favorite", "type", "name", "notes", "fields", "reprompt", "login_uri", "login_username", "login_password", "login_totp"];

/// Writes the chosen entries (all when `ids` is `None`) to `path` as plain `csv` or `json`.
pub fn export(store: &Store, ids: Option<&[String]>, format: &str, path: &Path) -> Result<ExportResult, String> {
    let json = match format {
        "csv" => false,
        "json" => true,
        other => return Err(format!("unknown plain export format {other:?}")),
    };
    let entries = gather(store, ids)?;
    let count = entries.len();
    let bytes = if json { plain_json(&entries) } else { csv_bytes(&entries) };
    wipe(entries);
    write_private(path, &bytes?)?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    record(store, "", "", PERSON, "exported", &file_name, &format!("{count} entries, unencrypted {}", format.to_uppercase()));
    Ok(ExportResult { count, file_name })
}

/// The CSV's bytes.
pub(super) fn csv_bytes(entries: &[Incoming]) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut out = Zeroizing::new(Vec::new());
    {
        let mut w = csv::WriterBuilder::new().terminator(csv::Terminator::CRLF).from_writer(&mut *out);
        w.write_record(HEADER).map_err(|e| e.to_string())?;
        for e in entries {
            let row = row(e);
            let result = w.write_record(row.iter().map(|c| c.as_str()));
            result.map_err(|e| format!("couldn't write the CSV: {e}"))?;
        }
        w.flush().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// One entry as a row of [`HEADER`]'s columns.
fn row(e: &Incoming) -> Vec<Zeroizing<String>> {
    let part = |name: &str| e.parts.get(name).map(String::as_str).unwrap_or("");
    let field = |name: &str| e.fields.get(name).map(String::as_str).unwrap_or("");
    let z = |s: &str| Zeroizing::new(s.to_string());
    let folder = z(e.tags.first().map(String::as_str).unwrap_or(""));
    if e.kind == "login" {
        let mut notes = field("notes").to_string();
        // A part this layout has no column for goes in the notes, so nothing is dropped.
        for (name, value) in &e.parts {
            if !matches!(name.as_str(), "password" | "totp") && !value.is_empty() {
                notes = format!("{name}: {value}\n{notes}");
            }
        }
        return vec![folder, z("0"), z("login"), z(&e.label), Zeroizing::new(notes), z(""), z("0"), z(field("site")), z(field("username")), z(part("password")), z(part("totp"))];
    }
    vec![folder, z("0"), z("note"), z(&e.label), Zeroizing::new(note_text(e)), z(""), z("0"), z(""), z(""), z(""), z("")]
}

/// The text of a secure note holding everything an entry has: a note's own text, or one line per
/// part and detail, by the names a KeePass export uses, then the entry's notes.
fn note_text(e: &Incoming) -> String {
    if e.kind == "note" {
        return e.parts.get("text").cloned().unwrap_or_default();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut used: Vec<&str> = Vec::new();
    let mut add = |title: &str, value: &str| {
        if !value.is_empty() {
            lines.push(format!("{title}: {value}"));
        }
    };
    if let Some(p) = kdbx_names::primary_for(&e.kind) {
        add(p.title, e.parts.get(p.part).map(String::as_str).unwrap_or(""));
        used.push(p.part);
    }
    for c in COMPANIONS.iter().filter(|c| c.kind == e.kind) {
        add(c.title, e.parts.get(c.part).map(String::as_str).unwrap_or(""));
        used.push(c.part);
    }
    for d in DETAILS {
        add(d.title, e.fields.get(d.name).map(String::as_str).unwrap_or(""));
    }
    // Whatever else the entry has, under its own name.
    for (name, value) in &e.parts {
        if !used.contains(&name.as_str()) {
            add(name, value);
        }
    }
    let mut text = lines.join("\n");
    if let Some(notes) = e.fields.get("notes").filter(|n| !n.is_empty()) {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(notes);
    }
    text
}

/// The file appears whole or not at all, and is readable by its owner alone where the OS has such
/// a thing (a plain copy of every secret is not for other accounts on the machine).
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options.open(&tmp).and_then(|mut f| f.write_all(bytes).and_then(|_| f.sync_all()));
    if let Err(e) = written {
        // A write that stopped part way (a full disk) leaves part of the secrets beside the destination.
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("couldn't write the file: {e}"));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("couldn't save the file there: {e}")
    })
}

#[cfg(test)]
#[path = "plain_export_tests.rs"]
mod tests;
