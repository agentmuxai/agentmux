// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Named browser profiles: the list of them, in `shared_dir`
//! (`~/.agentmux/shared/browser-profiles.json`), like the bookmarks
//! (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §3, §6). A
//! profile's cookie jar is a disk-backed CEF profile the host keeps at
//! `<cef-cache>/profile-<id>`; this file only names and colours them.
//! "Personal" (the shared jar every tab had before profiles) isn't listed:
//! it always exists and can't be deleted.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct BrowserProfile {
    /// Stable id, `[a-z0-9-]`, used in `browser:identity` (`profile:<id>`)
    /// and the host's profile folder. Never reused.
    pub id: String,
    pub name: String,
    /// `#rrggbb`: the profile's colour on its button and tabs.
    pub color: String,
    #[serde(default)]
    #[ts(type = "number")]
    pub created_at: i64,
    /// "Agents may use this profile": an agent's `OpenBrowser` may browse as
    /// it. Off until the user switches it on.
    #[serde(default)]
    pub agents_allowed: bool,
}

const PROFILES_FILE_NAME: &str = "browser-profiles.json";
pub const MAX_PROFILES: usize = 20;
pub const MAX_NAME_CHARS: usize = 40;

/// Colours a new profile is given in turn, unless one is chosen.
pub const PALETTE: [&str; 8] = ["#3b82f6", "#22c55e", "#f59e0b", "#ef4444", "#a855f7", "#14b8a6", "#ec4899", "#64748b"];

pub fn profiles_file_path() -> Option<PathBuf> {
    agentmux_common::DataPaths::from_env().map(|p| p.shared_dir.join(PROFILES_FILE_NAME))
}

/// The list. A missing or empty file is "no profiles yet"; a present but
/// unparseable one is an error, not an empty list that would look like the
/// profiles vanished.
pub fn read_profiles(path: &Path) -> Result<Vec<BrowserProfile>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path).map_err(|e| format!("read browser profiles: {e}"))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&content).map_err(|e| format!("parse browser profiles: {e}"))
}

/// Replace the list, through a temp file and a rename, so a crash never
/// leaves a half-written file.
pub fn write_profiles(path: &Path, profiles: &[BrowserProfile]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(profiles).map_err(|e| format!("serialize browser profiles: {e}"))?;
    let tmp = path.with_file_name(format!(".{PROFILES_FILE_NAME}.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, &json).map_err(|e| format!("write browser profiles: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename browser profiles: {e}")
    })
}

/// Is `id` a well-formed profile id?
pub fn is_profile_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn check_name(profiles: &[BrowserProfile], name: &str, except: Option<&str>) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a profile needs a name".to_string());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("a profile's name is at most {MAX_NAME_CHARS} characters"));
    }
    if ["personal", "incognito"].contains(&name.to_lowercase().as_str()) {
        return Err(format!("{name:?} is taken: pick another name"));
    }
    if profiles.iter().any(|p| Some(p.id.as_str()) != except && p.name.eq_ignore_ascii_case(name)) {
        return Err(format!("there is already a profile called {name:?}"));
    }
    Ok(name.to_string())
}

fn check_color(color: &str) -> Result<String, String> {
    let c = color.trim().to_lowercase();
    let hex = c.strip_prefix('#').unwrap_or("");
    if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(c)
    } else {
        Err(format!("{color:?} isn't a colour (#rrggbb)"))
    }
}

/// A new profile added to `profiles`: a fresh id, a checked name, and the
/// given colour or the next one from the palette.
pub fn add(profiles: &mut Vec<BrowserProfile>, name: &str, color: Option<&str>, now_ms: i64) -> Result<BrowserProfile, String> {
    if profiles.len() >= MAX_PROFILES {
        return Err(format!("at most {MAX_PROFILES} profiles"));
    }
    let name = check_name(profiles, name, None)?;
    let color = match color {
        Some(c) => check_color(c)?,
        None => PALETTE[profiles.len() % PALETTE.len()].to_string(),
    };
    let id = format!("p-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let profile = BrowserProfile { id, name, color, created_at: now_ms, agents_allowed: false };
    profiles.push(profile.clone());
    Ok(profile)
}

/// Rename or recolour profile `id`.
pub fn update(
    profiles: &mut [BrowserProfile],
    id: &str,
    name: Option<&str>,
    color: Option<&str>,
    agents_allowed: Option<bool>,
) -> Result<(), String> {
    let name = name.map(|n| check_name(profiles, n, Some(id))).transpose()?;
    let color = color.map(check_color).transpose()?;
    let p = profiles.iter_mut().find(|p| p.id == id).ok_or_else(|| format!("no profile {id:?}"))?;
    if let Some(n) = name {
        p.name = n;
    }
    if let Some(c) = color {
        p.color = c;
    }
    if let Some(a) = agents_allowed {
        p.agents_allowed = a;
    }
    Ok(())
}

/// Remove profile `id`. Its tabs and its jar are the caller's to close.
pub fn remove(profiles: &mut Vec<BrowserProfile>, id: &str) -> Result<(), String> {
    let before = profiles.len();
    profiles.retain(|p| p.id != id);
    if profiles.len() == before {
        return Err(format!("no profile {id:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_added_named_and_coloured() {
        let mut ps = Vec::new();
        let work = add(&mut ps, "  Work ", None, 1).unwrap();
        assert_eq!(work.name, "Work");
        assert_eq!(work.color, PALETTE[0]);
        assert!(is_profile_id(&work.id));
        let client = add(&mut ps, "Client X", Some("#ABCDEF"), 2).unwrap();
        assert_eq!(client.color, "#abcdef");
        assert_ne!(work.id, client.id);
    }

    #[test]
    fn bad_names_and_colours_are_refused() {
        let mut ps = Vec::new();
        add(&mut ps, "Work", None, 1).unwrap();
        for bad in ["", "   ", "work", "Personal", "incognito", &"x".repeat(MAX_NAME_CHARS + 1)] {
            assert!(add(&mut ps, bad, None, 2).is_err(), "{bad:?}");
        }
        assert!(add(&mut ps, "Home", Some("blue"), 2).is_err());
        assert!(add(&mut ps, "Home", Some("#12345"), 2).is_err());
    }

    #[test]
    fn renaming_keeps_its_own_name_free_and_others_taken() {
        let mut ps = Vec::new();
        let a = add(&mut ps, "A", None, 1).unwrap();
        add(&mut ps, "B", None, 1).unwrap();
        update(&mut ps, &a.id, Some("a"), None, None).unwrap();
        assert!(update(&mut ps, &a.id, Some("B"), None, None).is_err());
        update(&mut ps, &a.id, None, Some("#000000"), None).unwrap();
        assert_eq!(ps[0].color, "#000000");
        assert!(update(&mut ps, "p-none", Some("C"), None, None).is_err());
    }

    #[test]
    fn removing_and_the_cap() {
        let mut ps = Vec::new();
        for i in 0..MAX_PROFILES {
            add(&mut ps, &format!("P{i}"), None, 1).unwrap();
        }
        assert!(add(&mut ps, "One more", None, 1).is_err());
        let id = ps[0].id.clone();
        remove(&mut ps, &id).unwrap();
        assert!(remove(&mut ps, &id).is_err());
        assert_eq!(ps.len(), MAX_PROFILES - 1);
    }

    #[test]
    fn the_file_round_trips_and_a_missing_one_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROFILES_FILE_NAME);
        assert!(read_profiles(&path).unwrap().is_empty());
        let mut ps = Vec::new();
        add(&mut ps, "Work", None, 5).unwrap();
        write_profiles(&path, &ps).unwrap();
        assert_eq!(read_profiles(&path).unwrap(), ps);
        std::fs::write(&path, "{not json").unwrap();
        assert!(read_profiles(&path).is_err());
    }
}
