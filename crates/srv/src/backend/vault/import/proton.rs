// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Proton Pass's exports (spec §8.4): the zip it makes (`Proton Pass/data.json` beside the
//! attachments), its JSON on its own, and its CSV (logins; a card or an identity carries its fields
//! as JSON in the note column). Written from Proton's own open-source export and import code, not
//! from an export the service made. A passphrase-protected export (`data.pgp`) is refused with
//! how to export again.

use std::collections::HashMap;
use std::io::Read;

use serde_json::Value;

use super::shared::{add_line, label_of, month_year, parse_csv, present, Sink};
use super::{Imported, ReadError, Source, MAX_ITEMS};

/// The uncompressed `data.json` may be at most this big, as for every reader (§8.7).
const MAX_DATA_BYTES: u64 = 64 * 1024 * 1024;
/// A Proton Pass zip holds `data.json` and its attachments; a zip of more files than this isn't one.
const MAX_ZIP_ENTRIES: usize = 100_000;
/// Per item: how many of its addresses are kept.
const MOST_URLS: usize = 50;
/// Folders nest; a chain longer than this is a loop or a crafted file.
const MOST_DEPTH: usize = 32;

const DATA_JSON: &str = "Proton Pass/data.json";
const DATA_PGP: &str = "Proton Pass/data.pgp";
const FILES_DIR: &str = "Proton Pass/files/";

pub(super) const ENCRYPTED: &str = "this Proton Pass export is protected with a passphrase, which AgentMux can't open yet; export it again from Proton Pass without the passphrase (as a zip or as JSON), or as a CSV for the logins";

static NULL: Value = Value::Null;

pub(super) fn zip_looks_like(bytes: &[u8]) -> bool {
    zip::ZipArchive::new(std::io::Cursor::new(bytes)).is_ok_and(|z| z.file_names().any(|n| n == DATA_JSON || n == DATA_PGP))
}

pub(super) fn read_zip(bytes: &[u8]) -> Result<Imported, ReadError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| ReadError::from("this zip file is damaged or isn't one"))?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err("this zip has more files than a Proton Pass export does".into());
    }
    let attachments = archive.file_names().filter(|n| n.starts_with(FILES_DIR) && !n.ends_with('/')).count();
    let mut data = Vec::new();
    match archive.by_name(DATA_JSON) {
        Ok(file) => {
            file.take(MAX_DATA_BYTES + 1).read_to_end(&mut data).map_err(|_| ReadError::from("couldn't read data.json from the zip"))?;
        }
        Err(_) => return Err(ENCRYPTED.into()),
    }
    if data.len() as u64 > MAX_DATA_BYTES {
        return Err("this export's data is larger than the Vault imports".into());
    }
    let text = String::from_utf8(data).map_err(|_| ReadError::from("data.json isn't the text a Proton Pass export holds"))?;
    let mut imported = read_json(&text)?;
    if attachments > 0 {
        imported.skipped.push(("attachments".into(), format!("{attachments} attached file{} can't be imported", if attachments == 1 { "" } else { "s" })));
    }
    Ok(imported)
}

/// The text of a string member, untrimmed (a password is kept as exported); empty when there is none.
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

pub(super) fn read_json(text: &str) -> Result<Imported, ReadError> {
    let root: Value = serde_json::from_str(text).map_err(|_| ReadError::from("this isn't the JSON a Proton Pass export holds"))?;
    if root.get("encrypted").and_then(Value::as_bool) == Some(true) {
        return Err(ENCRYPTED.into());
    }
    let vaults = root.get("vaults").and_then(Value::as_object).ok_or("this JSON isn't a Proton Pass export: it has no vaults")?;
    let mut sink = Sink::default();
    let mut count = 0usize;
    for vault in vaults.values() {
        let vault_name = text_of(vault, "name");
        let folders = folder_paths(vault);
        for item in vault.get("items").and_then(Value::as_array).into_iter().flatten() {
            count += 1;
            if count > MAX_ITEMS {
                return Err(format!("this export has more than {MAX_ITEMS} items, more than the Vault imports").into());
            }
            let folder = item.get("folderId").and_then(Value::as_str).and_then(|id| folders.get(id));
            let tag = match folder {
                Some(path) if !vault_name.is_empty() => format!("{vault_name}/{path}"),
                Some(path) => path.clone(),
                None => vault_name.to_string(),
            };
            item_into(item, &tag, &mut sink);
        }
    }
    if count == 0 {
        return Err("this Proton Pass export has no items".into());
    }
    sink.done(Source::ProtonPass)
}

fn text_of<'a>(v: &'a Value, key: &str) -> &'a str {
    text(v, key).trim()
}

/// Each folder's path from the vault's root (`Work/Servers`), found by walking up its parents.
fn folder_paths(vault: &Value) -> HashMap<String, String> {
    let list: Vec<&Value> = vault.get("folders").and_then(Value::as_array).into_iter().flatten().take(MAX_ITEMS).collect();
    let by_id: HashMap<&str, &Value> = list.iter().filter_map(|f| Some((f.get("folderId")?.as_str()?, *f))).collect();
    let mut paths = HashMap::new();
    for (id, folder) in &by_id {
        let mut parts = vec![text_of(folder, "name")];
        let mut parent = folder.get("parentFolderId").and_then(Value::as_str);
        while let Some(p) = parent {
            if parts.len() >= MOST_DEPTH {
                break;
            }
            let Some(up) = by_id.get(p) else { break };
            parts.push(text_of(up, "name"));
            parent = up.get("parentFolderId").and_then(Value::as_str);
        }
        parts.reverse();
        let path = parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/");
        if !path.is_empty() {
            paths.insert(id.to_string(), path);
        }
    }
    paths
}

/// An extra field's value: a one-time-code address, a time, or its text.
fn extra_value(field: &Value) -> String {
    let data = field.get("data").unwrap_or(&NULL);
    match text(field, "type") {
        "totp" => text(data, "totpUri").to_string(),
        "timestamp" => match data.get("timestamp") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            Some(other) if !other.is_null() => other.to_string(),
            _ => String::new(),
        },
        _ => text(data, "content").to_string(),
    }
}

/// Extra fields, as lines to seal.
fn extra_lines(fields: Option<&Value>) -> String {
    let mut out = String::new();
    for field in fields.and_then(Value::as_array).into_iter().flatten() {
        add_line(&mut out, text_of(field, "fieldName"), &extra_value(field));
    }
    out
}

/// The sections of a custom item (its extra fields, grouped), as lines to seal.
fn section_lines(sections: Option<&Value>) -> String {
    let mut out = String::new();
    for section in sections.and_then(Value::as_array).into_iter().flatten() {
        let lines = extra_lines(section.get("sectionFields"));
        if lines.is_empty() {
            continue;
        }
        add_line(&mut out, "", text_of(section, "sectionName"));
        add_line(&mut out, "", &lines);
    }
    out
}

fn join(parts: &[&str]) -> String {
    let mut out = String::new();
    for p in parts {
        add_line(&mut out, "", p);
    }
    out
}

/// A login's addresses: the full list with its autofill modes when there is one, else the plain list.
fn login_urls(content: &Value) -> Vec<String> {
    let modes: Vec<String> = content
        .get("autofillUrls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|u| present(text(u, "url")).map(str::to_string))
        .take(MOST_URLS)
        .collect();
    if !modes.is_empty() {
        return modes;
    }
    content.get("urls").and_then(Value::as_array).into_iter().flatten().filter_map(|u| u.as_str().and_then(present).map(str::to_string)).take(MOST_URLS).collect()
}

fn item_into(item: &Value, tag: &str, sink: &mut Sink) {
    let data = item.get("data").unwrap_or(&NULL);
    let meta = data.get("metadata").unwrap_or(&NULL);
    let content = data.get("content").unwrap_or(&NULL);
    let name = text_of(meta, "name");
    let note = text(meta, "note");
    let label = if name.is_empty() { "Proton Pass item" } else { name };
    if item.get("state").and_then(Value::as_u64) == Some(2) {
        sink.skip(label, "it is in the trash");
        return;
    }
    let extra = extra_lines(data.get("extraFields"));
    match text(data, "type") {
        "login" => login_into(sink, label, content, note, &extra, tag),
        "alias" => {
            let mut t = String::new();
            add_line(&mut t, "alias", text(item, "aliasEmail"));
            sink.note(label, &join(&[&t, note, &extra]), tag);
        }
        "note" => sink.note(label, &join(&[note, &extra]), tag),
        "creditCard" => card_into(sink, label, content, note, &extra, tag),
        "identity" => identity_into(sink, label, content, note, &extra, tag),
        "sshKey" => {
            let mut t = String::new();
            for (name, key) in [("private key", "privateKey"), ("public key", "publicKey"), ("fingerprint", "fingerprint")] {
                add_line(&mut t, name, text(content, key));
            }
            sink.note(label, &join(&[&t, note, &section_lines(content.get("sections")), &extra]), tag);
        }
        "wifi" => {
            let security = match content.get("security").and_then(Value::as_u64) {
                Some(1) => "WPA",
                Some(2) => "WPA2",
                Some(3) => "WPA3",
                Some(4) => "WEP",
                _ => "",
            };
            let mut t = String::new();
            add_line(&mut t, "SSID", text(content, "ssid"));
            add_line(&mut t, "password", text(content, "password"));
            add_line(&mut t, "security", security);
            sink.note(label, &join(&[&t, note, &section_lines(content.get("sections")), &extra]), tag);
        }
        "custom" => sink.note(label, &join(&[note, &section_lines(content.get("sections")), &extra]), tag),
        _ => {
            // A type this reader doesn't know: its plain fields are kept, so nothing is lost.
            let mut t = String::new();
            for (key, value) in content.as_object().into_iter().flatten() {
                match value {
                    Value::String(s) => add_line(&mut t, key, s),
                    Value::Number(n) => add_line(&mut t, key, &n.to_string()),
                    _ => {}
                }
            }
            sink.note(label, &join(&[&t, note, &section_lines(content.get("sections")), &extra]), tag);
        }
    }
}

fn login_into(sink: &mut Sink, label: &str, content: &Value, note: &str, extra: &str, tag: &str) {
    let (user, email) = (text_of(content, "itemUsername"), text_of(content, "itemEmail"));
    // A login with no username is signed into with its email.
    let username = if user.is_empty() { email } else { user };
    let urls = login_urls(content);
    let mut secret = String::new();
    add_line(&mut secret, "", note);
    if !user.is_empty() && !email.is_empty() && user != email {
        add_line(&mut secret, "email", email);
    }
    for url in urls.iter().skip(1) {
        add_line(&mut secret, "url", url);
    }
    add_line(&mut secret, "", extra);
    let site = urls.first().map(String::as_str).unwrap_or("");
    let label_for = if label == "Proton Pass item" { label_of("", site, username) } else { label.to_string() };
    sink.login(&label_for, username, text(content, "password"), site, text(content, "totpUri"), &secret, tag);
    let passkeys = content.get("passkeys").and_then(Value::as_array).map_or(0, Vec::len);
    if passkeys > 0 {
        sink.skip(&format!("{label_for} (passkey)"), "a passkey can't be imported");
    }
}

/// `MM/YYYY` from the `2030-03` a card's expiry is written as.
fn card_expiry(s: &str) -> Option<String> {
    let mut parts = s.trim().split('-');
    let year = parts.next()?;
    month_year(parts.next()?, year)
}

fn card_into(sink: &mut Sink, label: &str, content: &Value, note: &str, extra: &str, tag: &str) {
    let mut secret = String::new();
    add_line(&mut secret, "", note);
    add_line(&mut secret, "PIN", text(content, "pin"));
    add_line(&mut secret, "", extra);
    sink.card(label, text(content, "number"), card_expiry(text(content, "expirationDate")), text(content, "verificationNumber"), text(content, "cardholderName"), &secret, tag);
}

/// The numbers an identity holds that the Vault has a kind for: (member, kind, what to call it).
const ID_NUMBERS: [(&str, &str, &str); 3] =
    [("socialSecurityNumber", "ssn", "Social Security number"), ("passportNumber", "passport", "passport"), ("licenseNumber", "drivers-licence", "driver's licence")];

fn identity_into(sink: &mut Sink, label: &str, content: &Value, note: &str, extra: &str, tag: &str) {
    let owner = match present(text(content, "fullName")) {
        Some(n) => n.to_string(),
        None => [text(content, "firstName"), text(content, "middleName"), text(content, "lastName")].iter().filter_map(|s| present(s)).collect::<Vec<_>>().join(" "),
    };
    for (key, kind, what) in ID_NUMBERS {
        if present(text(content, key)).is_some() {
            sink.id(kind, &format!("{label} ({what})"), text(content, key), &owner, "", "", tag);
        }
    }
    // The rest (names, addresses, contacts) has no kind: kept whole, sealed.
    let mut t = String::new();
    let mut extras = String::new();
    for (key, value) in content.as_object().into_iter().flatten() {
        if ID_NUMBERS.iter().any(|(k, _, _)| k == key) {
            continue;
        }
        match (key.as_str(), value) {
            ("extraSections", sections) => add_line(&mut extras, "", &section_lines(Some(sections))),
            (k, fields) if k.starts_with("extra") => add_line(&mut extras, "", &extra_lines(Some(fields))),
            (k, Value::String(s)) => add_line(&mut t, k, s),
            (k, Value::Number(n)) => add_line(&mut t, k, &n.to_string()),
            _ => {}
        }
    }
    let text = join(&[&t, note, &extras, extra]);
    if !text.is_empty() {
        sink.note(label, &text, tag);
    }
}

pub(super) fn csv_looks_like(names: &[String]) -> bool {
    let has = |n: &str| names.iter().any(|x| x == n);
    has("name") && has("password") && has("note") && has("totp") && has("type") && has("vault")
}

pub(super) fn read_csv(input: &str) -> Result<Imported, ReadError> {
    let t = parse_csv(input, true)?;
    let mut sink = Sink::default();
    for row in &t.rows {
        let c = |name: &str| t.get(row, name);
        let (name, url, vault) = (c("name"), c("url"), c("vault"));
        let password = t.col(&["password"]).and_then(|i| row.get(i)).unwrap_or("");
        match c("type").to_ascii_lowercase().as_str() {
            "" | "login" => {
                // An older export has no email column: its username column holds the email.
                let (user, email) = (c("username"), c("email"));
                let username = if user.is_empty() { email } else { user };
                let urls = csv_urls(c("autofillurls"), url);
                let mut secret = String::new();
                add_line(&mut secret, "", c("note"));
                if !user.is_empty() && !email.is_empty() && user != email {
                    add_line(&mut secret, "email", email);
                }
                for u in urls.iter().skip(1) {
                    add_line(&mut secret, "url", u);
                }
                let site = urls.first().map(String::as_str).unwrap_or("");
                sink.login(&label_of(name, site, username), username, password, site, c("totp"), &secret, vault);
            }
            "alias" => {
                let mut lines = String::new();
                add_line(&mut lines, "alias", c("email"));
                let label = if name.is_empty() { "Proton Pass alias" } else { name };
                sink.note(label, &join(&[&lines, c("note")]), vault);
            }
            "creditcard" => match serde_json::from_str::<Value>(c("note")) {
                Ok(content) => card_into(&mut sink, if name.is_empty() { "Proton Pass card" } else { name }, &content, text(&content, "note"), "", vault),
                Err(_) => sink.note(if name.is_empty() { "Proton Pass card" } else { name }, c("note"), vault),
            },
            "identity" => match serde_json::from_str::<Value>(c("note")) {
                Ok(content) => identity_into(&mut sink, if name.is_empty() { "Proton Pass identity" } else { name }, &content, text(&content, "note"), "", vault),
                Err(_) => sink.note(if name.is_empty() { "Proton Pass identity" } else { name }, c("note"), vault),
            },
            _ => sink.note(if name.is_empty() { "Proton Pass note" } else { name }, c("note"), vault),
        }
    }
    sink.done(Source::ProtonPass)
}

/// A login row's addresses: the structured column when it parses, else the plain one (`a, b`).
fn csv_urls(structured: &str, plain: &str) -> Vec<String> {
    if let Ok(Value::Array(list)) = serde_json::from_str::<Value>(structured) {
        let urls: Vec<String> = list.iter().filter_map(|u| present(text(u, "url")).map(str::to_string)).take(MOST_URLS).collect();
        if !urls.is_empty() {
            return urls;
        }
    }
    plain.split(", ").filter_map(|u| present(u).map(str::to_string)).take(MOST_URLS).collect()
}
