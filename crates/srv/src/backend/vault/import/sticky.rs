// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Sticky Password's XML export (spec §8.4): `root > Database` with its `Logins` (a username and a
//! password each), its `Accounts` (the title, address, notes and group of a sign-in, linked to its
//! login by `SourceLoginID`) and its `Groups` (folders, which nest). Written from the format as
//! Bitwarden's open-source importer reads it, not from an export Sticky Password made. Only the
//! logins are read; what else the export may hold (memos, bank cards) is not.

use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::shared::{add_line, label_of, Sink};
use super::{Imported, ReadError, Source, MAX_ITEMS};

/// Elements nest only a few deep in this export; more is a crafted file.
const MOST_DEPTH: usize = 64;
/// Groups nest; a chain longer than this is a loop.
const MOST_GROUP_DEPTH: usize = 32;

/// Whether this text is a Sticky Password export, from what its first lines hold.
pub(super) fn looks_like(text: &str) -> bool {
    let trimmed = text.trim_start();
    let mut end = trimmed.len().min(4096);
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    trimmed.starts_with('<') && trimmed[..end].contains("<Database")
}

struct Login {
    id: String,
    username: String,
    password: String,
}

#[derive(Default)]
struct Account {
    title: String,
    link: String,
    group: String,
    notes: String,
    logins: Vec<String>,
}

fn attr(e: &BytesStart, name: &str) -> String {
    for a in e.attributes().flatten() {
        if a.key.as_ref() == name.as_bytes() {
            return match a.unescape_value() {
                Ok(v) => v.into_owned(),
                Err(_) => String::from_utf8_lossy(&a.value).into_owned(),
            };
        }
    }
    String::new()
}

pub(super) fn read(text: &str) -> Result<Imported, ReadError> {
    let mut reader = Reader::from_str(text);
    let mut path: Vec<String> = Vec::new();
    let mut logins: Vec<Login> = Vec::new();
    let mut accounts: Vec<Account> = Vec::new();
    let mut open: Option<Account> = None;
    // A group's id: its name and its parent's id.
    let mut groups: HashMap<String, (String, String)> = HashMap::new();
    loop {
        let event = reader.read_event().map_err(|_| ReadError::from("this isn't a well-formed XML file"))?;
        let (e, empty) = match event {
            Event::Eof => break,
            Event::End(_) => {
                if path.join("/") == "root/Database/Accounts/Account" {
                    accounts.extend(open.take());
                }
                path.pop();
                continue;
            }
            Event::Start(e) => (e, false),
            Event::Empty(e) => (e, true),
            _ => continue,
        };
        if path.len() >= MOST_DEPTH {
            return Err("this XML file nests deeper than a Sticky Password export does".into());
        }
        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
        let here = if path.is_empty() { name.clone() } else { format!("{}/{name}", path.join("/")) };
        match here.as_str() {
            "root/Database/Logins/Login" => {
                if logins.len() >= MAX_ITEMS {
                    return Err(format!("this export has more than {MAX_ITEMS} logins, more than the Vault imports").into());
                }
                logins.push(Login { id: attr(&e, "ID"), username: attr(&e, "Name"), password: attr(&e, "Password") });
            }
            "root/Database/Accounts/Account" => {
                if accounts.len() >= MAX_ITEMS {
                    return Err(format!("this export has more than {MAX_ITEMS} accounts, more than the Vault imports").into());
                }
                let account = Account {
                    title: attr(&e, "Name"),
                    link: attr(&e, "Link"),
                    group: attr(&e, "ParentID"),
                    // The export writes a line break in a note as `/n`.
                    notes: attr(&e, "Comments").replace("/n", "\n"),
                    logins: Vec::new(),
                };
                if empty {
                    accounts.push(account);
                } else {
                    open = Some(account);
                }
            }
            "root/Database/Accounts/Account/LoginLinks/Login" => {
                if let Some(account) = open.as_mut() {
                    if account.logins.len() < MAX_ITEMS {
                        account.logins.push(attr(&e, "SourceLoginID"));
                    }
                }
            }
            "root/Database/Groups/Group" => {
                if groups.len() < MAX_ITEMS {
                    groups.insert(attr(&e, "ID"), (attr(&e, "Name"), attr(&e, "ParentID")));
                }
            }
            _ => {}
        }
        if !empty {
            path.push(name);
        }
    }
    if logins.is_empty() {
        return Err("this isn't a Sticky Password export, or it has no logins".into());
    }
    // A login's account: the first one that links to it.
    let mut account_of: HashMap<&str, &Account> = HashMap::new();
    for account in &accounts {
        for id in &account.logins {
            account_of.entry(id.as_str()).or_insert(account);
        }
    }
    let mut sink = Sink::default();
    for login in &logins {
        if login.id.trim().is_empty() {
            continue;
        }
        let account = account_of.get(login.id.as_str()).copied();
        let (title, link, notes, group) = match account {
            Some(a) => (a.title.trim(), a.link.trim(), a.notes.as_str(), group_path(&groups, &a.group)),
            None => ("", "", "", String::new()),
        };
        let mut secret = String::new();
        add_line(&mut secret, "", notes);
        sink.login(&label_of(title, link, &login.username), &login.username, &login.password, link, "", &secret, &group);
    }
    sink.done(Source::StickyPassword)
}

/// A group's path from the top (`Work/Servers`), found by walking up its parents.
fn group_path(groups: &HashMap<String, (String, String)>, id: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut next = id;
    while !next.is_empty() && parts.len() < MOST_GROUP_DEPTH {
        let Some((name, parent)) = groups.get(next) else { break };
        if !name.trim().is_empty() {
            parts.push(name.trim());
        }
        next = parent;
    }
    parts.reverse();
    parts.join("/")
}
