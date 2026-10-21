// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The CSV files password managers and browsers export: LastPass, 1Password,
//! Chrome, Edge, Brave, Firefox, Safari and Apple Passwords all write a table of
//! logins (spec §8.4), and LastPass also writes its secure notes into it. A CSV no
//! reader knows is imported with a column mapping the person chooses.
//!
//! The columns are found by their names, not their order, since exports differ and
//! change between versions.

use super::{Imported, Incoming, Mapping, MappingNeeded, ReadError, Source, MAX_ITEMS};
use std::collections::BTreeMap;

/// A column's lowercase name without spaces, dashes or underscores.
fn norm(name: &str) -> String {
    name.trim().trim_start_matches('\u{feff}').chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

/// The columns a login table can have, found by their names.
#[derive(Default)]
struct Columns {
    label: Option<usize>,
    site: Option<usize>,
    username: Option<usize>,
    password: Option<usize>,
    totp: Option<usize>,
    notes: Option<usize>,
    folder: Option<usize>,
    /// LastPass's secure notes are rows whose url is `http://sn`; their fields are in `extra`.
    extra: Option<usize>,
}

impl Columns {
    fn of(headers: &[String]) -> Self {
        let find = |names: &[&str]| headers.iter().position(|h| names.contains(&norm(h).as_str()));
        Columns {
            label: find(&["title", "name"]),
            site: find(&["url", "website", "loginuri", "weburl", "site"]),
            username: find(&["username", "loginusername", "user", "login"]),
            password: find(&["password", "loginpassword"]),
            totp: find(&["totp", "otp", "otpauth", "onetimepassword", "logintotp"]),
            notes: find(&["notes", "note"]),
            folder: find(&["grouping", "folder", "group", "tags"]),
            extra: find(&["extra"]),
        }
    }
}

/// Which export it is, from its column names.
fn source_of(headers: &[String]) -> Source {
    let names: Vec<String> = headers.iter().map(|h| norm(h)).collect();
    let has = |n: &str| names.iter().any(|x| x == n);
    if has("grouping") && has("extra") {
        Source::LastPass
    } else if has("httprealm") || has("formactionorigin") {
        Source::Firefox
    } else if has("otpauth") && has("title") {
        Source::Safari
    } else if has("title") && (has("website") || has("url")) && (has("type") || has("archived") || has("favorite") || has("tags")) {
        Source::OnePassword
    } else if has("name") && has("url") && has("username") && has("password") {
        Source::Chrome
    } else {
        Source::Csv
    }
}

fn cell<'a>(row: &'a csv::StringRecord, col: Option<usize>) -> &'a str {
    col.and_then(|i| row.get(i)).map(str::trim).unwrap_or("")
}

/// A host for a label when a row has no title.
fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or("").trim_start_matches("www.").to_string()
}

pub fn read(text: &str, mapping: Option<&Mapping>) -> Result<Imported, ReadError> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_reader(text.as_bytes());
    let headers: Vec<String> = reader.headers().map_err(|e| ReadError::from(format!("this isn't a CSV file: {e}")))?.iter().map(str::to_string).collect();
    if headers.len() < 2 {
        return Err("this doesn't look like a CSV file of passwords: it has one column".into());
    }
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| ReadError::from(format!("couldn't read a row of the CSV: {e}")))?;
        if rows.len() >= MAX_ITEMS {
            return Err(format!("this file has more than {MAX_ITEMS} rows, more than the Vault imports").into());
        }
        rows.push(record);
    }
    if let Some(m) = mapping {
        return mapped(&rows, &headers, m);
    }
    let cols = Columns::of(&headers);
    if cols.password.is_none() || (cols.site.is_none() && cols.label.is_none()) {
        return Err(ReadError::NeedsMapping(MappingNeeded { columns: headers, rows: rows.len() }));
    }
    let source = source_of(&headers);
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    for row in &rows {
        let site = cell(row, cols.site);
        let title = cell(row, cols.label);
        let label = if !title.is_empty() { title.to_string() } else if !host_of(site).is_empty() { host_of(site) } else { cell(row, cols.username).to_string() };
        let label = if label.is_empty() { "(unnamed)".to_string() } else { label };
        let folder = cell(row, cols.folder);
        if source == Source::LastPass && site == "http://sn" {
            match lastpass_note(&label, cell(row, cols.extra)) {
                Ok(made) => entries.extend(made.into_iter().map(|e| e.tag(folder))),
                Err(why) => skipped.push((label, why)),
            }
            continue;
        }
        // A password is kept exactly as exported, spaces and all.
        let password = cols.password.and_then(|i| row.get(i)).unwrap_or("");
        if password.is_empty() {
            skipped.push((label, "it has no password".to_string()));
            continue;
        }
        // LastPass keeps a login's notes in `extra`.
        let notes_col = cols.notes.or(if source == Source::LastPass { cols.extra } else { None });
        let mut login = Incoming::new("login", &label).part("password", password).field("site", site).field("username", cell(row, cols.username)).tag(folder);
        let totp = cell(row, cols.totp);
        if !totp.is_empty() {
            login = login.part("totp", totp);
        }
        entries.push(login);
        // Notes can hold secrets, so they are kept sealed in a note of their own.
        let notes = cell(row, notes_col);
        if !notes.is_empty() {
            entries.push(Incoming::new("note", &format!("{label} (notes)")).part("text", notes).tag(folder));
        }
    }
    Ok(Imported { source, entries, skipped, plaintext: true })
}

/// Rows read with the person's own column choices.
fn mapped(rows: &[csv::StringRecord], headers: &[String], m: &Mapping) -> Result<Imported, ReadError> {
    let kind = super::super::kinds::kind(&m.kind).ok_or_else(|| ReadError::from(format!("unknown kind {:?}", m.kind)))?;
    if headers.len() <= m.label.max(m.value) || m.site.is_some_and(|i| i >= headers.len()) || m.username.is_some_and(|i| i >= headers.len()) {
        return Err("a chosen column isn't in the file".into());
    }
    if kind.name == "login" && m.site.is_none() && m.username.is_none() {
        return Err("a login needs a site or a username column as well as its password".into());
    }
    let main = kind.parts[0];
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    for row in rows {
        let (label, value) = (cell(row, Some(m.label)), cell(row, Some(m.value)));
        let label = if label.is_empty() { "(unnamed)" } else { label };
        if value.is_empty() {
            skipped.push((label.to_string(), format!("its {main} column is empty")));
            continue;
        }
        let mut e = Incoming::new(kind.name, label).part(main, value);
        if kind.fields.contains(&"site") {
            e = e.field("site", cell(row, m.site));
        }
        if kind.fields.contains(&"username") {
            e = e.field("username", cell(row, m.username));
        }
        entries.push(e);
    }
    Ok(Imported { source: Source::Csv, entries, skipped, plaintext: true })
}

// ── LastPass secure notes ───────────────────────────────────────────────────

/// Every line that starts one of a secure note's fields, whatever its type.
const LASTPASS_KEYS: &[&str] = &[
    "NoteType",
    "Language",
    "Name on Card",
    "Type",
    "Number",
    "Security Code",
    "Start Date",
    "Expiration Date",
    "Notes",
    "Bank Name",
    "Account Type",
    "Routing Number",
    "Account Number",
    "SWIFT Code",
    "IBAN Number",
    "Pin",
    "Branch Address",
    "Branch Phone",
    "Name",
    "Issuing Authority",
    "Date of Birth",
    "Sex",
    "Height",
    "Issue Date",
    "Hostname",
    "Username",
    "Password",
    "Port",
];

/// The `Key:Value` fields of a secure note's `extra`: a line that starts with a
/// known key begins a field; any other line continues the one before.
fn lastpass_fields(extra: &str) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in extra.lines() {
        let key = LASTPASS_KEYS.iter().find(|k| line.strip_prefix(**k).is_some_and(|rest| rest.starts_with(':')));
        match (key, &current) {
            (Some(k), _) => {
                out.insert((*k).to_string(), line[k.len() + 1..].trim().to_string());
                current = Some((*k).to_string());
            }
            (None, Some(k)) => {
                let v = out.entry(k.clone()).or_default();
                v.push('\n');
                v.push_str(line);
            }
            (None, None) => {}
        }
    }
    out
}

/// `January,2030` as `01/2030`.
fn month_year(s: &str) -> String {
    const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    let mut parts = s.split(',');
    match (parts.next().map(|m| m.trim().to_lowercase()), parts.next().map(str::trim)) {
        (Some(m), Some(y)) if !y.is_empty() => match MONTHS.iter().position(|x| *x == m) {
            Some(i) => format!("{:02}/{y}", i + 1),
            None => s.trim().to_string(),
        },
        _ => s.trim().to_string(),
    }
}

/// A secure note by its `NoteType`: the kinds the Vault has, else a note holding
/// every field.
fn lastpass_note(label: &str, extra: &str) -> Result<Vec<Incoming>, String> {
    let f = lastpass_fields(extra);
    let get = |k: &str| f.get(k).map(|v| v.trim()).unwrap_or("");
    let notes = get("Notes");
    let mut made: Vec<Incoming> = match get("NoteType") {
        "Credit Card" if !get("Number").is_empty() => {
            let mut e = Incoming::new("card", label).part("number", get("Number"));
            for (part, key) in [("cvc", "Security Code"), ("name", "Name on Card")] {
                if !get(key).is_empty() {
                    e = e.part(part, get(key));
                }
            }
            if !get("Expiration Date").is_empty() {
                e = e.part("expiry", &month_year(get("Expiration Date")));
            }
            vec![e]
        }
        "Bank Account" if !get("Account Number").is_empty() => {
            let mut e = Incoming::new("bank-account", label).part("account", get("Account Number")).field("bank", get("Bank Name"));
            if !get("Routing Number").is_empty() {
                e = e.part("routing", get("Routing Number"));
            }
            vec![e]
        }
        "Social Security" if !get("Number").is_empty() => vec![Incoming::new("ssn", label).part("value", get("Number")).field("owner", get("Name"))],
        "Driver's License" if !get("Number").is_empty() => {
            vec![Incoming::new("drivers-licence", label).part("value", get("Number")).field("owner", get("Name")).field("expires", &month_year(get("Expiration Date")))]
        }
        "Passport" if !get("Number").is_empty() => {
            vec![Incoming::new("passport", label).part("value", get("Number")).field("owner", get("Name")).field("issuer", get("Issuing Authority")).field("expires", &month_year(get("Expiration Date")))]
        }
        _ => {
            // Every other type (an address, a server, an SSH key, a plain note) is kept
            // whole, line for line, without the two lines that only name its type.
            let text = extra.lines().filter(|l| !l.starts_with("NoteType:") && !l.starts_with("Language:")).collect::<Vec<_>>().join("\n");
            if text.trim().is_empty() {
                return Err("the secure note is empty".to_string());
            }
            return Ok(vec![Incoming::new("note", label).part("text", &text)]);
        }
    };
    if !notes.is_empty() {
        made.push(Incoming::new("note", &format!("{label} (notes)")).part("text", notes));
    }
    Ok(made)
}
