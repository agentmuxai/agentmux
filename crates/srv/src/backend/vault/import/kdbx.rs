// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! KeePass `.kdbx` databases (KeePass, KeePassXC, KeePassDX, KeeWeb, Strongbox;
//! KDBX 3.1 and 4), opened with the person's master password (spec §8.4).
//!
//! A database chooses its own key-stretching cost, and opening it spends that
//! cost before anything is known about the file. So the header is read first, by
//! hand, and a database asking for more than [`MAX_ARGON_MEMORY`],
//! [`MAX_ARGON_ITERATIONS`] or [`MAX_AES_ROUNDS`] is refused before the password is
//! ever tried (§8.7).

use std::collections::BTreeMap;

use keepass::db::GroupRef;
use keepass::{Database, DatabaseKey};

use super::{Imported, Incoming, ReadError, Source, MAX_ITEMS};

pub const MAGIC: [u8; 4] = [0x03, 0xD9, 0xA2, 0x9A];
/// The second signature of KDBX 2 and later; the original `.kdb` has another.
const SIGNATURE_2: [u8; 4] = [0x67, 0xFB, 0x4B, 0xB5];
const SIGNATURE_2_KDB: [u8; 4] = [0x65, 0xFB, 0x4B, 0xB5];

/// Argon2 memory a database may ask for: 1 GiB.
pub const MAX_ARGON_MEMORY: u64 = 1 << 30;
pub const MAX_ARGON_ITERATIONS: u64 = 100;
pub const MAX_ARGON_PARALLELISM: u64 = 64;
/// AES-KDF rounds: a few seconds on a current machine.
pub const MAX_AES_ROUNDS: u64 = 150_000_000;

const KDF_ARGON2D: [u8; 16] = [0xEF, 0x63, 0x6D, 0xDF, 0x8C, 0x29, 0x44, 0x4B, 0x91, 0xF7, 0xA9, 0xA4, 0x03, 0xE3, 0x0A, 0x0C];
const KDF_ARGON2ID: [u8; 16] = [0x9E, 0x29, 0x8B, 0x19, 0x56, 0xDB, 0x47, 0x73, 0xB2, 0x3D, 0xFC, 0x3E, 0xC6, 0xF0, 0xA1, 0xE6];

const DAMAGED: &str = "this KeePass database is damaged or isn't one";

/// A cursor over the header's bytes that never reads past the end.
struct Bytes<'a>(&'a [u8]);

impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err(DAMAGED.to_string());
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}

fn le_u64(b: &[u8]) -> Result<u64, String> {
    match b.len() {
        4 => Ok(u32::from_le_bytes(b.try_into().unwrap()) as u64),
        8 => Ok(u64::from_le_bytes(b.try_into().unwrap())),
        _ => Err(DAMAGED.to_string()),
    }
}

fn too_heavy(what: &str) -> String {
    format!("this KeePass database asks for more {what} to open than AgentMux will spend; open it in KeePassXC and export a CSV instead")
}

/// Refuses a database whose key-stretching is heavier than the caps, or whose
/// header doesn't parse. Reads nothing secret.
pub fn check_header(bytes: &[u8]) -> Result<(), String> {
    let mut b = Bytes(bytes);
    if b.take(4)? != MAGIC {
        return Err(DAMAGED.to_string());
    }
    let second = b.take(4)?;
    if second == SIGNATURE_2_KDB {
        return Err("this is a KeePass 1.x (.kdb) database, which AgentMux can't open; export it from KeePass as a CSV".to_string());
    }
    if second != SIGNATURE_2 {
        return Err(DAMAGED.to_string());
    }
    let _minor = b.u16()?;
    let major = b.u16()?;
    match major {
        3 => {
            // Fields: id (1), size (2), data.
            loop {
                let id = b.u8()?;
                let size = b.u16()? as usize;
                let data = b.take(size)?;
                match id {
                    0 => return Ok(()),
                    // TransformRounds
                    6 if le_u64(data)? > MAX_AES_ROUNDS => return Err(too_heavy("key-stretching")),
                    _ => {}
                }
            }
        }
        4 => {
            // Fields: id (1), size (4), data.
            loop {
                let id = b.u8()?;
                let size = b.u32()? as usize;
                let data = b.take(size)?;
                match id {
                    0 => return Ok(()),
                    // KdfParameters, a variant dictionary.
                    11 => return check_kdf(data),
                    _ => {}
                }
            }
        }
        _ => Err("this KeePass database is a version AgentMux doesn't know".to_string()),
    }
}

/// The KDF variant dictionary of a KDBX 4 header.
fn check_kdf(data: &[u8]) -> Result<(), String> {
    let mut b = Bytes(data);
    let _version = b.u16()?;
    let mut uuid: Option<Vec<u8>> = None;
    let mut numbers: BTreeMap<String, u64> = BTreeMap::new();
    loop {
        let kind = b.u8()?;
        if kind == 0 {
            break;
        }
        let key_len = b.u32()? as usize;
        let key = String::from_utf8_lossy(b.take(key_len)?).into_owned();
        let value_len = b.u32()? as usize;
        let value = b.take(value_len)?;
        match (kind, key.as_str()) {
            (0x42, "$UUID") => uuid = Some(value.to_vec()),
            // UInt32, UInt64, Int32, Int64
            (0x04 | 0x05 | 0x0C | 0x0D, _) => {
                numbers.insert(key, le_u64(value)?);
            }
            _ => {}
        }
    }
    let uuid = uuid.ok_or_else(|| DAMAGED.to_string())?;
    if uuid == KDF_ARGON2D || uuid == KDF_ARGON2ID {
        let get = |k: &str| numbers.get(k).copied().ok_or_else(|| DAMAGED.to_string());
        if get("M")? > MAX_ARGON_MEMORY {
            return Err(too_heavy("memory"));
        }
        if get("I")? > MAX_ARGON_ITERATIONS || get("P")? > MAX_ARGON_PARALLELISM {
            return Err(too_heavy("key-stretching"));
        }
    } else if numbers.get("R").copied().unwrap_or(0) > MAX_AES_ROUNDS {
        return Err(too_heavy("key-stretching"));
    }
    Ok(())
}

/// The field names KeePass itself uses; any other field is the person's own.
const STANDARD: &[&str] = &["Title", "UserName", "Password", "URL", "Notes", "otp"];

/// A custom field named for a kind the Vault has becomes that kind (the names an
/// AgentMux export to KeePass uses, §8.3).
fn kind_of_field(name: &str) -> Option<(&'static str, &'static str)> {
    match name.trim().to_lowercase().as_str() {
        "ssn" | "social security number" => Some(("ssn", "value")),
        "card number" | "credit card number" => Some(("card", "number")),
        "api key" | "api_key" | "apikey" => Some(("api-key", "value")),
        _ => None,
    }
}

pub fn read(bytes: &[u8], passphrase: &str) -> Result<Imported, ReadError> {
    check_header(bytes)?;
    if passphrase.is_empty() {
        return Err(ReadError::NeedsPassphrase);
    }
    let db = Database::parse(bytes, DatabaseKey::new().with_password(passphrase)).map_err(|e| {
        use keepass::db::DatabaseOpenError as E;
        match e {
            E::Key(keepass::error::DatabaseKeyError::IncorrectKey) => ReadError::from("that password didn't open the database (a database that also needs a key file can't be opened here yet)"),
            E::Key(_) => ReadError::from("that password didn't open the database"),
            _ => ReadError::from(DAMAGED),
        }
    })?;
    let recycle = db.recycle_bin().map(|g| g.id());
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    walk(db.root(), &[], recycle, &mut entries, &mut skipped)?;
    Ok(Imported { source: Source::KeePass, entries, skipped, plaintext: false })
}

fn walk(group: GroupRef<'_>, path: &[String], recycle: Option<keepass::db::GroupId>, entries: &mut Vec<Incoming>, skipped: &mut Vec<(String, String)>) -> Result<(), ReadError> {
    if recycle == Some(group.id()) {
        return Ok(());
    }
    // The root group's own name ("Root", "Database") says nothing; below it, the path does.
    let mut here = path.to_vec();
    if !path.is_empty() || group.parent().is_some() {
        here.push(group.name.clone());
    }
    let tag = here.join("/");
    for entry in group.entries() {
        if entries.len() + skipped.len() >= MAX_ITEMS {
            return Err(format!("this database has more than {MAX_ITEMS} entries, more than the Vault imports").into());
        }
        entry_into(&entry, &tag, entries, skipped);
    }
    for child in group.groups() {
        walk(child, &here, recycle, entries, skipped)?;
    }
    Ok(())
}

fn entry_into(e: &keepass::db::Entry, tag: &str, entries: &mut Vec<Incoming>, skipped: &mut Vec<(String, String)>) {
    let get = |k: &str| e.get(k).map(str::trim).unwrap_or("");
    let site = get("URL");
    let label = match get("Title") {
        "" if !site.is_empty() => site.to_string(),
        "" => get("UserName").to_string(),
        t => t.to_string(),
    };
    let label = if label.is_empty() { "(unnamed)".to_string() } else { label };
    let mut made = Vec::new();
    // A password is kept exactly as saved, spaces and all.
    let password = e.get("Password").unwrap_or("");
    if !password.is_empty() {
        let mut login = Incoming::new("login", &label).part("password", password).field("site", site).field("username", get("UserName"));
        // KeePassXC's `otp` field, or KeePass's own TimeOtp fields.
        let totp = e.get_raw_otp_value().map(str::trim).filter(|v| !v.is_empty());
        if let Some(t) = totp {
            login = login.part("totp", t);
        }
        made.push(login);
    }
    // Custom fields: the ones named like a kind become that kind; the rest are kept
    // sealed in a note, since a custom field is often a recovery code or a PIN.
    let mut extra = String::new();
    let mut names: Vec<&String> = e.fields.keys().filter(|k| !STANDARD.contains(&k.as_str()) && !k.starts_with("TimeOtp-") && !k.starts_with("KPH: ") && !k.starts_with("_etm_")).collect();
    names.sort();
    for name in names {
        let value = e.get(name).map(str::trim).unwrap_or("");
        if value.is_empty() {
            continue;
        }
        match kind_of_field(name) {
            Some((kind, part)) => made.push(Incoming::new(kind, &format!("{label} ({name})")).part(part, value)),
            None => {
                if !extra.is_empty() {
                    extra.push('\n');
                }
                extra.push_str(&format!("{name}: {value}"));
            }
        }
    }
    // A one-time-code seed with no password to go with it (a 2FA-only entry) has no
    // login to ride on, so it is kept in the sealed note rather than dropped.
    if password.is_empty() {
        if let Some(t) = e.get_raw_otp_value().map(str::trim).filter(|v| !v.is_empty()) {
            if !extra.is_empty() {
                extra.push('\n');
            }
            extra.push_str(&format!("otp: {t}"));
        }
    }
    // Notes and the remaining custom fields, kept together and sealed.
    let notes = get("Notes");
    let text = match (notes.is_empty(), extra.is_empty()) {
        (true, _) => extra,
        (false, true) => notes.to_string(),
        (false, false) => format!("{notes}
{extra}"),
    };
    if !text.is_empty() {
        // An entry with no password of its own is just a note.
        let name = if made.is_empty() { label.clone() } else { format!("{label} (notes)") };
        made.push(Incoming::new("note", &name).part("text", &text));
    }
    if made.is_empty() {
        skipped.push((label, "it has no password, note or secret field".to_string()));
        return;
    }
    entries.extend(made.into_iter().map(|m| m.tag(tag)));
}

#[cfg(test)]
#[path = "kdbx_tests.rs"]
mod tests;
