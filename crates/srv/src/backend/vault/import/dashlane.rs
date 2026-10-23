// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Dashlane's CSV export (spec §8.4). It is a zip of several CSVs, one for each kind of item
//! (credentials, payments, IDs, personal information, secure notes), and each is imported on its
//! own: AgentMux tells them apart by their columns, not their names. Written from the format as
//! Bitwarden's importer reads it, not from an export Dashlane made.

use super::shared::{cell, header_names, label_of, month_year, parse_csv, present, sealed, Sink, Table};
use super::{Imported, ReadError, Source};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Part {
    Credentials,
    Payments,
    Ids,
    Personal,
    Notes,
}

/// Which of Dashlane's files this is, from its normalised column names.
pub(super) fn which(names: &[String]) -> Option<Part> {
    let has = |n: &str| names.iter().any(|x| x == n);
    if has("title") && has("username2") && has("password") {
        Some(Part::Credentials)
    } else if has("type") && has("ccnumber") {
        Some(Part::Payments)
    } else if has("type") && has("number") && has("placeofissue") {
        Some(Part::Ids)
    } else if has("type") && has("firstname") && has("lastname") {
        Some(Part::Personal)
    } else if names.len() == 2 && has("title") && has("note") {
        Some(Part::Notes)
    } else {
        None
    }
}

pub(super) fn read(text: &str) -> Option<Result<Imported, ReadError>> {
    let part = which(&header_names(text))?;
    Some(parse_csv(text, true).and_then(|t| read_table(&t, part)))
}

fn read_table(t: &Table, part: Part) -> Result<Imported, ReadError> {
    let mut sink = Sink::default();
    for row in &t.rows {
        let c = |name: &str| t.get(row, name);
        match part {
            Part::Credentials => {
                let (url, title) = (c("url"), c("title"));
                let username = present(c("username")).or_else(|| present(c("username2"))).unwrap_or("");
                let label = label_of(title, url, username);
                let totp = present(c("otpurl")).or_else(|| present(c("otpsecret"))).unwrap_or("");
                let secret = sealed(&[("", c("note")), ("username2", c("username2")), ("username3", c("username3"))]);
                // The password is kept as exported.
                let password = t.col(&["password"]).and_then(|i| row.get(i)).unwrap_or("");
                sink.login(&label, username, password, url, totp, &secret, c("category"));
            }
            Part::Payments => {
                let name = c("accountname");
                let holder = present(c("accountholder")).unwrap_or(name);
                let secret = sealed(&[("issuing bank", c("issuingbank")), ("country", c("country"))]);
                if c("type").eq_ignore_ascii_case("bank") {
                    let label = [name, c("issuingbank")].into_iter().find(|s| !s.is_empty()).unwrap_or("Dashlane bank account");
                    sink.bank(label, c("accountnumber"), c("routingnumber"), c("issuingbank"), holder, "", "");
                } else {
                    let label = if name.is_empty() { "Dashlane card" } else { name };
                    sink.card(label, c("ccnumber"), month_year(c("expirationmonth"), c("expirationyear")), c("code"), holder, &secret, "");
                }
            }
            Part::Ids => {
                let (what, kind) = match c("type").to_ascii_lowercase().as_str() {
                    "passport" => ("passport", "passport"),
                    "license" => ("driver's licence", "drivers-licence"),
                    "social_security" => ("Social Security number", "ssn"),
                    "tax_number" => ("tax number", "tin"),
                    _ => ("ID card", "other-id"),
                };
                let name = c("name");
                let label = if name.is_empty() { what.to_string() } else { format!("{name} ({what})") };
                let issuer = [c("placeofissue"), c("state")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
                sink.id(kind, &label, c("number"), name, &issuer, c("expirationdate"), "");
            }
            Part::Personal => {
                // Names, addresses and phone numbers have no kind of their own: each row is kept
                // whole, sealed, as a note. A date of birth has one.
                let kind = c("type");
                let mut text = String::new();
                for (i, h) in t.headers.iter().enumerate() {
                    if i != t.col(&["type"]).unwrap_or(usize::MAX) {
                        if let Some(v) = present(cell(row, Some(i))) {
                            text.push_str(&format!("{h}: {v}\n"));
                        }
                    }
                }
                let full = ["firstname", "middlename", "lastname"].iter().filter_map(|n| present(c(n))).collect::<Vec<_>>().join(" ");
                let detail = [c("itemname"), full.as_str(), c("email"), c("phonenumber"), c("address"), c("url")].into_iter().find(|s| !s.is_empty()).unwrap_or("");
                let label = if detail.is_empty() { format!("Dashlane {kind}") } else { format!("{detail} ({kind})") };
                if let Some(dob) = present(c("dateofbirth")) {
                    sink.id("date-of-birth", &format!("{full} (date of birth)"), dob, &full, "", "", "");
                }
                sink.note(&label, text.trim_end(), "");
            }
            Part::Notes => sink.note(c("title"), c("note"), ""),
        }
    }
    sink.done(Source::Dashlane)
}
