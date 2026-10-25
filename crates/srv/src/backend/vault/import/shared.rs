// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the readers of the second wave of services share (Dashlane, Keeper, NordPass, RoboForm,
//! Zoho Vault, and the others of spec §12 P6): reading a table, and making entries the way the
//! first wave does. A login's notes and custom fields can hold recovery codes, so they are sealed
//! in a note of their own; a folder becomes a tag; a row with nothing to keep is listed as
//! skipped with the reason.

pub(super) use super::logins::{cell, host_of, norm};
use super::{Imported, Incoming, ReadError, Source, MAX_ITEMS};

/// A CSV file: its header row (empty when the service writes none) and its rows.
pub(super) struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<csv::StringRecord>,
}

pub(super) fn parse_csv(text: &str, has_header: bool) -> Result<Table, ReadError> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).has_headers(has_header).from_reader(text.as_bytes());
    let headers: Vec<String> = if has_header {
        reader.headers().map_err(|e| ReadError::from(format!("this isn't a CSV file: {e}")))?.iter().map(str::to_string).collect()
    } else {
        Vec::new()
    };
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| ReadError::from(format!("couldn't read a row of the CSV: {e}")))?;
        if rows.len() >= MAX_ITEMS {
            return Err(format!("this file has more than {MAX_ITEMS} rows, more than the Vault imports").into());
        }
        rows.push(record);
    }
    Ok(Table { headers, rows })
}

impl Table {
    /// A row's cell in the column with this name, trimmed; empty when the file has no such column.
    pub fn get<'a>(&self, row: &'a csv::StringRecord, name: &str) -> &'a str {
        cell(row, self.col(&[name]))
    }

    /// The first column with one of these names (compared without case, spaces, dashes or underscores).
    pub fn col(&self, names: &[&str]) -> Option<usize> {
        self.headers.iter().position(|h| names.contains(&norm(h).as_str()))
    }
}

/// A header row's names, normalised, for telling which service wrote the file.
pub(super) fn header_names(text: &str) -> Vec<String> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(text.as_bytes());
    reader.headers().map(|h| h.iter().map(norm).collect()).unwrap_or_default()
}

pub(super) fn present(s: &str) -> Option<&str> {
    let t = s.trim();
    (!t.is_empty()).then_some(t)
}

/// `MM/YYYY` from a month and a year written apart (`1` and `2030`, or `01` and `30`).
pub(super) fn month_year(month: &str, year: &str) -> Option<String> {
    let m: u32 = month.trim().parse().ok().filter(|m| (1..=12).contains(m))?;
    let y: u32 = year.trim().parse().ok()?;
    let y = if y < 100 { 2000 + y } else { y };
    (1000..=9999).contains(&y).then(|| format!("{m:02}/{y}"))
}

/// `MM/YYYY` from an expiry written `01 / 22`, `1/2030` or `03/23`.
pub(super) fn expiry(s: &str) -> Option<String> {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let (m, y) = compact.split_once('/')?;
    month_year(m, y)
}

/// Text to seal: each non-empty value on its own line, as `name: value` (or just the value when
/// the name is empty).
pub(super) fn sealed(parts: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (name, value) in parts {
        let Some(v) = present(value) else { continue };
        if !out.is_empty() {
            out.push('\n');
        }
        if name.is_empty() {
            out.push_str(v);
        } else {
            out.push_str(&format!("{name}: {v}"));
        }
    }
    out
}

/// A label for a row that has no title: its host, else its username.
pub(super) fn label_of(title: &str, site: &str, username: &str) -> String {
    let host = host_of(site);
    let label = [title.trim(), host.as_str(), username.trim()].into_iter().find(|s| !s.is_empty()).unwrap_or("(unnamed)").to_string();
    label
}

/// What a reader collects.
#[derive(Default)]
pub(super) struct Sink {
    pub entries: Vec<Incoming>,
    pub skipped: Vec<(String, String)>,
}

impl Sink {
    pub fn skip(&mut self, label: &str, why: &str) {
        self.skipped.push((label.to_string(), why.to_string()));
    }

    /// A login. `secret` is the text to seal beside it (notes, custom fields). A row with no
    /// password but something to keep is kept as a note, with where and who it was filed under; a
    /// row with nothing is listed as skipped.
    pub fn login(&mut self, label: &str, username: &str, password: &str, site: &str, totp: &str, secret: &str, tag: &str) {
        if password.is_empty() {
            if secret.trim().is_empty() {
                self.skip(label, "it has no password");
                return;
            }
            let mut text = String::new();
            for (name, value) in [("site", site), ("username", username)] {
                if let Some(v) = present(value) {
                    text.push_str(&format!("{name}: {v}\n"));
                }
            }
            text.push_str(secret);
            self.entries.push(Incoming::new("note", label).part("text", &text).tag(tag));
            return;
        }
        // A password is kept exactly as exported, spaces and all.
        let mut login = Incoming::new("login", label).part("password", password).field("site", site).field("username", username).tag(tag);
        if let Some(t) = present(totp) {
            login = login.part("totp", t);
        }
        self.entries.push(login);
        self.notes_of(label, secret, tag);
    }

    /// The text sealed beside another entry, in a note named for it.
    pub fn notes_of(&mut self, label: &str, secret: &str, tag: &str) {
        if !secret.trim().is_empty() {
            self.entries.push(Incoming::new("note", &format!("{label} (notes)")).part("text", secret).tag(tag));
        }
    }

    pub fn note(&mut self, label: &str, text: &str, tag: &str) {
        if text.trim().is_empty() {
            self.skip(label, "the note is empty");
        } else {
            self.entries.push(Incoming::new("note", label).part("text", text).tag(tag));
        }
    }

    pub fn card(&mut self, label: &str, number: &str, expiry: Option<String>, cvc: &str, holder: &str, secret: &str, tag: &str) {
        let Some(number) = present(number) else {
            self.skip(label, "the card has no number");
            return;
        };
        let mut e = Incoming::new("card", label).part("number", number);
        if let Some(x) = expiry {
            e = e.part("expiry", &x);
        }
        for (part, value) in [("cvc", cvc), ("name", holder)] {
            if let Some(v) = present(value) {
                e = e.part(part, v);
            }
        }
        self.entries.push(e.tag(tag));
        self.notes_of(label, secret, tag);
    }

    pub fn bank(&mut self, label: &str, account: &str, routing: &str, bank: &str, owner: &str, secret: &str, tag: &str) {
        let Some(account) = present(account) else {
            self.skip(label, "the account has no number");
            return;
        };
        let mut e = Incoming::new("bank-account", label).part("account", account).field("bank", bank).field("owner", owner);
        if let Some(r) = present(routing) {
            e = e.part("routing", r);
        }
        self.entries.push(e.tag(tag));
        self.notes_of(label, secret, tag);
    }

    /// An identity number of a kind the Vault has (`ssn`, `passport`, `drivers-licence`, …).
    pub fn id(&mut self, kind: &str, label: &str, value: &str, owner: &str, issuer: &str, expires: &str, tag: &str) {
        let Some(value) = present(value) else {
            self.skip(label, "it has no number");
            return;
        };
        let mut e = Incoming::new(kind, label).part("value", value).field("owner", owner);
        if kind != "ssn" && kind != "tin" && kind != "date-of-birth" {
            e = e.field("issuer", issuer).field("expires", expires);
        }
        self.entries.push(e.tag(tag));
    }

    pub fn done(self, source: Source) -> Result<Imported, ReadError> {
        Ok(Imported { source, entries: self.entries, skipped: self.skipped, plaintext: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiries_are_written_month_slash_year() {
        assert_eq!(expiry("01 / 22").as_deref(), Some("01/2022"));
        assert_eq!(expiry("1/2030").as_deref(), Some("01/2030"));
        assert_eq!(expiry("13/2030"), None);
        assert_eq!(expiry("soon"), None);
        assert_eq!(month_year("3", "2031").as_deref(), Some("03/2031"));
        assert_eq!(month_year("", ""), None);
    }

    #[test]
    fn sealed_text_skips_what_is_empty() {
        assert_eq!(sealed(&[("", "a note"), ("pin", " "), ("code", "1234")]), "a note\ncode: 1234");
    }
}
