// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Bitwarden's (and Vaultwarden's) unencrypted JSON export: logins with their
//! one-time-code seeds, cards, secure notes, and the numbers an identity holds
//! (spec §8.4). A password-protected export is refused with how to get an
//! unencrypted one: its decryption isn't built yet.

use serde::Deserialize;

use super::{Imported, Incoming, ReadError, Source, MAX_ITEMS};

/// A list that an export may leave out or write as `null`.
fn list<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

#[derive(Deserialize)]
struct Export {
    #[serde(default)]
    encrypted: bool,
    #[serde(default, deserialize_with = "list")]
    folders: Vec<Folder>,
    #[serde(default, deserialize_with = "list")]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Folder {
    id: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Item {
    #[serde(rename = "type")]
    kind: Option<u8>,
    name: Option<String>,
    notes: Option<String>,
    folder_id: Option<String>,
    login: Option<Login>,
    card: Option<Card>,
    identity: Option<Identity>,
    #[serde(default, deserialize_with = "list")]
    fields: Vec<Custom>,
}

#[derive(Deserialize, Default)]
struct Login {
    username: Option<String>,
    password: Option<String>,
    totp: Option<String>,
    #[serde(default, deserialize_with = "list")]
    uris: Vec<Uri>,
}

#[derive(Deserialize)]
struct Uri {
    uri: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Card {
    cardholder_name: Option<String>,
    number: Option<String>,
    exp_month: Option<String>,
    exp_year: Option<String>,
    code: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Identity {
    ssn: Option<String>,
    passport_number: Option<String>,
    license_number: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
}

#[derive(Deserialize)]
struct Custom {
    name: Option<String>,
    value: Option<String>,
}

fn present(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// `MM/YYYY` from Bitwarden's separate month and year.
fn expiry(month: &Option<String>, year: &Option<String>) -> Option<String> {
    let (m, y) = (present(month)?, present(year)?);
    let m: u32 = m.parse().ok().filter(|m| (1..=12).contains(m))?;
    Some(format!("{m:02}/{y}"))
}

pub fn read(text: &str) -> Result<Imported, ReadError> {
    let export: Export = serde_json::from_str(text).map_err(|_| ReadError::from("this looks like JSON but isn't a Bitwarden export"))?;
    if export.encrypted {
        return Err("this Bitwarden export is password-protected, which AgentMux can't open yet: export it again as an unencrypted .json".into());
    }
    if export.items.is_empty() && export.folders.is_empty() {
        return Err("this JSON has no items: it isn't a Bitwarden export".into());
    }
    if export.items.len() > MAX_ITEMS {
        return Err(format!("this file has more than {MAX_ITEMS} items, more than the Vault imports").into());
    }
    let folder_name = |id: &Option<String>| -> Option<String> {
        let id = id.as_deref()?;
        export.folders.iter().find(|f| f.id.as_deref() == Some(id)).and_then(|f| f.name.clone())
    };
    let mut entries: Vec<Incoming> = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    for item in &export.items {
        let label = present(&item.name).unwrap_or("(unnamed)").to_string();
        let tag = folder_name(&item.folder_id).unwrap_or_default();
        let mut made: Vec<Incoming> = Vec::new();
        match item.kind {
            Some(1) => {
                let l = item.login.as_ref();
                let password = l.and_then(|l| present(&l.password));
                match password {
                    Some(password) => {
                        let mut e = Incoming::new("login", &label).part("password", password);
                        if let Some(totp) = l.and_then(|l| present(&l.totp)) {
                            e = e.part("totp", totp);
                        }
                        if let Some(l) = l {
                            e = e.field("username", present(&l.username).unwrap_or("")).field("site", l.uris.iter().find_map(|u| present(&u.uri)).unwrap_or(""));
                        }
                        made.push(e);
                    }
                    None => skipped.push((label.clone(), "it has no password".to_string())),
                }
            }
            Some(2) => match present(&item.notes) {
                Some(text) => made.push(Incoming::new("note", &label).part("text", text)),
                None => skipped.push((label.clone(), "the note is empty".to_string())),
            },
            Some(3) => {
                let c = item.card.as_ref();
                match c.and_then(|c| present(&c.number)) {
                    Some(number) => {
                        let c = c.unwrap();
                        let mut e = Incoming::new("card", &label).part("number", number);
                        if let Some(x) = expiry(&c.exp_month, &c.exp_year) {
                            e = e.part("expiry", &x);
                        }
                        if let Some(cvc) = present(&c.code) {
                            e = e.part("cvc", cvc);
                        }
                        if let Some(n) = present(&c.cardholder_name) {
                            e = e.part("name", n);
                        }
                        made.push(e);
                    }
                    None => skipped.push((label.clone(), "the card has no number".to_string())),
                }
            }
            Some(4) => {
                let id = item.identity.as_ref();
                let owner = id.map(|i| [present(&i.first_name), present(&i.last_name)].into_iter().flatten().collect::<Vec<_>>().join(" ")).unwrap_or_default();
                for (kind, what, value) in [
                    ("ssn", "Social Security number", id.and_then(|i| present(&i.ssn))),
                    ("passport", "passport", id.and_then(|i| present(&i.passport_number))),
                    ("drivers-licence", "driver's licence", id.and_then(|i| present(&i.license_number))),
                ] {
                    if let Some(v) = value {
                        made.push(Incoming::new(kind, &format!("{label} ({what})")).part("value", v).field("owner", &owner));
                    }
                }
                if made.is_empty() {
                    skipped.push((label.clone(), "the identity holds no Social Security, passport or licence number".to_string()));
                }
            }
            other => skipped.push((label.clone(), format!("an item of type {other:?} (a key or other item) can't be imported"))),
        }
        // Custom fields and notes can hold secrets, so they are kept sealed in a note
        // of their own rather than as plain details.
        let mut extra = String::new();
        if item.kind != Some(2) {
            if let Some(n) = present(&item.notes) {
                extra.push_str(n);
            }
        }
        for f in &item.fields {
            if let (Some(n), Some(v)) = (present(&f.name), present(&f.value)) {
                if !extra.is_empty() {
                    extra.push('\n');
                }
                extra.push_str(&format!("{n}: {v}"));
            }
        }
        if !made.is_empty() && !extra.is_empty() {
            made.push(Incoming::new("note", &format!("{label} (notes)")).part("text", &extra));
        }
        for e in made {
            entries.push(e.tag(&tag));
        }
    }
    Ok(Imported { source: Source::Bitwarden, entries, skipped })
}
