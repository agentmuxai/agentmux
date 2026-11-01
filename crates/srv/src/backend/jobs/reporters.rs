// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The cloud reporters the operator pinned (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.2).
//!
//! A reporter is a name and a public key. Nothing is built in: no key for anyone's private jobs is compiled into the
//! public app. The file is the same shape as the widget and template publisher pins (name to base64 key), so it is
//! read and written by the same functions, and a key is shown by its fingerprint.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::backend::widget_signature::{self as sig, Pins};

/// Beside the template pins, in the job templates' data folder.
pub const REPORTERS_FILE: &str = "job-reporters.json";

/// A reporter, for Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ReporterPin {
    pub name: String,
    pub fingerprint: String,
}

/// Where a manifest's reporter stands against the pins: what approving it would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub enum ReporterState {
    /// Not pinned yet: approving pins it.
    New,
    /// Already pinned, under this name and key.
    Pinned,
    /// The name is pinned to another key: approving is refused until the operator forgets that key.
    NameTaken,
    /// The key is pinned under another name: approving is refused.
    KeyTaken,
}

pub struct Reporters {
    path: PathBuf,
    pins: Mutex<Pins>,
}

/// `acme-ci`: what a reporter may be called.
pub(crate) fn is_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Whether `key` is a 32-byte Ed25519 public key in base64.
pub(crate) fn is_public_key(key: &str) -> bool {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(key.trim()).is_ok_and(|b| b.len() == 32)
}

impl Reporters {
    /// The pins in `path`, none when there is no file.
    pub fn open(path: &Path) -> Self {
        Self { path: path.to_path_buf(), pins: Mutex::new(sig::read_pins(path)) }
    }

    fn pins(&self) -> std::sync::MutexGuard<'_, Pins> {
        self.pins.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn list(&self) -> Vec<ReporterPin> {
        self.pins().iter().map(|(name, key)| ReporterPin { name: name.clone(), fingerprint: sig::fingerprint(key) }).collect()
    }

    /// What pinning `name` to `public_key` would do, as `add` would decide it.
    pub fn state_of(&self, name: &str, public_key: &str) -> ReporterState {
        let key = public_key.trim();
        let pins = self.pins();
        match pins.get(name) {
            Some(existing) if existing == key => ReporterState::Pinned,
            Some(_) => ReporterState::NameTaken,
            None if pins.values().any(|k| k == key) => ReporterState::KeyTaken,
            None => ReporterState::New,
        }
    }

    /// Pin `name` to `public_key` (base64, a 32-byte Ed25519 key). Pinning a name again to another key is refused:
    /// the operator forgets the old one first, so a key never changes without being seen.
    pub fn add(&self, name: &str, public_key: &str) -> Result<ReporterPin, String> {
        if !is_name(name) {
            return Err("a reporter's name is lowercase letters, digits and -, up to 40".to_string());
        }
        let key = public_key.trim();
        if !is_public_key(key) {
            return Err("that isn't a 32-byte Ed25519 public key in base64".to_string());
        }
        let mut pins = self.pins();
        if let Some(existing) = pins.get(name).filter(|existing| existing.as_str() != key) {
            return Err(format!("{name:?} is pinned to another key ({}); forget it first", sig::fingerprint(existing)));
        }
        if let Some((other, _)) = pins.iter().find(|(n, k)| k.as_str() == key && n.as_str() != name) {
            return Err(format!("that key is already pinned as {other:?}"));
        }
        pins.insert(name.to_string(), key.to_string());
        sig::write_pins(&self.path, &pins)?;
        Ok(ReporterPin { name: name.to_string(), fingerprint: sig::fingerprint(key) })
    }

    pub fn forget(&self, name: &str) -> Result<(), String> {
        let mut pins = self.pins();
        if pins.remove(name).is_none() {
            return Err(format!("no reporter named {name:?} is pinned"));
        }
        sig::write_pins(&self.path, &pins)
    }

    /// The fingerprint of `public_key` if it is pinned, else `None`.
    pub fn fingerprint_of_pinned(&self, public_key: &str) -> Option<String> {
        let key = public_key.trim();
        self.pins().values().any(|k| k == key).then(|| sig::fingerprint(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::widget_signature::test_keys;

    #[test]
    fn a_reporter_is_pinned_listed_and_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(REPORTERS_FILE);
        let reporters = Reporters::open(&path);
        let key = test_keys::public_key(1);
        let pin = reporters.add("acme-ci", &key).unwrap();
        assert_eq!(pin.fingerprint, sig::fingerprint(&key));
        assert_eq!(reporters.list(), vec![pin.clone()]);
        // It survives a restart.
        assert_eq!(Reporters::open(&path).fingerprint_of_pinned(&key), Some(pin.fingerprint));
        reporters.forget("acme-ci").unwrap();
        assert!(Reporters::open(&path).list().is_empty());
        assert!(reporters.forget("acme-ci").is_err());
    }

    #[test]
    fn a_name_keeps_its_key_and_a_key_has_one_name() {
        let dir = tempfile::tempdir().unwrap();
        let reporters = Reporters::open(&dir.path().join(REPORTERS_FILE));
        let (a, b) = (test_keys::public_key(1), test_keys::public_key(2));
        reporters.add("acme-ci", &a).unwrap();
        assert!(reporters.add("acme-ci", &a).is_ok(), "pinning the same pair again changes nothing");
        assert!(reporters.add("acme-ci", &b).unwrap_err().contains("forget it first"));
        assert!(reporters.add("other", &a).unwrap_err().contains("already pinned"));
        assert_eq!(reporters.list().len(), 1);
    }

    #[test]
    fn only_a_name_and_a_32_byte_key_are_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let reporters = Reporters::open(&dir.path().join(REPORTERS_FILE));
        let key = test_keys::public_key(1);
        for bad in ["", "Acme", "a b", "../x", &"x".repeat(41)] {
            assert!(reporters.add(bad, &key).is_err(), "{bad:?}");
        }
        for bad in ["", "not base64!", "AAAA"] {
            assert!(reporters.add("acme-ci", bad).is_err(), "{bad:?}");
        }
        assert!(reporters.list().is_empty());
    }
}
