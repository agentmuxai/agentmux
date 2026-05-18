// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Cross-process error catalog.
//!
//! `AgentMuxError` is the single typed error returned by RPC handlers
//! and surfaced to the frontend. Each variant carries a stable
//! `AmxCode` (e.g. `AMX-IO-001`) so the renderer's translation table
//! can look up a user-friendly message + recovery hint without
//! caring about the wire-format details.
//!
//! See `docs/specs/SPEC_ERROR_CATALOG_2026_05_17.md` for the full
//! design and migration plan.

use serde::Serialize;
use thiserror::Error;

/// Stable string codes shipped to the frontend. The variant name in
/// `AgentMuxError` is for Rust callers; the `&'static str` returned
/// by `AmxCode::as_str()` is the contract with the catalog at
/// `frontend/app/errors/catalog.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmxCode {
    // Filesystem / I/O
    OutOfSpace,
    PermissionDenied,
    PathNotFound,
    PathTraversal,
    // Persistence
    MigrationFailed,
    VersionMismatch,
    // Provider CLI
    CliNotInstalled,
    NpmInstallFailed,
    CliShimMissing,
    // Auth
    AuthRequiresTty,
    AuthTimeout,
    // Network
    HttpError,
    // Lifecycle
    SidecarBindFailed,
    AlreadyRunning,
    // Fallback for un-migrated handlers — every legacy `Err(String)`
    // gets wrapped in this so the frontend still has a code to grep.
    Legacy,
}

impl AmxCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            AmxCode::OutOfSpace => "AMX-IO-001",
            AmxCode::PermissionDenied => "AMX-IO-002",
            AmxCode::PathNotFound => "AMX-IO-003",
            AmxCode::PathTraversal => "AMX-IO-004",
            AmxCode::MigrationFailed => "AMX-STORE-001",
            AmxCode::VersionMismatch => "AMX-STORE-002",
            AmxCode::CliNotInstalled => "AMX-CLI-001",
            AmxCode::NpmInstallFailed => "AMX-CLI-002",
            AmxCode::CliShimMissing => "AMX-CLI-003",
            AmxCode::AuthRequiresTty => "AMX-AUTH-001",
            AmxCode::AuthTimeout => "AMX-AUTH-002",
            AmxCode::HttpError => "AMX-NET-001",
            AmxCode::SidecarBindFailed => "AMX-LIFECYCLE-001",
            AmxCode::AlreadyRunning => "AMX-LIFECYCLE-002",
            AmxCode::Legacy => "AMX-LEGACY",
        }
    }
}

impl std::fmt::Display for AmxCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl serde::Serialize for AmxCode {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}

/// The typed error returned by RPC handlers. Serializes to:
/// `{ "code": "AMX-IO-001", "message": "device out of space ...", "details": { ... } }`
#[derive(Debug, Error)]
pub enum AgentMuxError {
    // ── Filesystem / I/O ────────────────────────────────────
    #[error("device out of space writing {path}")]
    OutOfSpace { path: String, source_msg: String },

    #[error("permission denied accessing {path}")]
    PermissionDenied { path: String, source_msg: String },

    #[error("path not found: {path}")]
    PathNotFound { path: String },

    #[error("path traversal blocked: {path}")]
    PathTraversal { path: String },

    // ── Persistence ─────────────────────────────────────────
    #[error("schema migration {from}→{to} failed: {message}")]
    MigrationFailed { from: u32, to: u32, message: String },

    #[error("optimistic-lock version mismatch on {oid} (expected {expected}, actual {actual})")]
    VersionMismatch { oid: String, expected: u64, actual: u64 },

    // ── Provider CLI ────────────────────────────────────────
    #[error("CLI {cli} not installed for provider {provider}")]
    CliNotInstalled { provider: String, cli: String },

    #[error("npm install failed for {package}: {message}")]
    NpmInstallFailed { package: String, message: String },

    #[error("installed CLI shim missing: {expected_path}")]
    CliShimMissing { provider: String, expected_path: String },

    // ── Auth ────────────────────────────────────────────────
    #[error("OAuth subprocess requires an interactive TTY: {provider}")]
    AuthRequiresTty { provider: String },

    #[error("OAuth login timed out after {seconds}s: {provider}")]
    AuthTimeout { provider: String, seconds: u64 },

    // ── Network ─────────────────────────────────────────────
    #[error("HTTP request failed ({status:?}) for {url}: {message}")]
    HttpError {
        url: String,
        status: Option<u16>,
        message: String,
    },

    // ── Lifecycle ───────────────────────────────────────────
    #[error("sidecar bind failed on port {port}: {message}")]
    SidecarBindFailed { port: u16, message: String },

    #[error("single-instance lock held by pid {pid}")]
    AlreadyRunning { pid: u32 },

    // ── Fallback ────────────────────────────────────────────
    #[error("{0}")]
    Legacy(String),
}

impl AgentMuxError {
    /// Stable code for this variant. Mirrors the JSON `code` field
    /// the frontend's catalog looks up.
    pub fn code(&self) -> AmxCode {
        match self {
            AgentMuxError::OutOfSpace { .. } => AmxCode::OutOfSpace,
            AgentMuxError::PermissionDenied { .. } => AmxCode::PermissionDenied,
            AgentMuxError::PathNotFound { .. } => AmxCode::PathNotFound,
            AgentMuxError::PathTraversal { .. } => AmxCode::PathTraversal,
            AgentMuxError::MigrationFailed { .. } => AmxCode::MigrationFailed,
            AgentMuxError::VersionMismatch { .. } => AmxCode::VersionMismatch,
            AgentMuxError::CliNotInstalled { .. } => AmxCode::CliNotInstalled,
            AgentMuxError::NpmInstallFailed { .. } => AmxCode::NpmInstallFailed,
            AgentMuxError::CliShimMissing { .. } => AmxCode::CliShimMissing,
            AgentMuxError::AuthRequiresTty { .. } => AmxCode::AuthRequiresTty,
            AgentMuxError::AuthTimeout { .. } => AmxCode::AuthTimeout,
            AgentMuxError::HttpError { .. } => AmxCode::HttpError,
            AgentMuxError::SidecarBindFailed { .. } => AmxCode::SidecarBindFailed,
            AgentMuxError::AlreadyRunning { .. } => AmxCode::AlreadyRunning,
            AgentMuxError::Legacy(_) => AmxCode::Legacy,
        }
    }

    /// Helper for the common case: an `std::io::Error` raised while
    /// operating on a known path. Use this at call sites that have
    /// the path on hand — the `From<std::io::Error>` impl below
    /// can't recover the path from the bare IO error.
    pub fn from_io_with_path(path: impl Into<String>, err: std::io::Error) -> Self {
        let path = path.into();
        let source_msg = err.to_string();
        match Self::classify_io(&err) {
            AmxCode::OutOfSpace => AgentMuxError::OutOfSpace { path, source_msg },
            AmxCode::PermissionDenied => AgentMuxError::PermissionDenied { path, source_msg },
            AmxCode::PathNotFound => AgentMuxError::PathNotFound { path },
            _ => AgentMuxError::Legacy(format!("{path}: {source_msg}")),
        }
    }

    fn classify_io(err: &std::io::Error) -> AmxCode {
        // `ErrorKind::StorageFull` was stabilized in 1.83 but we
        // can't rely on it across all toolchains the CI uses. Match
        // raw OS codes instead. ENOSPC=28 is portable across Unix +
        // also unused on Windows. The Windows-specific codes 39 and
        // 112 collide with Unix errnos (ENOTEMPTY / EHOSTDOWN) so
        // they must be gated to `cfg(windows)` — otherwise a
        // disconnected CIFS mount on Linux would mis-classify as
        // "Device out of space."
        if err.raw_os_error() == Some(28) {
            return AmxCode::OutOfSpace;
        }
        #[cfg(windows)]
        if matches!(err.raw_os_error(), Some(39) | Some(112)) {
            // 39  = ERROR_HANDLE_DISK_FULL (file-handle-bound APIs)
            // 112 = ERROR_DISK_FULL        (volume-level APIs)
            return AmxCode::OutOfSpace;
        }
        match err.kind() {
            std::io::ErrorKind::PermissionDenied => AmxCode::PermissionDenied,
            std::io::ErrorKind::NotFound => AmxCode::PathNotFound,
            _ => AmxCode::Legacy,
        }
    }

    /// Serializes the error into the wire format the RPC engine
    /// emits. Frontend pattern-matches on `code`.
    pub fn to_wire(&self) -> serde_json::Value {
        let mut details = serde_json::Map::new();
        match self {
            AgentMuxError::OutOfSpace { path, source_msg } => {
                details.insert("path".into(), path.clone().into());
                details.insert("source_msg".into(), source_msg.clone().into());
            }
            AgentMuxError::PermissionDenied { path, source_msg } => {
                details.insert("path".into(), path.clone().into());
                details.insert("source_msg".into(), source_msg.clone().into());
            }
            AgentMuxError::PathNotFound { path } | AgentMuxError::PathTraversal { path } => {
                details.insert("path".into(), path.clone().into());
            }
            AgentMuxError::MigrationFailed { from, to, message } => {
                details.insert("from".into(), (*from).into());
                details.insert("to".into(), (*to).into());
                details.insert("message".into(), message.clone().into());
            }
            AgentMuxError::VersionMismatch { oid, expected, actual } => {
                details.insert("oid".into(), oid.clone().into());
                details.insert("expected".into(), (*expected).into());
                details.insert("actual".into(), (*actual).into());
            }
            AgentMuxError::CliNotInstalled { provider, cli } => {
                details.insert("provider".into(), provider.clone().into());
                details.insert("cli".into(), cli.clone().into());
            }
            AgentMuxError::NpmInstallFailed { package, message } => {
                details.insert("package".into(), package.clone().into());
                details.insert("message".into(), message.clone().into());
            }
            AgentMuxError::CliShimMissing { provider, expected_path } => {
                details.insert("provider".into(), provider.clone().into());
                details.insert("expected_path".into(), expected_path.clone().into());
            }
            AgentMuxError::AuthRequiresTty { provider } => {
                details.insert("provider".into(), provider.clone().into());
            }
            AgentMuxError::AuthTimeout { provider, seconds } => {
                details.insert("provider".into(), provider.clone().into());
                details.insert("seconds".into(), (*seconds).into());
            }
            AgentMuxError::HttpError { url, status, message } => {
                details.insert("url".into(), url.clone().into());
                if let Some(s) = status {
                    details.insert("status".into(), (*s).into());
                }
                details.insert("message".into(), message.clone().into());
            }
            AgentMuxError::SidecarBindFailed { port, message } => {
                details.insert("port".into(), (*port).into());
                details.insert("message".into(), message.clone().into());
            }
            AgentMuxError::AlreadyRunning { pid } => {
                details.insert("pid".into(), (*pid).into());
            }
            AgentMuxError::Legacy(_) => {}
        }
        serde_json::json!({
            "code": self.code().as_str(),
            "message": self.to_string(),
            "details": serde_json::Value::Object(details),
        })
    }
}

impl Serialize for AgentMuxError {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        self.to_wire().serialize(ser)
    }
}

impl From<std::io::Error> for AgentMuxError {
    /// Implicit conversion routes by `ErrorKind` + raw OS code, but
    /// loses the path context. Prefer `from_io_with_path` at sites
    /// that know which file/dir was being operated on — the empty
    /// path here renders as the literal `(unknown path)` sentinel
    /// in `Display`, which leaks into wire `message` and the
    /// Details disclosure.
    fn from(err: std::io::Error) -> Self {
        let source_msg = err.to_string();
        let unknown = || UNKNOWN_PATH.to_string();
        match Self::classify_io(&err) {
            AmxCode::OutOfSpace => AgentMuxError::OutOfSpace {
                path: unknown(),
                source_msg,
            },
            AmxCode::PermissionDenied => AgentMuxError::PermissionDenied {
                path: unknown(),
                source_msg,
            },
            AmxCode::PathNotFound => AgentMuxError::PathNotFound { path: unknown() },
            _ => AgentMuxError::Legacy(source_msg),
        }
    }
}

/// Sentinel rendered when an `std::io::Error` is converted without
/// path context (via the `From` impl above). `from_io_with_path`
/// supplies the real path so the user sees the offending location.
const UNKNOWN_PATH: &str = "(unknown path)";

/// Wrap a free-text string in `AgentMuxError::Legacy`. Used by the
/// RPC engine to bridge un-migrated handlers that still return
/// `Result<_, String>`.
impl From<String> for AgentMuxError {
    fn from(s: String) -> Self {
        AgentMuxError::Legacy(s)
    }
}

impl From<&str> for AgentMuxError {
    fn from(s: &str) -> Self {
        AgentMuxError::Legacy(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_strs_unique_and_stable() {
        let all = [
            AmxCode::OutOfSpace,
            AmxCode::PermissionDenied,
            AmxCode::PathNotFound,
            AmxCode::PathTraversal,
            AmxCode::MigrationFailed,
            AmxCode::VersionMismatch,
            AmxCode::CliNotInstalled,
            AmxCode::NpmInstallFailed,
            AmxCode::CliShimMissing,
            AmxCode::AuthRequiresTty,
            AmxCode::AuthTimeout,
            AmxCode::HttpError,
            AmxCode::SidecarBindFailed,
            AmxCode::AlreadyRunning,
            AmxCode::Legacy,
        ];
        let mut seen: Vec<&str> = Vec::new();
        for c in all {
            let s = c.as_str();
            assert!(s.starts_with("AMX-"), "{s} missing AMX- prefix");
            assert!(!seen.contains(&s), "duplicate code {s}");
            seen.push(s);
        }
    }

    #[test]
    fn io_error_routes_enospc_to_out_of_space() {
        // Synthesize an error with ENOSPC OS code.
        let err = std::io::Error::from_raw_os_error(28);
        let mux: AgentMuxError = err.into();
        assert_eq!(mux.code(), AmxCode::OutOfSpace);
    }

    #[cfg(windows)]
    #[test]
    fn io_error_routes_windows_disk_full_to_out_of_space() {
        let err = std::io::Error::from_raw_os_error(112);
        let mux: AgentMuxError = err.into();
        assert_eq!(mux.code(), AmxCode::OutOfSpace);
    }

    #[cfg(windows)]
    #[test]
    fn io_error_routes_windows_handle_disk_full_to_out_of_space() {
        // ERROR_HANDLE_DISK_FULL — what `WriteFile` returns when the
        // disk fills up via a file-handle-bound write.
        let err = std::io::Error::from_raw_os_error(39);
        let mux: AgentMuxError = err.into();
        assert_eq!(mux.code(), AmxCode::OutOfSpace);
    }

    #[cfg(not(windows))]
    #[test]
    fn io_error_unix_ehostdown_does_not_route_to_out_of_space() {
        // On Linux raw OS error 112 = EHOSTDOWN, not disk-full.
        // Must NOT be classified as OutOfSpace — the Windows code
        // 112 (ERROR_DISK_FULL) must be cfg-gated.
        let err = std::io::Error::from_raw_os_error(112);
        let mux: AgentMuxError = err.into();
        assert_ne!(mux.code(), AmxCode::OutOfSpace);
    }

    #[test]
    fn io_error_permission_denied_routes() {
        let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let mux: AgentMuxError = err.into();
        assert_eq!(mux.code(), AmxCode::PermissionDenied);
    }

    #[test]
    fn io_error_not_found_routes() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let mux: AgentMuxError = err.into();
        assert_eq!(mux.code(), AmxCode::PathNotFound);
    }

    #[test]
    fn io_error_unclassified_routes_to_legacy() {
        let err = std::io::Error::new(std::io::ErrorKind::InvalidData, "bad bytes");
        let mux: AgentMuxError = err.into();
        assert_eq!(mux.code(), AmxCode::Legacy);
    }

    #[test]
    fn from_io_with_path_preserves_path() {
        let err = std::io::Error::from_raw_os_error(28);
        let mux = AgentMuxError::from_io_with_path("/tmp/foo.db", err);
        assert_eq!(mux.code(), AmxCode::OutOfSpace);
        match mux {
            AgentMuxError::OutOfSpace { path, .. } => assert_eq!(path, "/tmp/foo.db"),
            _ => panic!("expected OutOfSpace"),
        }
    }

    #[test]
    fn wire_format_has_code_message_details() {
        let mux = AgentMuxError::OutOfSpace {
            path: "/tmp/x".into(),
            source_msg: "ENOSPC".into(),
        };
        let wire = mux.to_wire();
        assert_eq!(wire["code"], "AMX-IO-001");
        assert!(wire["message"]
            .as_str()
            .unwrap()
            .contains("/tmp/x"));
        assert_eq!(wire["details"]["path"], "/tmp/x");
        assert_eq!(wire["details"]["source_msg"], "ENOSPC");
    }

    #[test]
    fn wire_format_round_trip_via_serde() {
        let mux = AgentMuxError::CliNotInstalled {
            provider: "claude".into(),
            cli: "claude".into(),
        };
        let json = serde_json::to_value(&mux).unwrap();
        assert_eq!(json["code"], "AMX-CLI-001");
        assert_eq!(json["details"]["provider"], "claude");
    }

    #[test]
    fn legacy_string_wraps_unchanged() {
        let mux: AgentMuxError = "legacy raw message".into();
        let wire = mux.to_wire();
        assert_eq!(wire["code"], "AMX-LEGACY");
        assert_eq!(wire["message"], "legacy raw message");
    }
}
