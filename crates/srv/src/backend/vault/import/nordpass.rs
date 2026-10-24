// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! NordPass's CSV export (spec §8.4): a header row, and a `type` column that says whether a row is
//! a login (`password`), a card (`credit_card`), a note or an identity. Written from the format as
//! Bitwarden's importer reads it, not from an export NordPass made.

use super::shared::{cell, expiry, label_of, parse_csv, present, sealed, Sink, Table};
use super::{Imported, ReadError, Source};

pub(super) fn looks_like(names: &[String]) -> bool {
    let has = |n: &str| names.iter().any(|x| x == n);
    has("name") && has("type") && has("cardholdername") && has("cardnumber") && has("expirydate")
}

pub(super) fn read(text: &str) -> Result<Imported, ReadError> {
    let t = parse_csv(text, true)?;
    let mut sink = Sink::default();
    for row in &t.rows {
        read_row(&t, row, &mut sink);
    }
    sink.done(Source::NordPass)
}

/// The `custom_fields` column: a JSON list of `{label, type, value}`, as lines to seal. Text that
/// isn't that list is kept as it is, so nothing is lost.
fn custom_fields(raw: &str) -> String {
    let Some(raw) = present(raw) else { return String::new() };
    match serde_json::from_str::<Vec<serde_json::Value>>(raw) {
        Ok(list) => {
            let lines: Vec<String> = list
                .iter()
                .filter_map(|f| {
                    let value = f.get("value")?.as_str()?;
                    let label = f.get("label").and_then(|l| l.as_str()).unwrap_or("");
                    present(value).map(|v| if label.is_empty() { v.to_string() } else { format!("{label}: {v}") })
                })
                .collect();
            lines.join("\n")
        }
        Err(_) => format!("custom_fields: {raw}"),
    }
}

/// `additional_urls`: a JSON list of addresses.
fn more_urls(raw: &str) -> String {
    let Some(raw) = present(raw) else { return String::new() };
    match serde_json::from_str::<Vec<String>>(raw) {
        Ok(urls) => urls.iter().filter_map(|u| present(u)).map(|u| format!("url: {u}")).collect::<Vec<_>>().join("\n"),
        Err(_) => format!("additional_urls: {raw}"),
    }
}

fn read_row(t: &Table, row: &csv::StringRecord, sink: &mut Sink) {
    let c = |name: &str| cell(row, t.col(&[name]));
    let (name, folder) = (c("name"), c("folder"));
    let extra = custom_fields(c("customfields"));
    match c("type").to_ascii_lowercase().as_str() {
        "password" => {
            let label = label_of(name, c("url"), c("username"));
            let secret = sealed(&[("", c("note")), ("", &more_urls(c("additionalurls"))), ("", &extra)]);
            let password = t.col(&["password"]).and_then(|i| row.get(i)).unwrap_or("");
            sink.login(&label, c("username"), password, c("url"), "", &secret, folder);
        }
        "credit_card" => {
            let label = if name.is_empty() { "NordPass card" } else { name };
            let secret = sealed(&[("", c("note")), ("zip code", c("zipcode")), ("", &extra)]);
            sink.card(label, c("cardnumber"), expiry(c("expirydate")), c("cvc"), c("cardholdername"), &secret, folder);
        }
        "note" => {
            let label = if name.is_empty() { "NordPass note" } else { name };
            sink.note(label, &sealed(&[("", c("note")), ("", &extra)]), folder);
        }
        "identity" => {
            // An address and a phone number have no kind of their own: kept whole, sealed.
            let label = if name.is_empty() { "NordPass identity" } else { name };
            let text = sealed(&[
                ("full name", c("fullname")),
                ("phone", c("phonenumber")),
                ("email", c("email")),
                ("address", c("address1")),
                ("address 2", c("address2")),
                ("city", c("city")),
                ("state", c("state")),
                ("zip code", c("zipcode")),
                ("country", c("country")),
                ("", c("note")),
                ("", &extra),
            ]);
            sink.note(label, &text, folder);
        }
        other => sink.skip(if name.is_empty() { "(unnamed)" } else { name }, &format!("an item of type {other:?} can't be imported")),
    }
}
