// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Keeper's CSV export (spec §8.4). It has no header row: each row is `folder, title, login,
//! password, website, notes, shared folder`, then custom fields as pairs of a name and a value,
//! and a one-time code is the field `TFC:Keeper`. Written from the format as Bitwarden's importer
//! reads it, not from an export Keeper made.

use super::shared::{cell, label_of, parse_csv, present, sealed, Sink};
use super::{Imported, ReadError, Source};

/// Whether a CSV that no other reader knew is Keeper's: no header, every row at least the seven
/// columns, a title in each, and a website column that is empty or looks like an address.
pub(super) fn read(text: &str) -> Option<Result<Imported, ReadError>> {
    let t = parse_csv(text, false).ok()?;
    if t.rows.is_empty() || t.rows.iter().any(|r| r.len() < 6 || cell(r, Some(1)).is_empty()) {
        return None;
    }
    let addresses = t.rows.iter().filter(|r| {
        let w = cell(r, Some(4));
        w.is_empty() || w.contains('.') || w.contains("://")
    });
    if addresses.count() * 2 < t.rows.len() {
        return None;
    }
    let mut sink = Sink::default();
    for row in &t.rows {
        let c = |i: usize| cell(row, Some(i));
        let mut totp = String::new();
        let mut kept: Vec<String> = Vec::new();
        let mut i = 7;
        while i < row.len() {
            let (name, value) = (c(i), c(i + 1));
            if name == "TFC:Keeper" {
                totp = value.to_string();
            } else if let Some(v) = present(value) {
                kept.push(if name.is_empty() { v.to_string() } else { format!("{name}: {v}") });
            }
            i += 2;
        }
        let label = label_of(c(1), c(4), c(2));
        let secret = sealed(&[("", c(5)), ("", &kept.join("\n"))]);
        let password = row.get(3).unwrap_or("");
        let tag = c(0).replace('\\', "/");
        sink.login(&label, c(2), password, c(4), &totp, &secret, tag.trim_matches('/'));
    }
    Some(sink.done(Source::Keeper))
}
