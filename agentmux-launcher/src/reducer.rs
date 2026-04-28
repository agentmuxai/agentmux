// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.3 — pure reducer.
//
// Per `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §5.1:
//
//   pub fn update(state: &State, cmd: Command) -> (State, Vec<Event>);
//
// We deviate slightly: the function takes `&mut State` (mutating in
// place rather than returning a new value) because cloning a HashMap
// per-command in the IPC hot path is wasteful and not needed for
// the testability properties we want — `proptest` works equally well
// against a mutating-reducer (apply commands one by one, assert
// invariants hold after each).
//
// Strict properties of `update`:
//   1. **Total**. Every (cmd, state) combination produces a result;
//      never panics on input. (Panics ARE used to enforce internal
//      invariants — those are bugs in the reducer or upstream code,
//      not user input. See spec §7 / §8.)
//   2. **Deterministic**. Same (state, cmd, conn) → same (state',
//      events). No clocks, no UUIDs, no env reads inside update —
//      injected via the `Reducer` context if needed (B.4 will need
//      it for spawned_at timestamps; for B.3 the conn carries one).
//   3. **No I/O**. update never blocks or awaits. Mutex is held for
//      the duration of update — must stay sub-millisecond.
//
// Connection context: every command arrives over a specific
// connection and the resulting events that are *replies* (Registered,
// Pong) belong to that connection only, while *broadcasts*
// (ProcessSpawned, LifecyclePhaseChanged) belong to all subscribers.
// Phase B.3 ships in a simplified model where every event goes over
// the originating connection; B.4 splits the routing.
//
// Invariants checked:
//   * Register on a PID that's already registered → AlreadyRegistered
//     error. (Connection-level enforcement that "Register sent twice"
//     also lives in the server; this is the cross-connection guard.)
//   * Lifecycle transitions: Starting → Running on first Host
//     register. Running → Quitting on Quit (B.3 placeholder).
//     Quitting → Dead on cleanup-done (B.3 placeholder). No skipping.

use agentmux_common::ipc::{ClientKind, Command, ErrorCode, Event, WindowKind};

use crate::state::{LifecyclePhase, ProcessRecord, ProcessState, State, WindowMirror};

/// Context the reducer needs but can't read from State (clocks,
/// connection identity). Passed in per-call so update remains pure.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// RFC3339 timestamp to stamp on new ProcessRecords. Injected
    /// (rather than read from chrono::Utc::now() inside update) to
    /// keep the function deterministic for tests.
    pub now_rfc3339: String,
    /// The connection on which this command arrived. Currently
    /// just used for log correlation; B.4+ will use it to route
    /// per-connection replies.
    pub conn_id: u64,
    /// PID the connection has Registered under, if any. The server
    /// tracks this server-side and passes it on every command after
    /// the initial Register so the reducer can mark the right
    /// process Exited on Goodbye. None for the very first command
    /// on a connection (which MUST be Register, enforced server-
    /// side). (codex P1 + gemini HIGH PR #574 round-1.)
    pub registered_pid: Option<u32>,
}

/// Apply one Command to State, returning the resulting Events. State
/// is mutated in place. Total function — never panics on input
/// (panics are reserved for internal invariant violations).
pub fn update(state: &mut State, cmd: Command, ctx: &Ctx) -> Vec<Event> {
    let _ = ctx.conn_id; // reserved for B.4 routing
    match cmd {
        Command::Register {
            kind,
            pid,
            version,
        } => handle_register(state, ctx, kind, pid, version),
        Command::Ping { nonce } => {
            let v = state.bump_version();
            vec![Event::Pong { nonce, version: v }]
        }
        Command::Goodbye => handle_goodbye(state, ctx.registered_pid.unwrap_or(0)),
        Command::ReportWindowOpened {
            label,
            kind,
            parent_label,
        } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportWindowOpened") {
                return vec![err];
            }
            handle_report_window_opened(state, ctx, label, kind, parent_label)
        }
        Command::ReportWindowClosed { label } => {
            if let Some(err) = enforce_host_only(state, ctx, "ReportWindowClosed") {
                return vec![err];
            }
            handle_report_window_closed(state, label)
        }
    }
}

/// Phase B.4 — gate window-mirror reports to Host clients only. The
/// host is the only source of truth about its own window lifecycle;
/// allowing Renderer/Srv/Tool clients to mutate the mirror would let
/// any registered process spoof open/close traffic and break the
/// host-authoritative model. (codex P1 PR #576 round-1.)
///
/// Returns `Some(Error)` if the calling connection is not a Host;
/// `None` if the call is allowed to proceed. Looks up the kind by
/// PID rather than threading it through `Ctx` because `processes`
/// is already the canonical source — single source of truth, no
/// extra plumbing.
fn enforce_host_only(state: &mut State, ctx: &Ctx, op: &'static str) -> Option<Event> {
    let kind = ctx
        .registered_pid
        .and_then(|pid| state.processes.get(&pid).map(|r| r.kind));
    if kind == Some(ClientKind::Host) {
        return None;
    }
    let v = state.bump_version();
    Some(Event::Error {
        code: ErrorCode::NotRegistered,
        message: format!(
            "{} is Host-only; caller kind={:?}",
            op, kind
        ),
        fatal: false,
        version: v,
    })
}

/// Phase B.4 — record a host-reported window opening in the launcher's
/// read-only mirror. Idempotent on duplicate opens (same label twice
/// in a row): the second insert overwrites with fresh metadata and
/// emits a fresh event. Subscribers must tolerate seeing the same
/// label twice; cleaner once B.5 makes the launcher authoritative.
fn handle_report_window_opened(
    state: &mut State,
    ctx: &Ctx,
    label: String,
    kind: WindowKind,
    parent_label: Option<String>,
) -> Vec<Event> {
    state.windows.insert(
        label.clone(),
        WindowMirror {
            label: label.clone(),
            kind,
            parent_label: parent_label.clone(),
            opened_at: ctx.now_rfc3339.clone(),
        },
    );
    let v = state.bump_version();
    vec![Event::WindowOpened {
        label,
        kind,
        parent_label,
        version: v,
    }]
}

/// Phase B.4 — drop a host-reported window from the mirror. Missing
/// label is logged-only (no Error event) because the close-before-
/// launcher-saw-the-open race is benign in B.4: the launcher's mirror
/// is a passive copy, and the host's authoritative view will simply
/// not have the window either. B.5 will tighten this contract.
fn handle_report_window_closed(state: &mut State, label: String) -> Vec<Event> {
    let was_present = state.windows.remove(&label).is_some();
    let v = state.bump_version();
    if !was_present {
        // Quiet: B.4 is read-only and the mirror can race with the
        // host's authoritative store on first launch. We still emit
        // the Closed event so subscribers stay consistent.
    }
    vec![Event::WindowClosed { label, version: v }]
}

fn handle_register(
    state: &mut State,
    ctx: &Ctx,
    kind: ClientKind,
    pid: u32,
    version: String,
) -> Vec<Event> {
    let mut out = Vec::with_capacity(3);

    // Cross-connection invariant: only ONE live ProcessRecord per
    // PID. We DO allow re-registration if the existing record is
    // Exited — the OS recycles PIDs over a long-running launcher,
    // so a new process can legitimately end up with a PID that was
    // previously held by a process that has cleanly Goodbye'd.
    // Without this carve-out, the process map would accumulate dead
    // records and the launcher would reject increasingly many real
    // registrations. (gemini MEDIUM PR #574 round-1.)
    let existing_state = state.processes.get(&pid).map(|r| r.state);
    if let Some(existing_state) = existing_state {
        if !matches!(existing_state, ProcessState::Exited { .. }) {
            let v = state.bump_version();
            out.push(Event::Error {
                code: ErrorCode::AlreadyRegistered,
                message: format!(
                    "pid {} already in process registry (state={:?})",
                    pid, existing_state
                ),
                fatal: true,
                version: v,
            });
            return out;
        }
        // Else: fall through. The insert below replaces the Exited
        // record with the new live one — same entry, fresh state.
    }

    let record = ProcessRecord {
        pid,
        kind,
        state: ProcessState::Running,
        spawned_at: ctx.now_rfc3339.clone(),
        version: version.clone(),
    };
    state.processes.insert(pid, record);

    let spawned_v = state.bump_version();
    out.push(Event::ProcessSpawned {
        pid,
        kind,
        client_version: version,
        version: spawned_v,
    });

    // Lifecycle: Starting → Running when the first Host registers.
    // Subsequent Host re-registers (after a host crash + restart in
    // some future world) won't double-fire because we'd already be
    // in Running. Other client kinds (Renderer, Srv, Tool) don't
    // drive the transition.
    if state.lifecycle == LifecyclePhase::Starting && kind == ClientKind::Host {
        let from = state.lifecycle;
        state.lifecycle = LifecyclePhase::Running;
        let v = state.bump_version();
        out.push(Event::LifecyclePhaseChanged {
            from,
            to: LifecyclePhase::Running,
            version: v,
        });
    }

    let registered_v = state.bump_version();
    let client_id = state.alloc_client_id();
    out.push(Event::Registered {
        client_id,
        // launcher_pid + launcher_version are filled in by the
        // server before broadcast — they don't belong in the pure
        // reducer (env reads). We use a sentinel here; the server
        // patches these before sending.
        launcher_pid: 0,
        launcher_version: String::new(),
        version: registered_v,
    });

    out
}

fn handle_goodbye(state: &mut State, pid: u32) -> Vec<Event> {
    if pid == 0 {
        // No pid known for this connection (B.3 limitation). Just
        // emit a synthetic ProcessExited with pid=0 to signal the
        // graceful close; the server will log + close.
        let v = state.bump_version();
        return vec![Event::ProcessExited {
            pid: 0,
            code: 0,
            version: v,
        }];
    }
    if let Some(record) = state.processes.get_mut(&pid) {
        record.state = ProcessState::Exited { code: 0 };
    }
    let v = state.bump_version();
    vec![Event::ProcessExited {
        pid,
        code: 0,
        version: v,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(conn_id: u64) -> Ctx {
        Ctx {
            now_rfc3339: "2026-04-28T00:00:00Z".to_string(),
            conn_id,
            registered_pid: None,
        }
    }

    fn ctx_with_pid(conn_id: u64, pid: u32) -> Ctx {
        Ctx {
            now_rfc3339: "2026-04-28T00:00:00Z".to_string(),
            conn_id,
            registered_pid: Some(pid),
        }
    }

    #[test]
    fn first_host_register_transitions_starting_to_running() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1234,
                version: "0.33.450".into(),
            },
            &ctx(1),
        );
        assert_eq!(state.lifecycle, LifecyclePhase::Running);
        assert!(state.processes.contains_key(&1234));
        // Should emit ProcessSpawned + LifecyclePhaseChanged + Registered.
        assert_eq!(events.len(), 3);
        assert!(matches!(
            events[0],
            Event::ProcessSpawned { pid: 1234, .. }
        ));
        assert!(matches!(
            events[1],
            Event::LifecyclePhaseChanged {
                from: LifecyclePhase::Starting,
                to: LifecyclePhase::Running,
                ..
            }
        ));
        assert!(matches!(events[2], Event::Registered { .. }));
    }

    #[test]
    fn second_host_register_does_not_re_emit_lifecycle_change() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1234,
                version: "0.33.450".into(),
            },
            &ctx(1),
        );
        // Different PID, second Host (e.g. test harness or doc)
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 5678,
                version: "0.33.450".into(),
            },
            &ctx(2),
        );
        // Lifecycle stays Running; no LifecyclePhaseChanged event.
        assert_eq!(state.lifecycle, LifecyclePhase::Running);
        assert_eq!(events.len(), 2); // ProcessSpawned + Registered
        assert!(events
            .iter()
            .all(|e| !matches!(e, Event::LifecyclePhaseChanged { .. })));
    }

    #[test]
    fn duplicate_pid_register_returns_already_registered() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1234,
                version: "0.33.450".into(),
            },
            &ctx(1),
        );
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Renderer,
                pid: 1234, // SAME pid
                version: "0.33.450".into(),
            },
            &ctx(2),
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            Event::Error {
                code: ErrorCode::AlreadyRegistered,
                fatal: true,
                ..
            }
        ));
        // Second register doesn't overwrite the first.
        assert_eq!(state.processes[&1234].kind, ClientKind::Host);
    }

    #[test]
    fn renderer_register_does_not_drive_lifecycle() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Renderer,
                pid: 4321,
                version: "0.33.450".into(),
            },
            &ctx(1),
        );
        // Lifecycle stays Starting until a HOST registers.
        assert_eq!(state.lifecycle, LifecyclePhase::Starting);
        assert_eq!(events.len(), 2); // ProcessSpawned + Registered
    }

    #[test]
    fn ping_returns_pong_with_same_nonce() {
        let mut state = State::default();
        let events = update(&mut state, Command::Ping { nonce: 42 }, &ctx(1));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::Pong { nonce: 42, .. }));
    }

    /// Helper used by both the unit test below and the proptest: extract
    /// the version number from any Event variant. New variants must be
    /// added here OR the helper switched to a generic accessor when
    /// the variant set grows large.
    fn extract_version(e: &Event) -> u64 {
        match e {
            Event::ProcessSpawned { version, .. }
            | Event::ProcessExited { version, .. }
            | Event::LifecyclePhaseChanged { version, .. }
            | Event::Registered { version, .. }
            | Event::Pong { version, .. }
            | Event::WindowOpened { version, .. }
            | Event::WindowClosed { version, .. }
            | Event::Error { version, .. } => *version,
        }
    }

    #[test]
    fn event_versions_are_strictly_monotonic() {
        let mut state = State::default();
        let mut versions = vec![];
        for pid in [100, 200, 300] {
            let events = update(
                &mut state,
                Command::Register {
                    kind: ClientKind::Host,
                    pid,
                    version: "0.33.450".into(),
                },
                &ctx(1),
            );
            versions.extend(events.iter().map(extract_version));
        }
        for w in versions.windows(2) {
            assert!(w[1] > w[0], "versions not monotonic: {:?}", versions);
        }
    }

    #[test]
    fn goodbye_marks_registered_pid_as_exited() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1234,
                version: "0.33.451".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            state.processes[&1234].state,
            ProcessState::Running
        ));
        let events = update(&mut state, Command::Goodbye, &ctx_with_pid(1, 1234));
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            Event::ProcessExited { pid: 1234, code: 0, .. }
        ));
        assert!(matches!(
            state.processes[&1234].state,
            ProcessState::Exited { code: 0 }
        ));
    }

    /// B.4 reports require a Host registration first (codex P1
    /// PR #576). Helper to set that up so each window-mirror test
    /// doesn't repeat the boilerplate.
    fn register_host_and_get_ctx(state: &mut State, pid: u32) -> Ctx {
        let _ = update(
            state,
            Command::Register {
                kind: ClientKind::Host,
                pid,
                version: "test".into(),
            },
            &ctx(1),
        );
        ctx_with_pid(1, pid)
    }

    #[test]
    fn report_window_opened_inserts_into_mirror_and_emits_event() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let events = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "main".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::WindowOpened { label, kind: WindowKind::FullInstance, parent_label: None, .. }
                if label == "main"
        ));
        let mirror = &state.windows["main"];
        assert_eq!(mirror.label, "main");
        assert_eq!(mirror.kind, WindowKind::FullInstance);
        assert_eq!(mirror.parent_label, None);
    }

    #[test]
    fn report_window_closed_removes_from_mirror() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "main".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        assert!(state.windows.contains_key("main"));
        let events = update(
            &mut state,
            Command::ReportWindowClosed {
                label: "main".into(),
            },
            &host_ctx,
        );
        assert!(!state.windows.contains_key("main"));
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::WindowClosed { label, .. } if label == "main"
        ));
    }

    #[test]
    fn report_window_closed_on_unknown_label_is_idempotent() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let events = update(
            &mut state,
            Command::ReportWindowClosed {
                label: "ghost".into(),
            },
            &host_ctx,
        );
        // Still emits an event (so subscribers stay consistent) but
        // doesn't error.
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::WindowClosed { .. }));
        assert!(state.windows.is_empty());
    }

    #[test]
    fn subwindow_open_records_parent_label() {
        let mut state = State::default();
        let host_ctx = register_host_and_get_ctx(&mut state, 1234);
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "main".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &host_ctx,
        );
        let _ = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "sub-1".into(),
                kind: WindowKind::Subwindow,
                parent_label: Some("main".into()),
            },
            &host_ctx,
        );
        assert_eq!(
            state.windows["sub-1"].parent_label.as_deref(),
            Some("main")
        );
    }

    #[test]
    fn report_window_opened_from_non_host_is_rejected() {
        let mut state = State::default();
        // Register as Renderer (not Host) at PID 4321.
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Renderer,
                pid: 4321,
                version: "test".into(),
            },
            &ctx(1),
        );
        let events = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "spoof".into(),
                kind: WindowKind::FullInstance,
                parent_label: None,
            },
            &ctx_with_pid(1, 4321),
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::NotRegistered, fatal: false, .. }
        ));
        // Mirror NOT mutated.
        assert!(state.windows.is_empty());
    }

    #[test]
    fn report_window_closed_from_unregistered_conn_is_rejected() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::ReportWindowClosed {
                label: "x".into(),
            },
            &ctx(1), // No Register first.
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::NotRegistered, fatal: false, .. }
        ));
    }

    #[test]
    fn register_replaces_exited_record_for_recycled_pid() {
        let mut state = State::default();
        // First process registers + cleanly exits
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 1234,
                version: "first".into(),
            },
            &ctx(1),
        );
        let _ = update(&mut state, Command::Goodbye, &ctx_with_pid(1, 1234));
        assert!(matches!(
            state.processes[&1234].state,
            ProcessState::Exited { .. }
        ));

        // OS recycles PID 1234 to a new process which Registers
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Renderer,
                pid: 1234,
                version: "second".into(),
            },
            &ctx(2),
        );
        // Should NOT emit AlreadyRegistered — record replaced.
        assert!(!events
            .iter()
            .any(|e| matches!(e, Event::Error {
                code: ErrorCode::AlreadyRegistered,
                ..
            })));
        assert!(matches!(
            state.processes[&1234].state,
            ProcessState::Running
        ));
        assert_eq!(state.processes[&1234].kind, ClientKind::Renderer);
        assert_eq!(state.processes[&1234].version, "second");
    }

    // ===== Property-based tests =====
    //
    // The unit tests above cover specific scenarios; these prove
    // the reducer's INVARIANTS hold across arbitrary input
    // sequences. Per spec §7 + the Phase B plan's testing strategy.

    use proptest::prelude::*;

    /// Generate an arbitrary Command. Constrained to the value-space
    /// the IPC server can actually produce: PIDs are realistic-ish
    /// u32s, version strings are short ASCII.
    fn arb_command() -> impl Strategy<Value = Command> {
        prop_oneof![
            // Register dominates the distribution because that's
            // where most state-machine logic lives.
            5 => (
                prop_oneof![
                    Just(ClientKind::Host),
                    Just(ClientKind::Renderer),
                    Just(ClientKind::Srv),
                    Just(ClientKind::Tool),
                ],
                1u32..10000u32,
                "[a-zA-Z0-9.]{1,16}",
            )
                .prop_map(|(kind, pid, version)| Command::Register { kind, pid, version }),
            1 => any::<u64>().prop_map(|nonce| Command::Ping { nonce }),
            1 => Just(Command::Goodbye),
            // B.4: window mirror commands. Labels drawn from a small
            // alphabet so duplicates (open then close) are common
            // enough for the proptest to exercise the idempotent
            // close path.
            2 => (
                "[a-c]{1,3}",
                prop_oneof![
                    Just(WindowKind::FullInstance),
                    Just(WindowKind::Subwindow),
                ],
                prop_oneof![Just(None::<String>), Just(Some("a".into()))],
            )
                .prop_map(|(label, kind, parent_label)| {
                    Command::ReportWindowOpened { label, kind, parent_label }
                }),
            2 => "[a-c]{1,3}".prop_map(|label| Command::ReportWindowClosed { label }),
        ]
    }

    proptest! {
        /// Versions across an arbitrary sequence of commands are
        /// always strictly monotonic. This is the foundation of
        /// Phase D's GetSnapshot resync: clients detect missed
        /// events by gap in version numbers.
        #[test]
        fn versions_strictly_monotonic_under_any_command_sequence(
            cmds in proptest::collection::vec(arb_command(), 1..50)
        ) {
            let mut state = State::default();
            let mut all_versions = vec![];
            for cmd in cmds {
                let events = update(&mut state, cmd, &ctx(1));
                all_versions.extend(events.iter().map(extract_version));
            }
            for w in all_versions.windows(2) {
                prop_assert!(
                    w[1] > w[0],
                    "version regression: {} → {}",
                    w[0], w[1]
                );
            }
        }

        /// Lifecycle invariant (spec §4): only ever Starting →
        /// Running → Quitting → Dead. No other transition.
        /// In B.3 we exercise just the Starting → Running edge
        /// since later transitions don't have triggering commands
        /// yet, but the harness is ready for B.4+.
        #[test]
        fn lifecycle_only_progresses_forward(
            cmds in proptest::collection::vec(arb_command(), 1..50)
        ) {
            let mut state = State::default();
            let mut prev = state.lifecycle;
            for cmd in cmds {
                let _ = update(&mut state, cmd, &ctx(1));
                let next = state.lifecycle;
                let valid = match (prev, next) {
                    (a, b) if a == b => true,
                    (LifecyclePhase::Starting, LifecyclePhase::Running) => true,
                    (LifecyclePhase::Running, LifecyclePhase::Quitting) => true,
                    (LifecyclePhase::Quitting, LifecyclePhase::Dead) => true,
                    _ => false,
                };
                prop_assert!(
                    valid,
                    "illegal lifecycle transition {:?} → {:?}",
                    prev, next
                );
                prev = next;
            }
        }

        /// Process map invariant: a successful Register inserts the
        /// PID; a duplicate Register (same PID) NEVER overwrites the
        /// existing record. This is what the server relies on for
        /// stale-state safety.
        #[test]
        fn duplicate_register_never_overwrites(
            initial_kind in prop_oneof![Just(ClientKind::Host), Just(ClientKind::Renderer)],
            second_kind in prop_oneof![Just(ClientKind::Srv), Just(ClientKind::Tool)],
            pid in 1u32..10000u32,
        ) {
            let mut state = State::default();
            let _ = update(
                &mut state,
                Command::Register {
                    kind: initial_kind,
                    pid,
                    version: "v1".into(),
                },
                &ctx(1),
            );
            let _ = update(
                &mut state,
                Command::Register {
                    kind: second_kind,
                    pid,
                    version: "v2".into(),
                },
                &ctx(2),
            );
            // Original record preserved across the rejected dup.
            prop_assert_eq!(state.processes[&pid].kind, initial_kind);
            prop_assert_eq!(&state.processes[&pid].version, "v1");
        }
    }
}
