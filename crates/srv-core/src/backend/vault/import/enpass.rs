// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Enpass's exports (spec §8.4): its JSON (folders and items whose `template_type` says what they
//! are and whose `fields` carry the values) and its CSV (a `Title,Field,Value,…` table whose rows
//! hold a title, then pairs of a field's name and value, then perhaps a note). Written from the
//! format as Bitwarden's open-source importer reads it, not from an export Enpass made.

use std::collections::HashMap;

use serde_json::Value;

use super::shared::{add_line, expiry, label_of, parse_csv, present, Sink};
use super::{Imported, ReadError, Source, MAX_ITEMS};

/// Folders nest; a chain longer than this is a loop or a crafted file.
const MOST_DEPTH: usize = 32;
/// Per login: how many of its addresses are kept.
const MOST_URLS: usize = 50;

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// A flag Enpass writes as a number (`1` is set).
fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_i64).is_some_and(|n| n != 0)
}

/// Each folder's path from the top (`Work/Servers`), found by walking up its parents.
fn folder_paths(root: &Value) -> HashMap<String, String> {
    let list: Vec<&Value> = root.get("folders").and_then(Value::as_array).into_iter().flatten().take(MAX_ITEMS).collect();
    let by_id: HashMap<&str, &Value> = list.iter().filter_map(|f| Some((f.get("uuid")?.as_str()?, *f))).collect();
    let mut paths = HashMap::new();
    for (id, folder) in &by_id {
        let mut parts = vec![text(folder, "title").trim()];
        let mut parent = text(folder, "parent_uuid");
        while !parent.is_empty() && parts.len() < MOST_DEPTH {
            let Some(up) = by_id.get(parent) else { break };
            parts.push(text(up, "title").trim());
            parent = text(up, "parent_uuid");
        }
        parts.reverse();
        let path = parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/");
        if !path.is_empty() {
            paths.insert(id.to_string(), path);
        }
    }
    paths
}

pub(super) fn read_json(input: &str) -> Result<Imported, ReadError> {
    let root: Value = serde_json::from_str(input).map_err(|_| ReadError::from("this isn't the JSON an Enpass export holds"))?;
    let items = root.get("items").and_then(Value::as_array).ok_or("this JSON isn't an Enpass export: it has no items")?;
    if items.len() > MAX_ITEMS {
        return Err(format!("this export has more than {MAX_ITEMS} items, more than the Vault imports").into());
    }
    if items.is_empty() {
        return Err("this Enpass export has no items".into());
    }
    let folders = folder_paths(&root);
    let mut sink = Sink::default();
    for item in items {
        let tag = item.get("folders").and_then(Value::as_array).and_then(|f| f.first()).and_then(Value::as_str).and_then(|id| folders.get(id)).map(String::as_str).unwrap_or("");
        item_into(item, tag, &mut sink);
    }
    sink.done(Source::Enpass)
}

/// One field of an item that has a value: (type, label, value). Deleted fields (Enpass keeps their
/// last value in the file) and section headings are left out.
fn live_fields(item: &Value) -> Vec<(&str, &str, &str)> {
    item.get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|f| !flag(f, "deleted"))
        .map(|f| (text(f, "type"), text(f, "label").trim(), text(f, "value")))
        .filter(|(ty, _, value)| *ty != "section" && present(value).is_some())
        .collect()
}

fn item_into(item: &Value, tag: &str, sink: &mut Sink) {
    let title = text(item, "title").trim();
    let label = if title.is_empty() { "Enpass item" } else { title };
    if flag(item, "trashed") {
        sink.skip(label, "it is in the trash");
        return;
    }
    let note = text(item, "note");
    let fields = live_fields(item);
    let template = text(item, "template_type");
    if template.starts_with("login.") || template.starts_with("password.") {
        login_into(sink, label, title.is_empty(), &fields, note, tag);
    } else if template.starts_with("creditcard.") {
        card_into(sink, label, &fields, note, tag);
    } else if !template.starts_with("identity.") && fields.iter().any(|(ty, _, _)| *ty == "password") {
        login_into(sink, label, title.is_empty(), &fields, note, tag);
    } else {
        // Anything else (an identity, a router, a licence…) has no kind of its own: kept whole, sealed.
        let mut lines = String::new();
        for (_, name, value) in &fields {
            add_line(&mut lines, name, value);
        }
        let mut all = String::new();
        add_line(&mut all, "", &lines);
        add_line(&mut all, "", note);
        sink.note(label, &all, tag);
    }
}

fn login_into(sink: &mut Sink, label: &str, untitled: bool, fields: &[(&str, &str, &str)], note: &str, tag: &str) {
    let (mut username, mut password, mut totp) = ("", "", "");
    let mut urls: Vec<&str> = Vec::new();
    let mut secret = String::new();
    add_line(&mut secret, "", note);
    for (ty, name, value) in fields {
        match *ty {
            "username" | "email" if username.is_empty() => username = value.trim(),
            "password" if password.is_empty() => password = *value,
            "totp" if totp.is_empty() => totp = value.trim(),
            "url" => {
                if urls.len() < MOST_URLS {
                    urls.push(value.trim());
                }
            }
            // The app a sign-in belongs to on a phone has no address a browser could offer it for.
            ".Android#" => add_line(&mut secret, "app", value),
            _ => add_line(&mut secret, name, value),
        }
    }
    for url in urls.iter().skip(1) {
        add_line(&mut secret, "url", url);
    }
    let site = urls.first().copied().unwrap_or("");
    let label = if untitled { label_of("", site, username) } else { label.to_string() };
    sink.login(&label, username, password, site, totp, &secret, tag);
}

fn card_into(sink: &mut Sink, label: &str, fields: &[(&str, &str, &str)], note: &str, tag: &str) {
    let (mut holder, mut number, mut cvc, mut expires) = ("", "", "", None);
    let mut secret = String::new();
    add_line(&mut secret, "", note);
    for (ty, name, value) in fields {
        match *ty {
            "ccName" if holder.is_empty() => holder = value.trim(),
            "ccNumber" if number.is_empty() => number = value.trim(),
            "ccCvc" if cvc.is_empty() => cvc = value.trim(),
            "ccExpiry" if expires.is_none() => match expiry(value) {
                Some(e) => expires = Some(e),
                None => add_line(&mut secret, name, value),
            },
            // The brand follows from the number.
            "ccType" => {}
            _ => add_line(&mut secret, name, value),
        }
    }
    sink.card(label, number, expires, cvc, holder, &secret, tag);
}

/// An Enpass CSV starts `Title,Field,Value`.
pub(super) fn csv_looks_like(names: &[String]) -> bool {
    names.len() >= 3 && names[0] == "title" && names[1] == "field" && names[2] == "value"
}

/// A field's name as Enpass labels it: a leading `*` marks a sensitive one.
fn plain_name(name: &str) -> &str {
    name.trim().trim_start_matches('*').trim()
}

pub(super) fn read_csv(input: &str) -> Result<Imported, ReadError> {
    let table = parse_csv(input, false)?;
    let mut sink = Sink::default();
    for (n, row) in table.rows.iter().enumerate() {
        let cells: Vec<&str> = row.iter().collect();
        if cells.len() < 2 || (n == 0 && cells[0].eq_ignore_ascii_case("title")) {
            continue;
        }
        let title = cells[0].trim();
        let label = if title.is_empty() { "Enpass item" } else { title };
        // Enpass appends a note as one more column, so a row of an even length ends with one.
        let note = if cells.len() % 2 == 0 { cells[cells.len() - 1] } else { "" };
        let pairs_end = if cells.len() % 2 == 0 { cells.len() - 1 } else { cells.len() };
        let pairs: Vec<(&str, &str)> = (1..pairs_end).step_by(2).filter(|i| i + 1 < pairs_end).map(|i| (cells[i], cells[i + 1])).filter(|(_, v)| present(v).is_some()).collect();
        let names: Vec<String> = (1..pairs_end).step_by(2).map(|i| plain_name(cells[i]).to_ascii_lowercase()).collect();
        let has = |n: &str| names.iter().any(|x| x == n);
        if has("cardholder") && has("number") && has("expiry date") {
            csv_card(&mut sink, label, &pairs, note);
        } else if cells.len() > 2 && ["username", "password", "email", "e-mail", "url", "website"].iter().any(|n| has(n)) {
            csv_login(&mut sink, label, title.is_empty(), &pairs, note);
        } else {
            let mut lines = String::new();
            for (name, value) in &pairs {
                add_line(&mut lines, plain_name(name), value);
            }
            let mut all = String::new();
            add_line(&mut all, "", &lines);
            add_line(&mut all, "", note);
            sink.note(label, &all, "");
        }
    }
    sink.done(Source::Enpass)
}

fn csv_login(sink: &mut Sink, label: &str, untitled: bool, pairs: &[(&str, &str)], note: &str) {
    let (mut username, mut password, mut totp) = ("", "", "");
    let mut urls: Vec<&str> = Vec::new();
    let mut secret = String::new();
    add_line(&mut secret, "", note);
    for (name, value) in pairs {
        match plain_name(name).to_ascii_lowercase().as_str() {
            "url" | "website" => {
                if urls.len() < MOST_URLS {
                    urls.push(value.trim());
                }
            }
            "username" | "email" | "e-mail" if username.is_empty() => username = value.trim(),
            "password" if password.is_empty() => password = *value,
            "totp" | "one-time code" if totp.is_empty() => totp = value.trim(),
            _ => add_line(&mut secret, plain_name(name), value),
        }
    }
    for url in urls.iter().skip(1) {
        add_line(&mut secret, "url", url);
    }
    let site = urls.first().copied().unwrap_or("");
    let label = if untitled { label_of("", site, username) } else { label.to_string() };
    sink.login(&label, username, password, site, totp, &secret, "");
}

fn csv_card(sink: &mut Sink, label: &str, pairs: &[(&str, &str)], note: &str) {
    let (mut holder, mut number, mut cvc, mut expires) = ("", "", "", None);
    let mut secret = String::new();
    add_line(&mut secret, "", note);
    for (name, value) in pairs {
        match plain_name(name).to_ascii_lowercase().as_str() {
            "cardholder" if holder.is_empty() => holder = value.trim(),
            "number" if number.is_empty() => number = value.trim(),
            "cvc" if cvc.is_empty() => cvc = value.trim(),
            "expiry date" if expires.is_none() => match expiry(value) {
                Some(e) => expires = Some(e),
                None => add_line(&mut secret, plain_name(name), value),
            },
            "type" => {}
            _ => add_line(&mut secret, plain_name(name), value),
        }
    }
    sink.card(label, number, expires, cvc, holder, &secret, "");
}
