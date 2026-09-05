// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.3 launcher state. Held inside Arc<Mutex<...>> by the IPC
// server; mutated only by the pure reducer (`crate::reducer::update`).
// Mutex is held for microseconds at a time — never across an .await
// boundary, never across I/O. Mirrors the Elm/Redux pattern: state
// is data, transitions are functions, side effects fire after the
// state mutation commits.
//
// What's here in B.3:
//   * `LifecyclePhase` (re-exported from agentmux-common::ipc so the
//     wire and internal types are the same enum)
//   * `ProcessRecord` — pid, kind, state, spawned_at
//   * `ProcessState` — Spawning / Running / Exited
//   * `State` — top-level container: lifecycle + process map +
//     monotonic version counter + monotonic client_id counter
//
// What's intentionally NOT here yet:
//   * Window state machine (B.4–B.5)
//   * Warm-pool (B.5)
//   * Event log ring buffer (B.4 — added when events first start
//     accumulating beyond handshake replies)

use std::collections::{HashMap, HashSet};

use agentmux_common::ipc::{ClientKind, Rect, WindowKind};
pub use agentmux_common::ipc::LifecyclePhase;

/// Lifecycle of a single process the launcher knows about. The
/// reducer transitions through these in order — there's no skipping
/// (Spawning → Running → Exited).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Spawn issued; child handle returned but process hasn't
    /// confirmed it's alive yet. In B.3 we transition straight to
    /// Running on Register because that's the first authoritative
    /// signal. B.4+ adds intermediate state for "spawned but not
    /// yet registered." F.7 cleanup audit: the variant is reserved
    /// for that future intermediate state; keep with allow rather
    /// than delete to preserve the documented state machine shape.
    #[allow(dead_code)]
    Spawning,
    /// Process has registered with the launcher and is doing its
    /// work. Healthy.
    Running,
    /// Process exited (clean Goodbye → code=0, crash → non-zero).
    Exited { code: i32 },
}

/// One process in the launcher's canonical view. Updated by the
/// reducer; read by IPC handlers + the eventual `--diag` printer.
///
/// F.7 cleanup audit: `pid` and `spawned_at` are written by
/// `handle_register` but never read at runtime. They're carried for
/// the future `--diag launcher` printer (alongside `version` /
/// `kind`) and for Debug derivations in tests and crash dumps. Keep
/// with allow rather than delete — losing them now means rebuilding
/// the diag printer from scratch.
#[derive(Debug, Clone)]
pub struct ProcessRecord {
    #[allow(dead_code)]
    pub pid: u32,
    pub kind: ClientKind,
    pub state: ProcessState,
    /// RFC3339 timestamp of the spawn (or first-register, whichever
    /// the launcher learned about first).
    #[allow(dead_code)]
    pub spawned_at: String,
    /// Free-form version string of the registered binary. For log
    /// correlation across version skew during a Phase B rollout.
    #[allow(dead_code)]
    pub version: String,
}

/// Phase B.4 read-only mirror of one host-owned window. The launcher
/// learns about windows via `Command::ReportWindowOpened`; the host
/// remains authoritative until B.5 flips the direction. `opened_at`
/// is the launcher's clock at the time the report arrived (RFC3339)
/// — useful for correlating launcher logs with host logs across
/// version skew.
///
/// Phase B.9.1 (WRR) — the observability axis (`hwnd`, `visible`,
/// `iconic`, `last_rect`, `last_foreground_at_ms`,
/// `foregrounded_since_open`) is populated by the host's Win32
/// event hooks. Pre-B.9 these all sit at default values, which is
/// fine: the WRR drift checks only fire when at least one of the
/// `ReportHwnd*` Commands arrives, and they don't arrive in
/// `task dev` mode (no launcher → no IPC) or installed mode until
/// the host-side hooks land. Existing reducer paths
/// (`ReportWindowOpened`, `ReportWindowClosed`, etc.) ignore these
/// fields entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowMirror {
    pub label: String,
    pub kind: WindowKind,
    /// Set only for `Subwindow`; identifies the FullInstance that
    /// owns this window so the eventual cascade-close logic (B.5)
    /// has the parent linkage.
    pub parent_label: Option<String>,
    pub opened_at: String,
    /// Milliseconds-since-launcher-start at which the host's
    /// `ReportWindowOpened` arrived. Used by `apply_hwnd_visibility_changed`
    /// to suppress `HiddenSinceOpen` drift during the post-create
    /// placement grace window — CEF creates windows hidden, places
    /// them, then shows them, and the intermediate `WM_HIDE` events
    /// would otherwise fire spurious drift on every fresh window.
    pub opened_at_ms: u64,
    /// Phase B.9.1 — Win32 HWND linked to this label by the WRR
    /// reducer arm (via `ReportHwndOpened` with matching
    /// `label_hint` or via the post-hoc reconciliation against
    /// `pending_hwnds`). `None` until the host's
    /// `ReportHwndOpened` for this label arrives.
    pub hwnd: Option<u64>,
    /// Phase B.9.1 — last-known `IsWindowVisible` state.
    pub visible: bool,
    /// Phase B.9.1 — last-known minimized state.
    pub iconic: bool,
    /// Phase B.9.1 — last-known window rect. `None` until the
    /// first `ReportHwndPositionChanged` arrives for the linked
    /// HWND.
    pub last_rect: Option<Rect>,
    /// Phase B.9.1 — milliseconds-since-launcher-start of the most
    /// recent `ReportHwndForegroundChanged` for this label's HWND.
    /// `None` if the window has never been foregrounded.
    pub last_foreground_at_ms: Option<u64>,
    /// Phase B.9.1 — has this label been foregrounded at any point
    /// since its `ReportWindowOpened`? Used to fire `HiddenSinceOpen`
    /// drift on the first hide event when this is still false.
    pub foregrounded_since_open: bool,
    /// Drift-storm fix: `HiddenSinceOpen` / `OffMonitor` /
    /// `CorrectiveWindowMove` each fire AT MOST ONCE per window per
    /// session. Without these caps, a fresh top-level window that
    /// goes through several SetWindowPos transitions during host
    /// placement re-emits the same event for every intermediate
    /// snapshot — observed up to 170 events in 1 second, exhausting
    /// the renderer's V8 stack and crashing it. The cap fires once;
    /// subscribers still see the signal, no storm.
    ///
    /// `OffMonitor` shares the same risk as `HiddenSinceOpen` because
    /// `apply_hwnd_position_changed` fires per WM_MOVE — dragging an
    /// already-off-monitor window emits drift on every pixel.
    /// `CorrectiveWindowMove` rides with it (fires per position
    /// change while `!foregrounded_since_open`).
    ///
    /// All three flags are monotonic for a window's lifetime: once
    /// true, never reset (preserve via OR-with-prior on duplicate
    /// `ReportWindowOpened`, codex P2 PR #708 round 3).
    ///
    /// See `docs/specs/ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`
    /// for storm context.
    pub hidden_since_open_emitted: bool,
    /// Set when `apply_hwnd_visibility_changed` sees `visible=false`
    /// during the placement grace window. Marks "we suppressed a
    /// hide; if it persists past the grace, drift fires on the next
    /// reducer call via `drain_deferred_hidden_since_open`". Cleared
    /// when the window subsequently becomes visible or is foregrounded.
    /// Without this, a window that goes hidden during grace and
    /// receives no further visibility events would permanently lose
    /// its `HiddenSinceOpen` drift signal (codex P2 PR #725 round 1).
    pub hidden_since_open_deferred: bool,
    /// See `hidden_since_open_emitted` doc above.
    pub off_monitor_drift_emitted: bool,
    /// See `hidden_since_open_emitted` doc above.
    pub corrective_window_move_emitted: bool,
}

/// Top-level launcher state. Single Arc<Mutex<State>> owned by the
/// IPC server; passed into `update(state, cmd, conn)` for every
/// incoming command.
#[derive(Debug)]
pub struct State {
    pub lifecycle: LifecyclePhase,
    /// Keyed by PID. Multiple records per PID would be a bug — the
    /// reducer enforces unique-pid on insert.
    pub processes: HashMap<u32, ProcessRecord>,
    /// Read-only window mirror (Phase B.4). Keyed by label. Source of
    /// truth still lives in `agentmux-cef::AppState.browsers` /
    /// `window_meta`; this is a passive copy fed by host
    /// `ReportWindow*` commands. B.5 inverts the dependency: host
    /// queries this map instead of maintaining its own.
    pub windows: HashMap<String, WindowMirror>,
    /// Phase B.4 follow-up — pre-warmed pool inventory. Tracked
    /// separately from `windows` because pool entries are not
    /// user-visible until promote. On promote the host emits
    /// `ReportPoolWindowRemoved` + `ReportWindowOpened` so the same
    /// label transitions atomically (from launcher's perspective)
    /// from `pool` to `windows`. On pre-promote destroy: only
    /// `ReportPoolWindowRemoved`.
    pub pool: HashSet<String>,
    /// Phase B.5 — authoritative window instance registry. Maps
    /// label → sequential instance number (1 for "main", 2 for the
    /// second window opened, etc.). Numbers are never reused within
    /// a launcher run — when a window closes the entry is removed
    /// but `next_instance_num` keeps advancing. Sole source of truth
    /// post-B.5e (host's `WindowInstanceRegistry` was deleted in
    /// PR #584); host holds a passive shadow projection in
    /// `agentmux-cef::AppState.shadow_instance_registry`. Updated by
    /// the same reducer paths that mutate `windows`.
    pub instance_registry: HashMap<String, u32>,
    /// Next instance number to assign. Starts at 2 — "main" is
    /// pre-seeded with 1 in `Default` (matching host's
    /// `WindowInstanceRegistry::new` behavior so a synthetic main
    /// open wouldn't collide).
    pub next_instance_num: u32,
    /// Phase B.5 (window_id_map step a) — authoritative
    /// label → backend window ID map. Mirrors host's existing
    /// `agentmux-cef::AppState.window_id_map`. Populated by
    /// `Command::ReportBackendWindowIdRegistered` (sent from host
    /// when the frontend calls `register_backend_window` IPC
    /// after init); drained by `ReportBackendWindowIdUnregistered`
    /// on close. Will become host-side authoritative through the
    /// standard a→b→c→d→e ratchet.
    pub backend_window_ids: HashMap<String, String>,
    /// Monotonic counter for `Event.version`. Bumped by `bump_version()`.
    pub event_version: u64,
    /// Monotonic counter for client_id (returned in Registered events).
    pub next_client_id: u64,
    /// Phase B.9.1 (WRR) — current monitor topology, replaced
    /// wholesale on `ReportMonitorTopologyChanged`. Empty by default
    /// until the host's `wrr/wndproc.rs` reports the first
    /// `WM_DISPLAYCHANGE`-equivalent (or its initial topology probe
    /// at startup). `OffMonitor` drift is suppressed when this is
    /// empty — we don't know enough to classify yet.
    pub monitors: Vec<Rect>,
    /// Phase B.9.1 — HWNDs the reducer has seen via `ReportHwndOpened`
    /// but couldn't yet associate with a `WindowMirror`. Three
    /// reasons an entry lives here transiently:
    ///   1. The OS create event raced ahead of the host's
    ///      `OnAfterCreated` → `ReportWindowOpened` chain.
    ///   2. The host couldn't determine `label_hint` at create time.
    ///   3. The HWND belongs to a pool window not yet promoted.
    /// Drained on each `ReportWindowOpened` (we try to match a
    /// pending HWND by `label_hint`/timing). Anything still here
    /// after a follow-up event is classified as `HwndWithoutBrowser`.
    pub pending_hwnds: HashMap<u64, PendingHwnd>,
    /// Drift-storm fix (PR #708 round 3) — labels for which the host
    /// emitted `ReportPoolWindowPromoted` but the corresponding
    /// `ReportWindowOpened` hasn't arrived yet. The actual host emit
    /// order on tear-off is `ReportPoolWindowRemoved` →
    /// `ReportPoolWindowPromoted` → `ReportWindowOpened`, so at
    /// promote-time the launcher has NO `WindowMirror` for the label
    /// — the mirror is created by `ReportWindowOpened` a few ms
    /// later. Without this set, the post-promote mirror is initialized
    /// with `foregrounded_since_open: false`, the open-transient drift
    /// detector then fires `HiddenSinceOpen` on every visible→hidden
    /// flicker during HWND repositioning, the host fans each event
    /// out across the bridge and the renderer's V8 isolate crashes.
    /// `ReportWindowOpened` consumes the entry to initialize the new
    /// mirror with `foregrounded_since_open: true`. Removed on
    /// `ReportWindowClosed` if open never arrived (bounded leak).
    /// See `docs/specs/ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`.
    pub just_promoted_labels: HashSet<String>,
    /// Workstream 0 Phase 1 (`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md`
    /// §7) — mirrors the host's `AGENTMUX_BACKGROUND_SERVICE` opt-in, set
    /// once via `Command::ReportBackgroundServiceEnabled` right after
    /// connect. Defaults false (today's behavior unchanged) until the host
    /// reports otherwise. Consulted by `handle_report_window_closed` and
    /// `wrr`'s crash-detected twin so an intentionally-resting host (zero
    /// windows, by design) doesn't get classified as `OrphanInstance` and
    /// arm the `teardown_backstop` — see PR #2983 review (Codex P2): without
    /// this, a host correctly staying alive on purpose would still get its
    /// whole process tree killed by the backstop after a transient UI-thread
    /// probe hiccup, since "armed" would otherwise last for the entire
    /// (now potentially long) resting period instead of the few seconds it
    /// used to.
    pub background_service_enabled: bool,
    // F.7 cleanup audit: removed unused `launcher_start_ms: Option<u64>`
    // field. It was set in Default but no reducer arm or consumer
    // ever read it — the WRR observability path uses the OnceLock-
    // backed `launcher_start_ms()` helper in `ipc::server` directly,
    // not a state field. Genuine leftover from the early B.9.1 sketch.
}

/// Phase B.9.1 — transient HWND record held until the reducer can
/// link it to a `WindowMirror`. See `State::pending_hwnds`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingHwnd {
    pub class_name: String,
    pub title: String,
    pub label_hint: Option<String>,
    /// Milliseconds-since-launcher-start when the
    /// `ReportHwndOpened` arrived. Used to age out pending entries
    /// — if we still have it when a *different* event arrives that
    /// should have caused reconciliation, classify as
    /// `HwndWithoutBrowser`.
    pub arrived_at_ms: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            lifecycle: LifecyclePhase::Starting,
            processes: HashMap::new(),
            windows: HashMap::new(),
            pool: HashSet::new(),
            instance_registry: {
                let mut m = HashMap::new();
                m.insert("main".to_string(), 1);
                m
            },
            next_instance_num: 2,
            backend_window_ids: HashMap::new(),
            event_version: 0,
            next_client_id: 1,
            monitors: Vec::new(),
            pending_hwnds: HashMap::new(),
            just_promoted_labels: HashSet::new(),
            background_service_enabled: false,
        }
    }
}

impl State {
    /// Bump and return the new event version. Always called inside
    /// the reducer when constructing an Event so version numbers
    /// stay strictly monotonic.
    ///
    /// Strict (non-wrapping) add: Phase D's GetSnapshot resync
    /// protocol relies on monotonicity (`event.version >
    /// snapshot.version`), and a wrap to 0 would silently break
    /// that contract. u64 at one event/ns would take 584 years to
    /// overflow — never going to happen in practice; if it ever
    /// does, the panic is the right failure mode.
    /// (gemini MEDIUM PR #574 round-1.)
    pub fn bump_version(&mut self) -> u64 {
        self.event_version += 1;
        self.event_version
    }

    /// Bump and return the next client_id. Client IDs are stable
    /// per launcher run; not persisted across restart. Same strict-
    /// add reasoning as bump_version.
    pub fn alloc_client_id(&mut self) -> u64 {
        let id = self.next_client_id;
        self.next_client_id += 1;
        id
    }
}
