//! Phase B.2 IPC wire protocol — shared between agentmux-launcher
//! (server) and agentmux-cef (client). One source of truth so the
//! Command / Event shapes can't drift between binaries on a
//! version-skew release.
//!
//! See `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §5.
//!
//! Wire format: newline-delimited JSON. One message per line,
//! parsed via serde_json. Format chosen for debuggability —
//! operators can `cat` / `nc` the named pipe and read traffic
//! without a binary protocol decoder.
//!
//! Backward compat policy (B.2 baseline; harden in Phase D):
//!   * Externally tagged enums (`{"cmd":"register",...}`) so adding
//!     variants is non-breaking; clients send what they know.
//!   * Unknown commands → server replies `Event::Error` rather than
//!     crashing.
//!   * `version: u64` on every Event lets Phase D's GetSnapshot /
//!     resync detect skew. For B.2 it's set but not enforced.

use serde::{Deserialize, Serialize};

/// Stable identifier for a connected client. Tagged so the launcher
/// can route replies + log who said what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    /// The CEF host process (one per launcher run).
    Host,
    /// A frontend renderer (proxied via the host's CEF JS bridge — Phase B.7).
    Renderer,
    /// The agentmux-srv backend (proxy connection used for Quit ack
    /// + process-tree facts; the workspace data path stays on
    /// HTTP/WS through host).
    Srv,
    /// External tooling (`agentmux.exe --diag` etc.).
    Tool,
}

/// Commands flow client → launcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Identifies the connection. MUST be the first command on every
    /// new connection. Server enforces.
    Register {
        kind: ClientKind,
        /// PID of the connecting process — used for cross-checking
        /// against the launcher's `ProcessRecord` map and for
        /// log correlation.
        pid: u32,
        /// Free-form version string of the client binary, for log
        /// correlation across version skew.
        version: String,
    },
    /// Health probe — server replies with `Event::Pong` carrying the
    /// same nonce. NOT a polling heartbeat (per spec §4.3) — sent
    /// only on demand by clients that need round-trip confirmation.
    Ping {
        nonce: u64,
    },
    /// Graceful disconnect. Server logs and closes the connection.
    /// In B.3+ this becomes `Quit { reason }` with shutdown semantics;
    /// for B.2 it's just a polite goodbye.
    Goodbye,
}

/// Events flow launcher → client. Versioned per spec §5.2 — every
/// event carries a monotonic `version: u64` per launcher run, used
/// by Phase D's resync protocol.
///
/// Phase B.3 introduces the first non-handshake events
/// (ProcessSpawned, ProcessExited, LifecyclePhaseChanged) emitted
/// by the launcher's reducer when commands transition state. B.4+
/// adds the window-state events (WindowAdded, WindowStateChanged,
/// WindowRemoved) per spec §5.2.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Reply to `Command::Register`. Acknowledges the client kind +
    /// confirms the launcher's view of the world.
    Registered {
        client_id: u64,
        launcher_pid: u32,
        launcher_version: String,
        version: u64,
    },
    /// Reply to `Command::Ping`. Echoes the nonce.
    Pong {
        nonce: u64,
        version: u64,
    },
    /// Sent when an incoming command can't be parsed or violates an
    /// invariant (e.g. Command before Register). Connection stays
    /// open unless `fatal: true`.
    Error {
        code: ErrorCode,
        message: String,
        fatal: bool,
        version: u64,
    },
    /// A process joined the launcher's canonical registry. Emitted
    /// when a client first Registers (B.3) and, in B.4+, when the
    /// launcher itself spawns a child.
    ProcessSpawned {
        pid: u32,
        kind: ClientKind,
        client_version: String,
        version: u64,
    },
    /// A process exited or disconnected gracefully. Emitted on
    /// Goodbye (B.3) and, in B.4+, on detected child exit.
    ProcessExited {
        pid: u32,
        /// Exit code. 0 = clean Goodbye; non-zero = OS-reported
        /// exit code or synthetic value for crashes.
        code: i32,
        version: u64,
    },
    /// The launcher's lifecycle phase changed. Spec §4 defines the
    /// valid transitions: Starting → Running → Quitting → Dead.
    /// Emitted at most once per transition.
    LifecyclePhaseChanged {
        from: LifecyclePhase,
        to: LifecyclePhase,
        version: u64,
    },
}

/// Coarse-grained launcher state. Spec §4: Starting → Running →
/// Quitting → Dead, no other transitions allowed. The reducer in
/// agentmux-launcher::reducer enforces this; a violation panics
/// (Job Object reaps via OS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LifecyclePhase {
    /// Initial state; launcher has not yet seen the host register.
    Starting,
    /// Host has registered and the canonical state is being
    /// maintained. Steady state.
    Running,
    /// Quit { reason } received, ack outstanding to subscribers.
    /// Phase B.3 keeps this state-shape only — the actual Quit
    /// command lands in a later sub-PR.
    Quitting,
    /// Cleanup done; launcher about to exit. Transient.
    Dead,
}

/// Discriminant for `Event::Error` — keeps clients structured against
/// failure modes without parsing message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Couldn't deserialize the line into a Command.
    InvalidCommand,
    /// Command sent before Register.
    NotRegistered,
    /// Register sent twice on the same connection.
    AlreadyRegistered,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_register_roundtrip() {
        let c = Command::Register {
            kind: ClientKind::Host,
            pid: 12345,
            version: "0.33.449".into(),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"cmd\":\"register\""));
        assert!(json.contains("\"kind\":\"host\""));
        let back: Command = serde_json::from_str(&json).unwrap();
        if let Command::Register { kind, pid, version } = back {
            assert_eq!(kind, ClientKind::Host);
            assert_eq!(pid, 12345);
            assert_eq!(version, "0.33.449");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn event_registered_roundtrip() {
        let e = Event::Registered {
            client_id: 1,
            launcher_pid: 9999,
            launcher_version: "0.33.449".into(),
            version: 42,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"event\":\"registered\""));
        let back: Event = serde_json::from_str(&json).unwrap();
        if let Event::Registered { client_id, version, .. } = back {
            assert_eq!(client_id, 1);
            assert_eq!(version, 42);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn unknown_cmd_fails_to_deserialize() {
        let json = r#"{"cmd":"banana"}"#;
        let r: Result<Command, _> = serde_json::from_str(json);
        assert!(r.is_err());
    }
}
