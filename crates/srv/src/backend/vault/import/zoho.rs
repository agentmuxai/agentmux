// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Zoho Vault's CSV export (spec §8.4): `Password Name, Description, Password URL, SecretData,
//! Notes, CustomData, Tags, Classification, Favorite, TOTP, Folder Name`. The username and
//! password are not columns: they sit in `SecretData` as `Key:Value` lines. Written from the
//! format as Bitwarden's importer reads it, not from an export Zoho made.

use super::shared::{cell, label_of, parse_csv, sealed, Sink};
use super::{Imported, ReadError, Source};

pub(super) fn looks_like(names: &[String]) -> bool {
    let has = |n: &str| names.iter().any(|x| x == n);
    has("secretdata") && (has("passwordname") || has("secretname"))
}

/// The `Key:Value` lines of a cell. A line with no key continues the value before it.
fn pairs(data: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in data.lines() {
        match line.split_once(':') {
            // A key is short and isn't the scheme of an address (`https://…`); a password that
            // starts with `//` is still the value of `Password`.
            Some((k, v)) if !k.trim().is_empty() && k.len() <= 40 && !(v.starts_with("//") && URL_SCHEMES.contains(&k.trim().to_ascii_lowercase().as_str())) => {
                out.push((k.trim().to_string(), v.to_string()));
            }
            _ => match out.last_mut() {
                Some((_, v)) => {
                    v.push('\n');
                    v.push_str(line);
                }
                None if !line.trim().is_empty() => out.push((String::new(), line.to_string())),
                None => {}
            },
        }
    }
    out
}

const URL_SCHEMES: &[&str] = &["http", "https", "ftp", "ftps", "sftp", "ssh", "ws", "wss", "file"];
const USER_KEYS: &[&str] = &["user name", "username", "login", "login name", "user", "user id", "userid", "email"];
const PASSWORD_KEYS: &[&str] = &["password", "passphrase", "pin", "key"];

pub(super) fn read(text: &str) -> Result<Imported, ReadError> {
    let t = parse_csv(text, true)?;
    let mut sink = Sink::default();
    for row in &t.rows {
        let c = |names: &[&str]| cell(row, t.col(names));
        let (name, url) = (c(&["passwordname", "secretname"]), c(&["passwordurl", "secreturl"]));
        if name.is_empty() && url.is_empty() {
            continue;
        }
        let (mut username, mut password) = (String::new(), String::new());
        let mut kept: Vec<String> = Vec::new();
        for (k, v) in pairs(c(&["secretdata"])).into_iter().chain(pairs(c(&["customdata"]))) {
            let lower = k.to_ascii_lowercase();
            if lower == "secrettype" {
                continue;
            }
            if USER_KEYS.contains(&lower.as_str()) && username.is_empty() {
                username = v.trim().to_string();
            } else if PASSWORD_KEYS.contains(&lower.as_str()) && password.is_empty() {
                password = v;
            } else if !v.trim().is_empty() {
                kept.push(if k.is_empty() { v.trim().to_string() } else { format!("{k}: {}", v.trim()) });
            }
        }
        let label = label_of(name, url, &username);
        let secret = sealed(&[("", c(&["notes"])), ("", &kept.join("\n"))]);
        sink.login(&label, &username, &password, url, c(&["totp", "logintotp"]), &secret, c(&["foldername"]));
    }
    sink.done(Source::ZohoVault)
}
