# Pillar 1 Step 4 — Crash Reproject: Automatic Multi-Window Reconstruction

**Date:** 2026-07-07
**Type:** Design spec (grounds Step 4 before any code — Steps 1-3 are done; this is genuinely new
machinery, not "fire an existing path on a new trigger")
**Status:** Ready for review — not yet implemented
**Builds on:** SPEC_864 (merged), SPEC_PILLAR1_STEP2 (merged), SPEC_PILLAR1_STEP3 (merged) —
persistence prerequisites are all in place.
**Corrects/resolves:** `SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` §6 step 4 — the design
doc's own 2026-07-07 addendum already flagged this needs "genuinely new multi-window recreation
code" and "zero existing scaffolding" for in-flight re-derivation. This spec grounds both claims
with a concrete design, and finds one of them is less true than believed (see §2.1).

---

## 0. TL;DR

The host today rebuilds exactly **one** window on any (re)start — cold or post-crash, identically —
by reading `Client.windowids[0]` from srv. Everything else the user had open is gone unless they
manually reopen it. This spec makes reproject **unconditional and idempotent**: every host start
enumerates the full persisted window set and recreates whatever's missing, whether that's zero
extra windows (the common case — nothing to do) or several (the crash-recovery case). No new
"is this a restart" signal is needed — see §2.2 for why.

**The most important finding grounding this spec:** the launcher (which stays alive across a
host-only crash — the far more common failure than a full process-tree kill) already holds a
richer, faster, zero-query copy of the window set in memory (`label`, `kind`, `parent_label`, and
even `last_rect`) than what a fresh host would get by querying srv. There is **already a complete,
tested wire protocol for the host to fetch it** — `Command::GetSnapshot` → `Event::Snapshot`
(`agentmux-launcher/src/reducer/connection.rs:71-113`) — that the host has simply never called.
This spec's design is a **two-tier reproject**: prefer the launcher's live snapshot when available
(fast, richer, no srv round-trip); fall back to srv's durable `Client.windowids` +
`Window.kind`/`parent_window_id` (Step 3) when the launcher itself also died. Both tiers converge on
the same per-window recreation code path.

---

## 1. Current state (verified against source, 2026-07-07 research pass)

### 1.A — No distinguishing "this is a crash restart" signal exists, anywhere
Grepped `agentmux-launcher/src/host_spawn.rs`, `supervisor/windows.rs`, `supervisor/unix.rs` for any
restart counter, env var, or CLI flag — none exists. The crash-relaunch call sites
(`supervisor/windows.rs:558-624`, `unix.rs:516-556`) pass the **identical** `args`/`env` as the
original spawn; the only thing that ever differs is `host_degraded` (a `--disable-gpu` rendering
fallback after repeated crashes, unrelated to topology). `agentmux-cef/src/app.rs::on_context_
initialized` (the sole native-window-creation call at startup) has no branch on prior-session state.
**Conclusion: cold launch and post-crash launch are byte-identical code paths today.** (Confirms and
extends the SPEC_PILLAR1_STEP3 finding.)

### 1.B — The launcher's in-memory window state survives a host-only crash, completely unexploited
`agentmux-launcher/src/state.rs:172-259` (`struct State`) — `windows: HashMap<String, WindowMirror>`
(with `kind`, `parent_label` at `state.rs:96-100`), `instance_registry`, `backend_window_ids` — is
constructed **once**, for the launcher's entire process lifetime, inside `Arc<Mutex<State>>`. A
host-only crash does not touch this `Arc` — only the host *process* dies. Tracing what happens on an
ungraceful host disconnect: `dispatch_synthetic_goodbye` → `handle_goodbye`
(`agentmux-launcher/src/reducer/connection.rs:200-221`) mutates only `state.processes[pid].state =
Exited` — `state.windows` is never touched. The only thing that ever removes a `WindowMirror` entry
is an explicit `ReportWindowClosed`, which a crash never sends.

**The wire protocol to fetch this already exists, fully built, and is simply never called:**
- `Command::GetSnapshot` (`agentmux-common/src/ipc.rs:240`) → `handle_get_snapshot`
  (`agentmux-launcher/src/reducer/connection.rs:71-113`) → `Event::Snapshot { version, lifecycle,
  windows: Vec<WindowSnapshot>, pool, instance_registry, backend_window_ids, monitors }`, where each
  `WindowSnapshot` carries `label, kind, parent_label, hwnd, visible, iconic, last_rect,
  foregrounded_since_open`.
- Grepped all of `agentmux-cef/src` for `GetSnapshot` — **zero call sites.** The host's launcher-
  connect flow (`agentmux-cef/src/launcher_ipc.rs::connect_to_launcher`) sends `Register` then just
  starts a passive delta-apply read loop; it never requests a snapshot.
- `handle_register`'s own comment (`agentmux-launcher/src/reducer/connection.rs:169`) already
  anticipated this: *"Subsequent Host re-registers (after a host crash + restart in some future
  world) won't double-fire [the lifecycle transition]"* — written in the future tense, never acted
  on.

**This is a green-field design opportunity, not a bug fix.** The launcher's snapshot is strictly
richer than what Step 3 persisted to srv (it includes `last_rect`, which srv's `Window.pos`/
`winsize` fields are — per the parent design doc — currently dead/unwritten) and requires no query
latency (already in the launcher's memory). It is *not* a replacement for srv persistence, though —
a full process-tree kill (launcher + host together, e.g. an OS-level OOM-killer sweep) loses it
entirely, which is exactly the case Step 3's srv persistence exists to cover.

### 1.C — In-flight/transient `HostState` fields (design doc's "2.C") already start correctly empty
Every field in this category (`pending_window_creations`, `pending_browser_pane_creates`,
`browser_panes`'s Live/Closing lifecycle, `pool.respawn_in_flight`/`pane_pool.respawn_in_flight`,
`quit_state`, `top_level_creation`) is a plain Rust struct field with no cross-process persistence.
`AppState::default()` (`agentmux-cef/src/state.rs:923`) constructs a byte-fresh `HostState::default()`
on every process start — cold or post-crash, identically. **Verified: there is nothing to "clear" or
"re-derive" in the sense of resetting stale data — every field is already empty/reset by construction
on any fresh process.** The design doc's §7 "subtle part" warning is still correct in spirit but was
mis-aimed at state-clearing; the actual discipline needed is **procedural**: the reproject driver
must create windows/panes through the same code paths an interactive user action would use (which
correctly populate these transient queues as a side effect of real creation), never by synthesizing
entries directly into `pending_window_creations`/`browser_panes` to *simulate* a resumed operation.
No existing code does the latter — there is no anti-pattern to fix, only a constraint to write down
before someone invents one.

### 1.D — No overlay/splash mechanism distinguishes cold-start from crash-restart, and none has a text slot
Three existing loading-treatment mechanisms, none reusable as-is:
- **Launcher native splash** (`agentmux-launcher/src/splash.rs`) — spawned exactly once per launcher
  lifetime (`supervisor/windows.rs:199`, before the *first* host spawn only), destroyed permanently
  after first dismiss (`splash.rs:315-338`: fade → `DestroyWindow` → thread returns). Never
  re-invoked on the crash-restart branches (`windows.rs:558-624`). Its content is a stage-telemetry
  list (`"Saga recovery"`, `"Host startup"`, …), not a swappable headline — there's no generic text
  field to repurpose.
- **`BrainSpinner`** (`frontend/app/element/BrainSpinner.tsx`) — confirmed frontend/DOM-only; every
  call site is a `Suspense`/`Show` fallback *inside an already-mounted pane inside an already-loaded
  window*. Not usable for the pre-window phase of a reproject, which is exactly the phase that
  matters most (the user sees nothing at all until a window exists).
- **`index.html`'s `#startup-loading`** (`frontend/app/init/startup-splash.ts`) — a CSS-only
  full-cover overlay baked into the HTML template, shown before any JS runs, dismissed by
  `fadeOutStartupSplash()`. Reappears automatically on every window's page load (including a
  reprojected one) with zero host-side wiring — but has no text slot either, just an animated logo.

### 1.E — `open_window_with_kind` is the right internal function to build on, with two gaps
`agentmux-cef/src/commands/window/creation.rs:334-416` — the private function both `open_new_window`
and `open_subwindow` (the IPC entry points) delegate to. It already does the right thing for
`(kind, parent_instance_id)`: builds the label, the frontend URL (`windowLabel=`/`initialView=`/
`initialMeta=`), dispatches `EnqueuePendingWindowCreation`, and calls
`ui_tasks::post_create_window`. Two gaps for reproject's purposes:
1. **Not `pub(crate)`** — trivial visibility fix.
2. **No explicit-rect parameter** — it always computes its own offset/70%-of-monitor placement
   (`get_offset_position`/`get_secondary_window_size`, `:391-392`); there's no way to pass a target
   rect (needed to restore a window to `last_rect` from the launcher's snapshot, or a subwindow to
   its last known position). Needs an `explicit_rect: Option<PaneRect>` parameter that skips the
   placement heuristics when present.
3. **No multi-window loop** — it creates exactly one window per call, as expected; the enumeration
   over "all windows this session should have" is new code regardless of source (launcher snapshot
   or srv query).

---

## 2. Target design

### 2.1 — Two-tier reproject source (the central design decision)

```
On host startup (unconditional, every time — cold or post-crash):
  1. Connect to the launcher (as today), send Register.
  2. NEW: immediately send Command::GetSnapshot.
  3. If a non-empty Event::Snapshot arrives within a short bound (e.g. 500ms — the launcher is
     local IPC, this should be near-instant if it responds at all) AND it lists more than the
     one window the host is about to create by default:
       → FAST PATH: reproject from the launcher's snapshot (`WindowSnapshot.kind`, `.parent_label`,
         `.last_rect`). No srv query needed for topology; srv is still the source for each window's
         workspace/tab/layout content (unchanged from today's per-window bootstrap).
  4. Otherwise (no snapshot, empty snapshot, or launcher itself is also fresh — e.g. after a full
     process-tree kill, a fresh launcher has no in-memory history):
       → SLOW PATH: reproject from srv. Read `Client.windowids`; for each id beyond the one the
         default bootstrap already handles, `GetWindow` to read `kind`/`parent_window_id` (Step 3).
         No `last_rect` available this way (Step 2/3 never persisted window pos/size — see §4 risk).
  5. Either path: drive per-window recreation through `open_window_with_kind` (made `pub(crate)`,
     with the new explicit-rect parameter), in parent-before-child order (a Subwindow's parent must
     exist before the Subwindow is created — trivial to guarantee by sorting FullInstance windows
     first, matching `WindowKind`'s natural precedence).
```

**Why two tiers, not just the srv path:** the srv-only path loses window position/size entirely
(never persisted — a real, separate gap, see §4) and pays a query round-trip per window. The
launcher-snapshot path is strictly better whenever available (the overwhelmingly common case — most
crashes are host-only OOM/panic, not full-tree kills) and costs nothing to prefer. Skipping it would
leave the existing, tested `GetSnapshot`/`Event::Snapshot` machinery permanently dead for no reason.

**Why this doesn't need a new "is this a restart" signal:** the enumeration is written to be
idempotent by construction — on a genuine first-ever launch, the launcher's snapshot has zero
windows (nothing running yet) and srv's `Client.windowids` has zero or one entry, so the "recreate
what's missing beyond the default" loop simply does nothing extra. The same code path handles cold
start and crash-restart uniformly, exactly matching the crash-only-software principle the parent
design doc cites (Candea & Fox: *one* way to start). This is a stronger, simpler resolution than
inventing a restart flag.

### 2.2 — Per-window recreation

For each window beyond the default one:
1. Resolve `kind` (`FullInstance`/`Subwindow`) and, for a `Subwindow`, its parent's **new** label
   (not its old one — the parent is being recreated fresh too, with a new `window-<uuid>` label;
   the reproject driver must track old-label→new-label for the current pass to wire
   `parent_instance_id` correctly. `WindowMirror`'s `parent_label` / srv's `parent_window_id` both
   reference identity, not the literal label string, so this remapping is required either way).
2. Call `open_window_with_kind(state, kind, Some(new_parent_label), None, None, explicit_rect)`.
3. The window's *content* (workspace/tab/layout) is **not** part of this call — exactly as today,
   the frontend inside the newly-created window resolves its own workspace via the existing
   per-window bootstrap (`frontend/app-init.ts`), reading srv directly. This is the part of the
   design doc's claim that *is* still true: content-level reproject reuses existing machinery
   without new deserialization code. Only the **window-set** enumeration is new.

### 2.3 — In-flight state: procedural discipline, not new state management (see §1.C)

State this: reproject must call `open_window_with_kind` (or its future variants) exactly as an
interactive "New Window"/"New Subwindow" action would — never write directly into
`pending_window_creations`, `browser_panes`, or any other transient map to simulate a resumed
operation. This is already true of every existing caller; the spec's job is to make the constraint
explicit so a future contributor doesn't "optimize" reproject into a shortcut that reintroduces the
exact race classes #864/Pillar 2 spent this session eliminating.

### 2.4 — Overlay UX (secondary priority — do not let this block the mechanism)

Two independent, low-coupling pieces, matching the two phases:
- **Pre-window phase** (nothing on screen yet): extend the launcher's native splash to be
  re-spawnable on the crash-restart branches (`supervisor/windows.rs:558-624`/`unix.rs:516-556`),
  with a new headline concept distinct from the stage-telemetry list — "Restoring session…" — and a
  fresh dismiss-event/env-var pair per re-spawn (the original event's consumer thread is already
  gone by the time a crash-restart happens). This is genuinely new launcher-side work, not a text
  swap.
- **In-window phase** (a reprojected window's page is loading): extend `index.html`'s
  `#startup-loading` with an optional query-param-driven headline (e.g. `?restoring=1`) — cheap,
  frontend-only, and the overlay already reappears automatically per the design doc's own
  observation that flicker/rebuild-visibility is a crash-path-only, acceptable event.

**Recommendation: land 2.1-2.3 (the actual mechanism) first, behind no overlay at all if necessary
(a blank moment before the window appears is not worse than today's status quo of the window simply
never reappearing) — then treat 2.4 as a fast follow, not a blocking dependency.** This mirrors how
this session's own splash-hold fix (3s→2s) was folded into an unrelated PR as a "quick addon," not
gating a larger piece of work.

---

## 3. Phased plan

**Phase 1 — wire the launcher snapshot fetch (fast path), no window recreation yet. ✅ Done,
live-verified.** `request_snapshot()` sends `Command::GetSnapshot` right after `COMMAND_TX` is
published in both platform variants of `connect_to_launcher`
(`agentmux-cef/src/launcher_ipc.rs`); the resulting `Event::Snapshot` is logged (window count,
label/kind/parent/last_rect per window) and deliberately **not** broadcast to renderers (it's a
large host-internal payload, not a typed delta the frontend expects — broadcasting it would violate
Phase 1's zero-behavior-change goal). Live-verified on an isolated instance: opened a subwindow,
killed the inner host process only (launcher survives), and the respawned host's snapshot request
came back with `window_count=2`, correctly showing `("main", FullInstance, None, None)` and
`("window-...", Subwindow, Some("main"), Some(Rect{...}))` — kind, parent, AND `last_rect` all
correctly reflected from the launcher's live memory, with zero srv query involved. 159 existing unit
tests unaffected (no test regressions); no new unit tests added for the request/response round trip
itself since Phase 1's own log line was the live-verification signal — revisit if Phase 2 needs a
more structured (non-string-log) handoff.

**Phase 2 — per-window recreation from the fast-path snapshot.** ✅ Done. `open_window_with_kind`
made `pub(crate)` with the explicit-rect parameter; `reproject_from_snapshot` implements the
enumeration + parent-before-child ordering + old-label→new-label remapping from §2.2.

**Addendum 2026-07-07 (post-implementation) — UI-thread-readiness race found and fixed.** First
live-verification pass (kill inner host, 2 extra top-level windows including a subwindow open)
looked like total success at every reducer/log level — `BrowserRegistered`, `WindowOpened`,
`WindowInstanceAssigned` all fired for both recreated labels — but neither window ever appeared in
CDP's target list and the renderer process count never grew. Root cause, confirmed by
cross-referencing timestamps across two independent test runs: `reproject_from_snapshot` runs from
the launcher-ipc reader task, which lives on its own tokio runtime and can complete **before CEF's
UI-thread message loop starts pumping** (`run_message_loop()` in `lib.rs`, called well after
`connect_to_launcher`'s synchronous handshake returns). `post_task(ThreadId::UI, ...)` posted before
that point is a silent no-op — the `CreateWindowTask` it wraps never runs (`execute()`'s own first
log line never appears), even though `post_task` itself returns without error. The orphaned
`PendingWindowCreation` queue entries were then silently claimed by unrelated, concurrently-starting
pool-warmup window creations once the message loop did start, producing exactly the misleading
"reducer says success, CDP says nothing exists" signature.

Fix: `AppState` gained `ui_thread_ready: AtomicBool` and `pending_reproject_snapshot: Mutex<Option<Vec<WindowSnapshot>>>`.
`apply_event_to_shadow`'s `Event::Snapshot` arm stashes the snapshot instead of calling
`reproject_from_snapshot` directly when `ui_thread_ready` is false. `"main"`'s own registration in
`on_after_created` (the first point with direct proof the UI thread is alive — pool-window creations
posted immediately afterward were verified to execute correctly) flips the flag and drains/replays
any stashed snapshot. Re-verified live after the fix: both recreated windows now show
`"[create-window] task entered UI thread"`, appear as real CDP targets, get non-null `windowId`s via
`listWindowInstances()`, and the renderer process count grew by exactly 2 (9 → 11) as expected. See
retro (to be written) for the full timestamp cross-reference.

**Addendum 2026-07-07 (reagent review, PR #2015) — TOCTOU between the two fields above.** reagent
caught a real race in the fix just described: the reader thread's `ui_thread_ready` load and
`"main"`'s registration `store` + stash-drain were two independent operations. If the reader thread
read `ready == false`, then `"main"` registered (flipping `ready` and draining the still-empty
stash) before the reader thread reached its stash write, the snapshot would be written *after* the
one-time drain point had already passed and would never replay — a silent do-nothing regression to
the exact class of bug this addendum's parent fix was written to close, just moved one layer up.
Fixed by collapsing `ui_thread_ready` + `pending_reproject_snapshot` into one field,
`ui_thread_gate: Mutex<UiThreadGate { ready: bool, stashed: Option<Vec<WindowSnapshot>> }>`, so the
reader's check-then-stash and `"main"`'s flip-then-drain are both performed under the same lock
acquisition — whichever runs first is guaranteed complete (and visible) before the other starts, by
construction rather than by timing luck. Re-verified live after this second fix (fresh build, fresh
isolated instance, same kill/respawn methodology): both recreated windows again registered under
their own correct labels with zero mislabeling, and appeared as real CDP targets.

**Phase 3 — slow-path fallback from srv.** ✅ Done. `reproject_from_srv` (in `creation.rs`) reads
srv's `Client.windowids` + each window's `kind`/`parent_window_id` (`backend_get_client_window_ids`/
`backend_get_window_topology`, new blocking read helpers in `client/helpers.rs`, same raw-TCP shape
as every other `backend_*` helper), skips `windowids[0]` (the entry the frontend's own bootstrap
already resolves for `"main"`), and converges on the same `reproject_from_snapshot` driver Phase 2
uses — exactly matching the parent design doc's "both tiers converge on the same per-window
recreation code path."

Trigger point: `"main"`'s registration (`client/lifecycle.rs`) is now also the fast-vs-slow decision
point. `UiThreadGate` gained a `reprojected: bool` (decided under the same lock as `ready`/`stashed`)
so exactly one of {replay a stashed fast-path snapshot, try the slow path} ever runs — necessary
because a fast-path `Event::Snapshot` can arrive either before or after this decision, and a
late-arriving one must not double-create on top of an already-run slow path.

**A real bug was caught during live verification, not by reagent this time — by the test itself**:
the first version's decision logic treated "a stash exists" as "the fast path succeeded," but
`Event::Snapshot` always stashes *something* when it arrives early, even an empty list. A fresh
launcher (the full-process-tree-kill case this phase exists for) sends a real, non-stale snapshot
with zero windows — which the buggy check treated as success, permanently suppressing the slow path
it was supposed to trigger. Fixed by checking whether the stash actually contains anything beyond
`"main"` (`has_extra`), not merely whether a stash is `Some`.

**Live verification** (isolated build, full process-tree kill — outer launcher wrapper + inner host
both killed, letting the Job Object cascade srv's termination too; relaunched fresh from the same
extracted folder so srv's on-disk data survived while all in-memory state, including the launcher's,
was gone): the fresh launcher's snapshot correctly came back empty (`window_count=0`), the slow path
correctly triggered, and reproject recreated all 4 windows srv had on record (the 2 intentionally
opened plus 2 pre-existing pool-warm windows that had also registered backend window IDs) — including
gracefully defaulting 2 legacy rows with no persisted `kind` to `FullInstance` with a warning, rather
than failing. All 4 appeared as real, correctly-labeled CDP targets.

**Phase 4 — overlay UX** (§2.4), as a follow-on, not gating Phases 1-3.

**Phase 5 — E2E test** ("host OOM ⇒ session reprojects", per the parent design doc's §3 acceptance
criterion): automate the Phase 2 manual verification.

Each phase independently shippable, matching every other Pillar 1 spec's phasing discipline this
session established.

---

## 4. Risks / honest caveats

- **Window position/size (`Window.pos`/`winsize`) is not persisted by any live path today** —
  confirmed dead fields per the parent design doc's own §2.C ("genuinely useful for reproject but not
  one of the two facts the design doc's Q1 table names; a natural Step 2.5"). The slow (srv-only)
  path therefore cannot restore exact window placement, only kind/parent/content — windows reproject
  at `open_window_with_kind`'s default offset/70% heuristic, not where the user left them. The fast
  (launcher-snapshot) path *does* have `last_rect` and should use it. This asymmetry should be
  called out to users/reviewers, not silently accepted — restoring approximate-but-wrong-position
  windows is still much better than not restoring them at all, but it's not full parity between the
  two tiers.
- **Old-label→new-label remapping for parent linkage is a real bookkeeping requirement**, not
  automatic — every recreated window gets a fresh `window-<uuid>` label; a naive implementation that
  reuses the *old* parent label when creating a Subwindow will silently produce a dangling
  `parent_instance_id` pointing at a label that no longer exists. Phase 2's implementation must build
  this remap table before creating any Subwindow.
- **The 500ms `GetSnapshot` response bound (§2.1 step 3) is a first guess, not measured.** Local IPC
  should be fast, but this needs empirical tuning during Phase 1's implementation, not an assumed
  constant.
- **This spec does not implement the overlay (§2.4)** beyond scoping it — per the recommendation in
  that section, it should not block Phases 1-3.
- **Multi-monitor / DPI edge cases for restored `last_rect`** (the fast path) aren't addressed here —
  a `last_rect` from a monitor configuration that no longer exists (laptop undocked, external
  monitor removed) needs a bounds-check/clamp-to-nearest-available-monitor fallback, mirroring
  patterns already used elsewhere in this codebase for monitor-aware placement
  (`get_offset_position`/`get_secondary_window_size` presumably already have some of this logic —
  verify and reuse at implementation time, don't invent new monitor-geometry code).

---

## 5. Explicitly out of scope

- Persisting window `pos`/`winsize` to close the fast/slow-path placement-fidelity gap (§4) — a
  natural "Step 2.5" the parent design doc already named, not blocking this spec.
- The saga-layer collapse and graceful-flush-vs-crash incoherence deletion (parent design doc §6
  step 6) — downstream of this spec, not a prerequisite.
- Floating-pane read-back-on-reopen (deferred from SPEC_PILLAR1_STEP2 Slice B) — this spec's
  window-level reproject is the trigger that finally makes that read-back path live/testable, but
  wiring the read-back itself is that spec's follow-up, not this one's.
- Cross-platform (macOS/Linux) parity — this research pass and the cited code (`splash_mac.rs`
  aside) was Windows-focused, matching this session's live-verification methodology throughout.
  macOS/Linux equivalents for the launcher-snapshot fast path and the native-splash re-spawn need
  their own verification pass at implementation time.

---

## 6. Definition of done

1. ✅ `Command::GetSnapshot` is sent by the host on every launcher connection; the response is parsed
   and logged (Phase 1).
2. ✅ Killing the inner host process only (launcher survives) and confirming a multi-window session
   fully reprojects with correct kind/parent/content and approximately correct placement (Phase 2,
   live-verified — see the UI-thread-readiness addendum above for the race that had to be fixed
   first).
3. ✅ Killing the entire process tree and relaunching confirms a multi-window session reprojects
   with correct kind/parent/content, position/size at default placement (Phase 3, live-verified).
4. ✅ No regression in existing single-window cold-start behavior — confirmed across every Phase 2/3
   live-verification run in this session: a plain cold start with no extra windows takes the
   fast-path-empty → slow-path-empty-too (or fast-path-has-data) route and both no-op cleanly, per
   the idempotent-by-construction design in §2.1. 159 unit tests pass throughout, no regressions.
5. ⬜ E2E test automating #2 (Phase 5).

---

## 7. Sources

- `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` (parent design doc, corrected
  2026-07-07 — the doc this spec resolves step 4 of).
- `docs/specs/SPEC_PILLAR1_STEP3_WINDOW_TOPOLOGY_2026_07_07.md` (the srv-side persistence this
  spec's slow path reads from).
- `docs/status/STATUS_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_07_07.md` (the status snapshot that
  recommended this spec be written next).
- Code read for this spec (two research passes, 2026-07-07): `agentmux-launcher/src/state.rs:172-
  259`, `agentmux-launcher/src/reducer/connection.rs:60-221`, `agentmux-launcher/src/host_spawn.rs:
  14-179`, `agentmux-launcher/src/supervisor/windows.rs:199,558-624`, `agentmux-launcher/src/
  supervisor/unix.rs:516-556`, `agentmux-common/src/ipc.rs:240-243,1146`, `agentmux-cef/src/
  launcher_ipc.rs:84-306`, `agentmux-cef/src/app.rs:1125-1233`, `agentmux-cef/src/reducer/mod.rs:65-
  188`, `agentmux-cef/src/state.rs:923`, `agentmux-cef/src/commands/window/creation.rs:237-463`,
  `agentmux-cef/src/ui_tasks/window.rs:1146-1158`, `agentmux-launcher/src/splash.rs:173-338`,
  `frontend/app/element/BrainSpinner.tsx:1-40`, `frontend/app/init/startup-splash.ts:1-38`,
  `index.html:20-82`.
