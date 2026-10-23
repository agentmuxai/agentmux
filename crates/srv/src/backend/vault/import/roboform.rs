// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! RoboForm's CSV export (spec §8.4): `Name, Url, MatchUrl, Login, Pwd, Note, Folder, RfFieldsV2`,
//! where each extra field of a login is one more cell after the last column, written
//! `label,,,type,value`. A safe note is a row with only a name and a note. Written from the format
//! as Bitwarden's importer reads it, not from an export RoboForm made.

use super::shared::{cell, label_of, parse_csv, present, sealed, Sink};
use super::{Imported, ReadError, Source};

pub(super) fn looks_like(names: &[String]) -> bool {
    let has = |n: &str| names.iter().any(|x| x == n);
    has("matchurl") && has("name") && (has("pwd") || has("password")) && (has("rffieldsv2") || has("rffields"))
}

pub(super) fn read(text: &str) -> Result<Imported, ReadError> {
    let t = parse_csv(text, true)?;
    let (name, url, login, pwd) = (t.col(&["name"]), t.col(&["url"]), t.col(&["login"]), t.col(&["pwd", "password"]));
    let (note, folder) = (t.col(&["note"]), t.col(&["folder"]));
    let v2 = t.col(&["rffieldsv2"]);
    let fields_at = v2.or_else(|| t.col(&["rffields"]));
    let mut sink = Sink::default();
    for row in &t.rows {
        let label = label_of(cell(row, name), cell(row, url), cell(row, login));
        let password = pwd.and_then(|i| row.get(i)).unwrap_or("");
        // The last row of an export is sometimes a lone `--`.
        if label == "--" && password.is_empty() {
            continue;
        }
        let mut totp = String::new();
        let mut kept: Vec<String> = Vec::new();
        if let Some(from) = fields_at {
            for raw in row.iter().skip(from) {
                match (v2.is_some(), field_of(raw)) {
                    (true, Some((l, value))) => match l {
                        "TOTP KEY$" => totp = value.to_string(),
                        // These two repeat the login and password above.
                        "User ID$" | "Password$" if value == cell(row, login) || value == password => {}
                        "-no-name-" => kept.push(value.to_string()),
                        _ => kept.push(format!("{l}: {value}")),
                    },
                    // A cell that isn't in the form, or an older export's: kept whole.
                    _ => {
                        if let Some(r) = present(raw) {
                            kept.push(r.to_string());
                        }
                    }
                }
            }
        }
        let extra = kept.join("\n");
        let secret = sealed(&[("", cell(row, note)), ("", &extra)]);
        let tag = cell(row, folder).replace('\\', "/");
        sink.login(&label, cell(row, login), password, cell(row, url), &totp, &secret, tag.trim_matches('/'));
    }
    sink.done(Source::RoboForm)
}

/// A field cell, `label,,,type,value`: its label and value. The value may hold commas.
fn field_of(raw: &str) -> Option<(&str, &str)> {
    let parts: Vec<&str> = raw.splitn(5, ',').collect();
    if parts.len() < 5 {
        return None;
    }
    let value = parts[4].trim();
    (!value.is_empty()).then_some((parts[0].trim(), value))
}
