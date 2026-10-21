// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! 1Password's `.1pux` export: a zip whose `export.data` is JSON of accounts,
//! vaults and items (spec §8.4). It holds every secret unencrypted.
//!
//! The category of an item says what it is (`001` login, `002` card, …) but the
//! fields inside vary, so cards and bank accounts are found by what a field
//! holds or its id, and anything that isn't a kind the Vault has is kept whole,
//! field by field, in a sealed note rather than dropped.

use std::io::Read;

use serde_json::Value;

use super::{Imported, Incoming, ReadError, Source, MAX_ITEMS};

/// The uncompressed `export.data` may be at most this big: the zip is read
/// through this cap, so a crafted archive can't expand without limit (§8.7).
const MAX_DATA_BYTES: u64 = 128 * 1024 * 1024;
/// A `.1pux` holds a few files and its attachments; a zip of more is not one.
const MAX_ZIP_ENTRIES: usize = 100_000;

pub fn looks_like(bytes: &[u8]) -> bool {
    zip::ZipArchive::new(std::io::Cursor::new(bytes)).is_ok_and(|z| z.file_names().any(|n| n == "export.data"))
}

pub fn read(bytes: &[u8]) -> Result<Imported, ReadError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| ReadError::from("this zip file is damaged or isn't one"))?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err("this zip has more files than a 1Password export does".into());
    }
    let mut data = Vec::new();
    {
        let file = archive.by_name("export.data").map_err(|_| ReadError::from("this zip isn't a 1Password export: it has no export.data"))?;
        file.take(MAX_DATA_BYTES + 1).read_to_end(&mut data).map_err(|_| ReadError::from("couldn't read export.data from the zip"))?;
    }
    if data.len() as u64 > MAX_DATA_BYTES {
        return Err("this export's data is larger than the Vault imports".into());
    }
    let root: Value = serde_json::from_slice(&data).map_err(|_| ReadError::from("export.data isn't the JSON a 1Password export holds"))?;
    drop(data);
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    let mut count = 0usize;
    for account in root.get("accounts").and_then(Value::as_array).into_iter().flatten() {
        for vault in account.get("vaults").and_then(Value::as_array).into_iter().flatten() {
            let tag = vault.pointer("/attrs/name").and_then(Value::as_str).unwrap_or("");
            for item in vault.get("items").and_then(Value::as_array).into_iter().flatten() {
                count += 1;
                if count > MAX_ITEMS {
                    return Err(format!("this export has more than {MAX_ITEMS} items, more than the Vault imports").into());
                }
                item_into(item, tag, &mut entries, &mut skipped);
            }
        }
    }
    if count == 0 {
        return Err("this 1Password export has no items".into());
    }
    Ok(Imported { source: Source::OnePassword, entries, skipped, plaintext: true })
}

/// One field of an item, as text, with what it is called.
struct Field {
    id: String,
    title: String,
    value: String,
    /// What the value is: `string`, `concealed`, `totp`, `creditCardNumber`, `monthYear`, …
    kind: String,
}

fn text_of(kind: &str, v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) if kind == "monthYear" => {
            // 203012: December 2030.
            let n = n.as_i64()?;
            format!("{:02}/{}", n % 100, n / 100)
        }
        Value::Number(n) => n.to_string(),
        Value::Object(o) => {
            // An address, or an email or phone with its parts: the text parts, in order.
            let parts: Vec<String> = o.values().filter_map(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string).collect();
            parts.join(", ")
        }
        _ => return None,
    };
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn fields_of(details: &Value) -> Vec<Field> {
    let mut out = Vec::new();
    for section in details.get("sections").and_then(Value::as_array).into_iter().flatten() {
        for f in section.get("fields").and_then(Value::as_array).into_iter().flatten() {
            let Some(value) = f.get("value").and_then(Value::as_object) else { continue };
            // A value is a one-key object: {"concealed": "…"}, {"totp": "otpauth://…"}.
            let Some((kind, inner)) = value.iter().next() else { continue };
            let Some(text) = text_of(kind, inner) else { continue };
            out.push(Field {
                id: f.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
                title: f.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
                value: text,
                kind: kind.clone(),
            });
        }
    }
    out
}

fn find<'a>(fields: &'a [Field], ids: &[&str]) -> Option<&'a Field> {
    fields.iter().find(|f| ids.iter().any(|i| f.id.eq_ignore_ascii_case(i)))
}

/// The first field that looks like the item's one secret.
fn main_secret(fields: &[Field], ids: &[&str]) -> Option<String> {
    find(fields, ids)
        .or_else(|| fields.iter().find(|f| f.kind == "concealed"))
        .or_else(|| fields.iter().find(|f| f.kind == "string"))
        .map(|f| f.value.clone())
}

fn item_into(item: &Value, tag: &str, entries: &mut Vec<Incoming>, skipped: &mut Vec<(String, String)>) {
    let overview = item.get("overview").cloned().unwrap_or(Value::Null);
    let details = item.get("details").cloned().unwrap_or(Value::Null);
    let title = overview.get("title").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).unwrap_or("(unnamed)").to_string();
    if item.get("trashed").and_then(Value::as_bool) == Some(true) {
        skipped.push((title, "it is in 1Password's trash".to_string()));
        return;
    }
    let fields = fields_of(&details);
    let notes = details.get("notesPlain").and_then(Value::as_str).map(str::trim).unwrap_or("").to_string();
    let site = overview
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .or_else(|| overview.pointer("/urls/0/url").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let mut made: Vec<Incoming> = Vec::new();
    // Fields the item's own kind doesn't take, kept whole in the sealed note.
    let mut used: Vec<usize> = Vec::new();
    let mark = |used: &mut Vec<usize>, f: &Field| {
        if let Some(i) = fields.iter().position(|x| std::ptr::eq(x, f)) {
            used.push(i);
        }
    };
    match item.get("categoryUuid").and_then(Value::as_str).unwrap_or("") {
        "001" | "005" => {
            let login = |designation: &str| -> Option<String> {
                details
                    .get("loginFields")
                    .and_then(Value::as_array)?
                    .iter()
                    .find(|f| f.get("designation").and_then(Value::as_str) == Some(designation))
                    .and_then(|f| f.get("value").and_then(Value::as_str))
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
            };
            // An item of the Password category keeps its password in `details.password`.
            let password = login("password").or_else(|| details.get("password").and_then(Value::as_str).filter(|p| !p.is_empty()).map(str::to_string));
            match password {
                Some(pw) => {
                    let mut e = Incoming::new("login", &title).part("password", &pw).field("site", &site).field("username", &login("username").unwrap_or_default());
                    if let Some(t) = fields.iter().find(|f| f.kind == "totp") {
                        e = e.part("totp", &t.value);
                        mark(&mut used, t);
                    }
                    made.push(e);
                }
                None => {
                    if notes.is_empty() && fields.is_empty() {
                        skipped.push((title, "it has no password".to_string()));
                        return;
                    }
                }
            }
        }
        "002" => match fields.iter().find(|f| f.kind == "creditCardNumber").or_else(|| find(&fields, &["ccnum"])) {
            Some(number) => {
                mark(&mut used, number);
                let mut e = Incoming::new("card", &title).part("number", &number.value);
                for (part, ids, kind) in [("expiry", &["expiry", "validUntil"][..], "monthYear"), ("cvc", &["cvv"][..], ""), ("name", &["cardholder"][..], "")] {
                    let hit = find(&fields, ids).or_else(|| if kind.is_empty() { None } else { fields.iter().find(|f| f.kind == kind) });
                    if let Some(f) = hit {
                        e = e.part(part, &f.value);
                        mark(&mut used, f);
                    }
                }
                made.push(e);
            }
            None => {}
        },
        "101" => {
            if let Some(account) = find(&fields, &["accountNo", "accountNumber", "iban"]) {
                mark(&mut used, account);
                let mut e = Incoming::new("bank-account", &title).part("account", &account.value);
                if let Some(r) = find(&fields, &["routingNo", "routing"]) {
                    e = e.part("routing", &r.value);
                    mark(&mut used, r);
                }
                if let Some(b) = find(&fields, &["bankName"]) {
                    e = e.field("bank", &b.value);
                    mark(&mut used, b);
                }
                made.push(e);
            }
        }
        kind @ ("108" | "106" | "103") => {
            let (name, ids): (&str, &[&str]) = match kind {
                "108" => ("ssn", &["number", "ssn"]),
                "106" => ("passport", &["number", "passportNumber"]),
                _ => ("drivers-licence", &["number", "licenseNumber"]),
            };
            if let Some(v) = main_secret(&fields, ids) {
                if let Some(f) = fields.iter().find(|f| f.value == v) {
                    mark(&mut used, f);
                }
                let mut e = Incoming::new(name, &title).part("value", &v);
                if let Some(o) = find(&fields, &["fullname", "name", "owner"]) {
                    e = e.field("owner", &o.value);
                    mark(&mut used, o);
                }
                made.push(e);
            }
        }
        "112" => {
            if let Some(v) = main_secret(&fields, &["credential"]) {
                if let Some(f) = fields.iter().find(|f| f.value == v) {
                    mark(&mut used, f);
                }
                made.push(Incoming::new("api-key", &title).part("value", &v).field("service", &site));
            }
        }
        _ => {}
    }
    // What is left (a note's text, other fields, an item of a category the Vault has no
    // kind for) is kept whole, so nothing in the item is lost without being listed.
    let mut rest = String::new();
    if !notes.is_empty() {
        rest.push_str(&notes);
    }
    for (i, f) in fields.iter().enumerate() {
        if used.contains(&i) {
            continue;
        }
        if !rest.is_empty() {
            rest.push('\n');
        }
        let name = if f.title.is_empty() { &f.id } else { &f.title };
        rest.push_str(&format!("{name}: {}", f.value));
    }
    if !rest.is_empty() {
        let name = if made.is_empty() { title.clone() } else { format!("{title} (notes)") };
        made.push(Incoming::new("note", &name).part("text", &rest));
    }
    if made.is_empty() {
        skipped.push((title, "it holds nothing the Vault can keep".to_string()));
        return;
    }
    entries.extend(made.into_iter().map(|m| m.tag(tag)));
}

#[cfg(test)]
#[path = "onepux_tests.rs"]
mod tests;
