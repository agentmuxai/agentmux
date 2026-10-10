// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a sandboxed widget can reach through srv
//! (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.3, §11).
//!
//! The widget itself never talks to srv: its pane host does, for the calls
//! that need srv (storage, net, agents). Each pane gets a session token when
//! it opens. The token is no credential anywhere else; it names one package
//! at the hash the user approved, and every call checks again that the
//! package is still approved at that hash, enabled, and granted the
//! permission the call needs, whatever the frontend already checked.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::widget_packages::{WidgetKind, WidgetPackageInfo, WidgetState};

/// Errors reach the pane host as `widget-error:` and a JSON object
/// `{ code, message, data? }`, the bridge's own error (§6.6).
pub const ERROR_PREFIX: &str = "widget-error:";

pub const STORAGE_MAX_BYTES: usize = 5 * 1024 * 1024;
pub const STORAGE_VALUE_MAX_BYTES: usize = 1024 * 1024;
pub const STORAGE_KEY_MAX_BYTES: usize = 256;
pub const SEND_MAX_BYTES: usize = 8 * 1024;
pub const SENDS_PER_MINUTE: usize = 10;
const MAX_SESSIONS: usize = 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct AccessError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

impl AccessError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }
    pub fn denied(pkg_name: &str, permission: &str) -> Self {
        Self {
            code: 1001,
            message: format!("{pkg_name} wasn't granted \"{permission}\""),
            data: Some(json!({ "permission": permission })),
        }
    }
    pub fn limit(limit: &str, message: impl Into<String>) -> Self {
        Self { code: 1002, message: message.into(), data: Some(json!({ "limit": limit })) }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(1003, message)
    }
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(1004, message)
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(1005, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(1099, message)
    }
    /// The form an RPC error takes on the wire.
    pub fn to_rpc(&self) -> String {
        let mut v = json!({ "code": self.code, "message": self.message });
        if let Some(d) = &self.data {
            v["data"] = d.clone();
        }
        format!("{ERROR_PREFIX}{v}")
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub widget_id: String,
    pub hash: String,
    pub block_id: String,
    opened: Instant,
}

#[derive(Default)]
pub struct Sessions {
    open: Mutex<HashMap<String, Session>>,
    sends: Mutex<HashMap<String, VecDeque<Instant>>>,
}

pub fn sessions() -> &'static Sessions {
    static SESSIONS: OnceLock<Sessions> = OnceLock::new();
    SESSIONS.get_or_init(Sessions::default)
}

/// The package as it is now, if it may run at all: approved, at this hash,
/// sandboxed (a trusted widget runs in the app and needs no session).
fn runnable<'a>(packages: &'a [WidgetPackageInfo], id: &str, hash: &str) -> Option<&'a WidgetPackageInfo> {
    packages
        .iter()
        .find(|p| p.id == id && p.hash == hash && p.state == WidgetState::Approved && p.kind == WidgetKind::Sandboxed)
}

impl Sessions {
    /// A token for one pane of an approved package at `hash`.
    pub fn open(&self, packages: &[WidgetPackageInfo], id: &str, hash: &str, block_id: &str) -> Result<String, AccessError> {
        if runnable(packages, id, hash).is_none() {
            return Err(AccessError::unavailable(format!("{id} isn't approved at this version")));
        }
        let token = hex::encode(uuid::Uuid::new_v4().as_bytes()) + &hex::encode(uuid::Uuid::new_v4().as_bytes());
        let mut open = self.open.lock().unwrap_or_else(|p| p.into_inner());
        if open.len() >= MAX_SESSIONS {
            // Panes that closed without saying so: drop the oldest.
            if let Some(oldest) = open.iter().min_by_key(|(_, s)| s.opened).map(|(t, _)| t.clone()) {
                open.remove(&oldest);
            }
        }
        open.insert(
            token.clone(),
            Session { widget_id: id.to_string(), hash: hash.to_string(), block_id: block_id.to_string(), opened: Instant::now() },
        );
        Ok(token)
    }

    pub fn close(&self, token: &str) {
        self.open.lock().unwrap_or_else(|p| p.into_inner()).remove(token);
    }

    /// The session's package, if it may still run: approved at the hash the
    /// session was opened for. An edited, re-approved, disabled or removed
    /// package ends every session of the old version.
    pub fn authorize(&self, packages: &[WidgetPackageInfo], token: &str) -> Result<(Session, WidgetPackageInfo), AccessError> {
        let session = self
            .open
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(token)
            .cloned()
            .ok_or_else(|| AccessError::unavailable("this widget's session ended; reload it"))?;
        let pkg = runnable(packages, &session.widget_id, &session.hash)
            .cloned()
            .ok_or_else(|| AccessError::unavailable(format!("{} changed or was turned off; reload it", session.widget_id)))?;
        Ok((session, pkg))
    }

    /// Counts one `agents.send` for the package, or refuses it over the rate.
    pub fn take_send(&self, widget_id: &str) -> Result<(), AccessError> {
        self.take_send_at(widget_id, Instant::now())
    }

    fn take_send_at(&self, widget_id: &str, now: Instant) -> Result<(), AccessError> {
        let mut sends = self.sends.lock().unwrap_or_else(|p| p.into_inner());
        let recent = sends.entry(widget_id.to_string()).or_default();
        while recent.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60)) {
            recent.pop_front();
        }
        if recent.len() >= SENDS_PER_MINUTE {
            return Err(AccessError::limit("rate", format!("a widget can send at most {SENDS_PER_MINUTE} messages a minute")));
        }
        recent.push_back(now);
        Ok(())
    }
}

/// Refuses unless `pkg` was granted `permission`.
pub fn require(pkg: &WidgetPackageInfo, permission: &str) -> Result<(), AccessError> {
    if pkg.granted.iter().any(|g| g == permission) {
        Ok(())
    } else {
        Err(AccessError::denied(&pkg.name, permission))
    }
}

/// A storage key: non-empty, at most 256 bytes.
pub fn check_key(key: &str) -> Result<(), AccessError> {
    if key.is_empty() || key.len() > STORAGE_KEY_MAX_BYTES {
        return Err(AccessError::invalid(format!("a storage key is 1 to {STORAGE_KEY_MAX_BYTES} bytes")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::widget_packages::WidgetPaneInfo;

    fn pkg(state: WidgetState, kind: WidgetKind, granted: &[&str]) -> WidgetPackageInfo {
        WidgetPackageInfo {
            id: "acme.notes".into(),
            name: "Notes".into(),
            version: "1.0.0".into(),
            description: None,
            author: None,
            homepage: None,
            icon: "note-sticky".into(),
            default_hue: None,
            kind,
            permissions: granted.iter().map(|s| s.to_string()).collect(),
            granted: granted.iter().map(|s| s.to_string()).collect(),
            state,
            error: None,
            hash: "h1".into(),
            panes: Vec::<WidgetPaneInfo>::new(),
            files_url: None,
            implied: false,
            folder: String::new(),
        }
    }

    #[test]
    fn a_session_is_for_one_approved_sandboxed_version_only() {
        let s = Sessions::default();
        let approved = vec![pkg(WidgetState::Approved, WidgetKind::Sandboxed, &["storage"])];
        assert!(s.open(&approved, "acme.notes", "h0", "b1").is_err(), "another hash");
        assert!(s.open(&[pkg(WidgetState::NeedsApproval, WidgetKind::Sandboxed, &[])], "acme.notes", "h1", "b1").is_err());
        assert!(s.open(&[pkg(WidgetState::Approved, WidgetKind::Trusted, &[])], "acme.notes", "h1", "b1").is_err());
        let token = s.open(&approved, "acme.notes", "h1", "b1").unwrap();
        let (session, p) = s.authorize(&approved, &token).unwrap();
        assert_eq!((session.widget_id.as_str(), session.block_id.as_str()), ("acme.notes", "b1"));
        assert!(require(&p, "storage").is_ok());
        assert_eq!(require(&p, "agents:send").unwrap_err().data, Some(json!({ "permission": "agents:send" })));
        // Changed since: the session no longer works.
        let changed = vec![pkg(WidgetState::Changed, WidgetKind::Sandboxed, &["storage"])];
        assert_eq!(s.authorize(&changed, &token).unwrap_err().code, 1005);
        s.close(&token);
        assert!(s.authorize(&approved, &token).is_err());
        assert!(s.authorize(&approved, "made-up").is_err());
    }

    #[test]
    fn sends_are_limited_per_package_per_minute() {
        let s = Sessions::default();
        let t0 = Instant::now();
        for _ in 0..SENDS_PER_MINUTE {
            s.take_send_at("acme.a", t0).unwrap();
        }
        assert_eq!(s.take_send_at("acme.a", t0).unwrap_err().code, 1002);
        assert!(s.take_send_at("acme.b", t0).is_ok(), "another package has its own budget");
        assert!(s.take_send_at("acme.a", t0 + Duration::from_secs(61)).is_ok());
    }

    #[test]
    fn errors_carry_their_code_and_data_on_the_wire() {
        let e = AccessError::denied("Notes", "storage").to_rpc();
        let v: Value = serde_json::from_str(e.strip_prefix(ERROR_PREFIX).unwrap()).unwrap();
        assert_eq!(v, json!({ "code": 1001, "message": "Notes wasn't granted \"storage\"", "data": { "permission": "storage" } }));
    }
}
