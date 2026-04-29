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
    /// Phase B.4: host reports that a real window has been created
    /// (CEF `on_after_created` fired). Launcher records it in its
    /// read-only mirror and broadcasts `Event::WindowOpened` to other
    /// subscribers. Pool windows do NOT report via this command —
    /// they get their own `ReportPool*` commands in a follow-up so
    /// the mirror can distinguish user-visible windows from pool
    /// inventory.
    ReportWindowOpened {
        /// Stable label assigned by the host (e.g. "main", "window-{uuid}").
        label: String,
        kind: WindowKind,
        /// For `Subwindow` only: label of the `FullInstance` parent.
        /// `None` for `FullInstance`.
        parent_label: Option<String>,
    },
    /// Phase B.4: host reports a window is closing (`on_before_close`).
    /// Launcher removes from mirror, broadcasts `Event::WindowClosed`.
    /// Idempotent: a missing label is logged but not an error (covers
    /// the close-before-launcher-saw-the-open race).
    ReportWindowClosed {
        label: String,
    },
    /// Phase B.4 follow-up — host reports a pre-warmed pool window
    /// being added (`spawn_pool_window`). Pool windows live in a
    /// SEPARATE map from the user-visible window mirror; the host
    /// transitions them out of the pool with `ReportPoolWindowRemoved`
    /// + `ReportWindowOpened` on promote, or just
    /// `ReportPoolWindowRemoved` on pre-promote destroy.
    ReportPoolWindowAdded {
        label: String,
    },
    /// Phase B.4 follow-up — host reports a pool window leaving the
    /// pool (promote, destroy, or app exit).
    ReportPoolWindowRemoved {
        label: String,
    },
    /// Phase B.4 follow-up — drift detection (full snapshot). Host
    /// sends its own post-mutation counts after each window-level
    /// transition; the launcher reducer compares both dimensions to
    /// its mirror counts and emits `Event::DriftDetected` per
    /// disagreeing dimension. Sent in a separate command (rather
    /// than embedded in each Report*) so the existing wire shapes
    /// stay unchanged.
    ///
    /// Known limitation (B.4 observe-only): emissions originate
    /// from multiple execution contexts (CEF UI thread for
    /// `on_after_created`/`on_before_close`, IPC handler thread
    /// for `promote_pool_window`). Cross-thread interleaving in
    /// the outbound channel can produce a snapshot whose counts
    /// were taken at a moment that doesn't match the channel
    /// order seen by the reducer, occasionally emitting a
    /// transient false `DriftDetected`. Acceptable for B.4
    /// (drift is diagnostic — false positives are ephemeral and
    /// self-correct on the next stable state). B.5 will tighten
    /// with a transition-ID protocol once the launcher is
    /// authoritative. (codex P2 PR #578 round-4 — accepted as
    /// known limitation.)
    ReportHostCounts {
        /// User-visible top-level windows in the host's
        /// authoritative store (`browsers` minus browser-pane
        /// children minus unpromoted pool labels).
        windows: u32,
        /// Pre-promote pool inventory size.
        pool: u32,
    },
    /// Phase B.4 follow-up — pool-dimension-only drift check. Used
    /// by `spawn_pool_window` (pool transitions) where snapshotting
    /// the windows dimension would produce transient false drift:
    /// pool refill is triggered DURING `on_before_close` BEFORE the
    /// matching `ReportWindowClosed` lands, so the host's window
    /// count is mid-flight relative to the launcher mirror. Pool
    /// count IS stable at that moment (the new pool label was just
    /// added), so checking pool alone preserves the "check every
    /// transition" guarantee for the dimension that actually
    /// changed. (codex P2 PR #578 round-3.)
    ReportHostPoolCount {
        count: u32,
    },
    /// Phase B.5 (window_id_map step a) — host reports the
    /// frontend's `register_backend_window` call: a window's label
    /// → backend window ID (a srv-side UUID the frontend resolves
    /// via `WOS.makeORef`). The launcher mirrors it for the same
    /// reasons it mirrors `instance_registry`: host's authoritative
    /// copy will be retired through the a→b→c→d→e ratchet.
    ReportBackendWindowIdRegistered {
        label: String,
        window_id: String,
    },
    /// Phase B.5 (window_id_map step a) — host reports a window
    /// closing, so the launcher should drop the label→window_id
    /// mapping. Sent from the same close path that emits
    /// `ReportWindowClosed`.
    ReportBackendWindowIdUnregistered {
        label: String,
    },
    /// Phase B.9.1 (WRR — Window Reality Reconciliation) — host
    /// reports a Win32 top-level window was created. Hooked off
    /// `EVENT_OBJECT_CREATE` (objid=`OBJID_WINDOW`) via
    /// `SetWinEventHook(WINEVENT_OUTOFCONTEXT)`. The host filters
    /// CEF subprocess HWNDs (renderer, GPU, plugin) at the hook
    /// callback so the launcher only sees plausible app windows.
    /// `label_hint` is the host's best guess from
    /// `pending_window_creations` if it can disambiguate; `None`
    /// when the create event arrives before the host has linked
    /// the HWND back to a label (caught up later by the reducer's
    /// pending_hwnds).
    ReportHwndOpened {
        hwnd: u64,
        class_name: String,
        title: String,
        label_hint: Option<String>,
    },
    /// Phase B.9.1 — host reports a Win32 top-level window was
    /// destroyed. Hooked off `EVENT_OBJECT_DESTROY`.
    ReportHwndDestroyed {
        hwnd: u64,
    },
    /// Phase B.9.1 — `EVENT_OBJECT_SHOW` / `EVENT_OBJECT_HIDE`.
    ReportHwndVisibilityChanged {
        hwnd: u64,
        visible: bool,
    },
    /// Phase B.9.1 — `EVENT_SYSTEM_FOREGROUND`. Tells the launcher
    /// the user actually saw this window (not just opened it).
    /// Used to distinguish "opened but never foregrounded" (drift)
    /// from "opened and shown".
    ReportHwndForegroundChanged {
        hwnd: u64,
    },
    /// Phase B.9.1 — `EVENT_SYSTEM_MINIMIZESTART` /
    /// `EVENT_SYSTEM_MINIMIZEEND`.
    ReportHwndIconicChanged {
        hwnd: u64,
        iconic: bool,
    },
    /// Phase B.9.1 — `WM_WINDOWPOSCHANGED` from the host's wndproc
    /// wrapper. The host coalesces bursts (50ms debounce per HWND)
    /// before sending so the wire stays light during topology
    /// changes. Reducer compares `rect` against `state.monitors` to
    /// classify off-monitor drift.
    ReportHwndPositionChanged {
        hwnd: u64,
        rect: Rect,
    },
    /// Phase B.9.1 — `WM_DISPLAYCHANGE`. Replaces the launcher's
    /// `state.monitors` wholesale; reducer re-evaluates every
    /// known window's last_rect against the new topology and
    /// emits `OffMonitor` drift for any that newly fall off.
    ReportMonitorTopologyChanged {
        rects: Vec<Rect>,
    },
}

/// Phase B.9.1 — rectangle in Win32 screen coordinates (pixels).
/// Matches Windows' `RECT` semantics: `right` and `bottom` are one
/// past the last included pixel, so `right - left == width`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Wire-side enum for `WindowKind`. Mirrors `agentmux-cef::state::WindowKind`
/// — kept here so the launcher can deserialize without depending on the
/// host crate. The host serializes its own type via `serde(rename_all =
/// "snake_case")` so the JSON shape matches exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    /// Independent AgentMux window. Appears in the Windows taskbar.
    FullInstance,
    /// Hidden from the taskbar; closes when its parent FullInstance closes.
    Subwindow,
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
    /// Phase B.4: a window joined the launcher's mirror. Emitted in
    /// response to `Command::ReportWindowOpened` from the host. Other
    /// subscribers (Tool clients, eventually srv) receive this to
    /// keep their own views consistent.
    WindowOpened {
        label: String,
        kind: WindowKind,
        parent_label: Option<String>,
        version: u64,
    },
    /// Phase B.4: a window left the launcher's mirror. Emitted on
    /// `Command::ReportWindowClosed`. Cascades for FullInstance
    /// closures are NOT modeled here yet (B.5 tightens) — for now
    /// the host emits one ReportWindowClosed per window even on
    /// cascade closes, so subscribers see the same N events.
    WindowClosed {
        label: String,
        version: u64,
    },
    /// Phase B.4 follow-up — pool inventory transitioned. Emitted in
    /// response to `ReportPoolWindow{Added,Removed}`. Subscribers
    /// (Tool clients) use this to track pool warmth without polling.
    PoolWindowAdded {
        label: String,
        version: u64,
    },
    PoolWindowRemoved {
        label: String,
        version: u64,
    },
    /// Phase B.5 (window_id_map step a) — launcher recorded the
    /// label → backend window ID mapping. Subscribers (host's
    /// shadow, eventually srv-side consumers) update their
    /// projections.
    BackendWindowIdRegistered {
        label: String,
        window_id: String,
        version: u64,
    },
    /// Phase B.5 (window_id_map step a) — launcher dropped the
    /// label → backend window ID mapping (window closed).
    BackendWindowIdUnregistered {
        label: String,
        window_id: String,
        version: u64,
    },
    /// Phase B.5 — sequential instance number assigned to a window
    /// by the launcher's authoritative registry. Numbers start at 1
    /// for "main" and increment for each subsequent open. Never
    /// reused within a launcher run. The host caches these to
    /// display window titles; B.5 step 2 will retire host's own
    /// `WindowInstanceRegistry` in favor of this stream.
    WindowInstanceAssigned {
        label: String,
        num: u32,
        version: u64,
    },
    /// Phase B.5 — instance number released (window closed). The
    /// launcher's authoritative registry drops the label; the slot
    /// is NOT reused (numbers monotonic per spec invariant: stable
    /// instance# across promotions / unregisters).
    WindowInstanceReleased {
        label: String,
        num: u32,
        version: u64,
    },
    /// Phase B.4 follow-up — emitted when the launcher's mirror
    /// disagrees with the host's reported counts. Logged at WARN
    /// level so operators see drift immediately. Drift in B.4 is
    /// a CONTRACT BUG (the host should report every state change);
    /// B.5 will turn drift into a hard failure once the mirror is
    /// authoritative.
    DriftDetected {
        kind: DriftKind,
        host_count: u32,
        mirror_count: u32,
        version: u64,
    },
    /// Phase B.9.1 (WRR) — emitted when the launcher's reducer
    /// detects a divergence between CEF browser identity (tracked
    /// in `state.windows` / `state.pool` via `ReportWindow*`) and
    /// Win32 reality (newly tracked via `ReportHwnd*`). Each variant
    /// of `kind` is emitted at the moment the OS event that
    /// surfaces it is dispatched through the reducer — there is no
    /// timer / heartbeat. See `docs/retro/wrr-design-2026-04-28.md`
    /// for the full classification table.
    HwndDriftDetected {
        kind: HwndDriftKind,
        /// Affected label, when known.
        label: Option<String>,
        /// Affected HWND, when known. `u64` to keep the wire format
        /// stable across pointer-width differences.
        hwnd: Option<u64>,
        /// Human-readable description for the launcher log + `--diag`
        /// output. Free-form; not parsed by any consumer.
        detail: String,
        severity: Severity,
        version: u64,
    },
    /// Phase B.9.2 — pure-reducer self-heal. Emitted alongside an
    /// `HwndDriftDetected` when the reducer is confident the bug
    /// happened at OPEN TIME (not from later user action) and a
    /// safe corrective rect is computable. The host's WRR
    /// subscriber listens for this and applies `SetWindowPos` on
    /// the UI thread. No timers — the trigger is the same OS
    /// event tick that surfaced the bug, so the correction lands
    /// before the user has time to notice the orphan window.
    ///
    /// Guard for emission (per the design, suppress over-correction):
    /// - The window's `mirror.foregrounded_since_open == false`
    ///   (we never auto-move a window the user has already
    ///   touched).
    /// - The reducer can compute a target rect from
    ///   `state.monitors` (i.e., monitors are known).
    CorrectiveWindowMove {
        /// Win32 HWND to move.
        hwnd: u64,
        /// Reducer-computed target rect. Default policy:
        /// primary-monitor-centered at default window size.
        target_rect: Rect,
        /// Why correction fired — surfaces in host log + audit.
        reason: HwndDriftKind,
        version: u64,
    },
}

/// Phase B.9.1 — six classes of CEF↔Win32 disagreement the reducer
/// can detect at event-dispatch time. See the WRR design doc for
/// the per-kind triggering Command and reducer action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HwndDriftKind {
    /// `state.windows` has a label whose `hwnd` field never got
    /// populated, AND a follow-up event arrived that should have
    /// reconciled it. CEF says open, Win32 has no matching HWND.
    BrowserWithoutHwnd,
    /// HWND in the host's report doesn't map to any
    /// `state.windows` / `state.pool` label. Win32 has it, CEF
    /// didn't open it. The "stray taskbar" case.
    HwndWithoutBrowser,
    /// HWND for a known label was never SHOWN (no foreground
    /// event) since open and a subsequent state transition has
    /// elapsed. User can't see it.
    HiddenSinceOpen,
    /// Window rect doesn't intersect any monitor in
    /// `state.monitors`. Off-screen orphan.
    OffMonitor,
    /// `ReportHwndDestroyed` arrived without a preceding
    /// `ReportWindowClosed` for the matching label. Renderer
    /// crashed, took the HWND with it.
    OrphanDestroy,
    /// `ReportWindowClosed` arrived, but subsequent OS events for
    /// the HWND keep firing (it never went away on the Win32 side).
    LingeringHwnd,
}

/// Phase B.9.1 — drift severity. Operator-tunable severity floor
/// in `WrrConfig.severity_floor` controls which events get
/// broadcast (ones below the floor are still logged at DEBUG so
/// they show up in `--diag wrr` post-mortem).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
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

/// Phase B.4 follow-up — which mirror diverged. Tagged so subscribers
/// can route alerts (windows-drift might page; pool-drift is more
/// ephemeral since the pool turns over fast).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftKind {
    Windows,
    Pool,
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
