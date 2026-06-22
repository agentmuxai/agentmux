# SPEC — Reducer single-source-of-truth (SSOT) consolidation

- **Status:** Draft (audit + roadmap; spec-first, rides with code per no-doc-only-PR rule)
- **Date:** 2026-06-22
- **Author:** AgentA
- **Origin:** the stale/desynced window-count bug (`SPEC_WINDOW_COUNT_STALE_ON_VIEWS_CLOSE_2026_06_22.md`)
  turned out to be one instance of a systemic pattern — state reconstructed/duplicated outside a
  single reducer authority. The user asked: *"check if there are other loose ends that should be in
  the reducer."* This is the rigorous answer.
- **Supersedes/absorbs:** extends `SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md` — that spec
  predates the #1676 win_event fix, which introduced a NEW fourth quit authority this document must
  now account for (finding L1).
- **Evidence:** two grounded codebase audits (frontend desync surface; host/launcher SSOT). All
  file:line citations below are from those audits against the live tree at repo root (NOT the
  `.claude/worktrees/*` copies).

---

## 1. The principle (what "in the reducer" means)

Four rules. A violation of any is a "loose end":

- **P1 — Single owner.** Each piece of state has exactly ONE authoritative home (a reducer over
  `HostState` on the host, or the launcher's event-sourced mirror). No second copy that can drift.
- **P2 — Decide by re-running a pure function, not by edge-triggered ad-hoc checks.** Lifecycle
  decisions (quit, drain, promote) are pure `fn(&HostState) -> Decision` re-run after EVERY
  transition — not recomputed independently at each call site with subtly different logic.
- **P3 — Read the authority; never reconstruct from a lossy stream.** Renderers (and the launcher
  mirror) reflect authoritative state via reliable snapshots/reconciliation, not by replaying a
  best-effort incremental event feed that silently drops.
- **P4 — Type state; don't parse it back out of labels.** Identity/role lives in a typed enum
  (`BrowserKind`), not reconstructed from `label.starts_with("window-pool-")` string prefixes — which
  are provably wrong (a promoted pool window keeps its `window-pool-*` label but is no longer a pool
  window).

The just-merged #1676 lifecycle bug violated P2 (quit decision duplicated/edge-triggered) and P4
(user-window-ness parsed from label prefixes). The window-count desync violates P3. The audit below
shows these are not isolated.

---

## 2. The six-headed counter (the canonical evidence)

The lifecycle spec flagged THREE "is this the last user window?" computations. There are **six**, over
**three different data sources**, each with its own exclusion rule:

| # | Site | Data source | Exclusion rule | Purpose |
|---|------|-------------|----------------|---------|
| 1 | `state.rs:1096` `host_counts_snapshot` | CEF `browsers` map | `browser-pane-*` ∪ `floating-pool-*` ∪ pool-set | launcher pool-mirror drift feed |
| 2 | `state.rs:1178` `user_visibility_snapshot` | `browsers` map | `pool.{unpromoted,queue}` ∪ `pane_pool.{…}` ∪ `browser-pane-*` | on_before_close logging |
| 3 | `reducer/quit.rs:116` `count_live_user_windows` | `browsers` map | `TopLevel{is_pool:false}` AND not `floating-pool-*` | "authoritative" quit count |
| 4 | `wrr/win_event.rs:236` `count_visible_user_windows` | **Win32 `EnumWindows`** | `IsWindowVisible` + `Chrome_WidgetWin_` + `x > -20000` pixel heuristic | **the LIVE quit trigger (post-#1676)** |
| 5 | `saga_dispatch.rs:355` `drain_pool_if_last` | launcher state | `user_count==0 \|\| (==1 && label_present)` | launcher DrainPoolIfLast |
| 6 | `orphan_reconcile.rs:101` `live_user_count` | **launcher `shadow_window_meta`** | keys minus `browser-pane-*` | orphan reconcile drain |

Three sources (CEF `browsers` map, Win32 enumeration, launcher shadow projection) and four distinct
exclusion rules computing the *same predicate*. Sites 1 and 3 must hand-mirror each other's
`floating-pool-*` exclusion via cross-referencing comments (`quit.rs:106` literally points at
`state.rs:~1121`) — change one, silently desync the other. **This is the single most dangerous SSOT
violation in the tree.**

---

## 3. Ranked findings

Severity = likelihood × blast-radius of the bug it can cause. 🔴 active/likely bug source, 🟠 real
drift risk, 🟡 latent debt.

### Track 1 — Lifecycle reducer (the quit/drain/count/typing cluster)

**L1 🔴 — The live quit decision bypasses the reducer entirely (post-#1676).**
`wrr/win_event.rs::maybe_quit_on_last_user_window` (win_event.rs:284-322) is now the actual quit
trigger; it calls `cef::quit_message_loop()` (win_event.rs:321) based on a Win32 `EnumWindows` +
`OFFSCREEN_POOL_THRESHOLD_X=-20000` pixel heuristic (win_event.rs:236-279) and two `static AtomicBool`s
(`HAD_VISIBLE_USER_WINDOW`, `QUIT_INITIATED`, win_event.rs:218-223). `QuitState` in the reducer
(`reducer/mod.rs:113`) is **not consulted** for the decision. The `-20000` threshold is itself
duplicated into `commands/window/lifecycle.rs` (win_event.rs:226-227). Two MORE live exit primitives
still exist: `client/mod.rs:1186` (on_before_close Stage 2) and `orphan_reconcile.rs:335,406`.
*This was the correct emergency fix for #1676 (the Views close HIDES rather than firing
on_before_close, so the reducer never saw the transition) — but the right end-state is to feed that
HIDE/DESTROY OS signal INTO the reducer as a transition, then let `reconcile_quit` decide, rather
than fork a parallel coordinate-based authority.*
→ Owner: `reducer::quit::reconcile_quit` over typed state; the OS hook becomes a transition source, not a decider.

**L2 🔴 — `reconcile_quit`/`should_begin_drain`/`user_creation_in_flight` are built, tested, UNWIRED.**
`reducer/quit.rs:131-167`, all `#[allow(dead_code)]` (quit.rs:73,130,143,160), zero production callers
(grep-confirmed: only quit.rs + tests). The level-triggered reconciler the lifecycle spec is built
around already exists and is dormant; the live paths still edge-trigger (L1). quit.rs:56-60 documents
the intended wiring as "the explicit next step."
→ Wire `reconcile_quit(state)` at the tail of the browser-deregister, promote-complete, pane-reap, and
creation-abort reducer arms; surface a "should exit" signal in `DispatchOutput` consumed by the
UI-thread exit primitive. **Prerequisite: L4** (`user_creation_in_flight` currently parses pending-
creation labels because `PendingWindowCreation` has no `source` field — state.rs:104-108).

**L3 🟠 — Six counts, three sources (§2).** Route sites 1, 2, 5 (and ideally 4) through the single
`reducer::quit::count_live_user_windows` (quit.rs:116; `AppState` already delegates at state.rs:1007).
Site 6 (`orphan_reconcile`) reads the lagging launcher shadow projection — a genuinely different
source that disagrees during the promote race; fold its decision into `reconcile_quit`.

**L4 🟠 — Pane-pool has no `BrowserKind` variant → label-prefix classification in the hot path.**
`BrowserKind` (state.rs:242-249) is `TopLevel{is_pool}` | `Pane{block_id}` — no pane-pool variant. So
`floating-pool-*` windows register as `TopLevel{is_pool:false}` (classifier at client/mod.rs:434-446
only special-cases `window-pool-`), indistinguishable BY TYPE from real user windows — only the
`floating-pool-` label prefix separates them. This is the literal cause of the #1676 reagent P0. And
the **promoted-pool trap** (P4): `PopAndPromoteFrontPoolWindow` flips `is_pool=false` (pool.rs:104-106)
but the label stays `window-pool-*` forever, so prefix-classifying a `window-pool-*` label as "pool"
is wrong for promoted windows — documented at quit.rs:83-85, client/mod.rs:993-996,
orphan_reconcile.rs:709-725, yet prefix logic persists at ~20 sites (enumerated in the audit; key
hot-path ones: client/mod.rs:440/1101/1345, quit.rs:75-77/110, state.rs:1122-1124, saga_dispatch.rs:367,
orphan_reconcile.rs:104/302-304, meta.rs:111, motion.rs:411/435, lifecycle.rs:478).
→ Extend `BrowserKind` with a pane-pool variant (lifecycle spec §5.2/§10.3). Then L3's `floating-pool-`
exclusions disappear (they become a typed match) and L2's `user_creation_in_flight` can read a typed
`source`. **This is the keystone prerequisite for L2/L3.**

**Chosen design (2026-06-22) — a typed `Floater` variant, not just pane-pool.** Investigation showed
the gap is broader: there are TWO floater labelings (`floating-<uuid>` direct, `floating-pool-<uuid>`
pane-pool), and the count gate only excluded `floating-pool-` — so direct floaters were *accidentally*
counted as instance-keeping windows while pane-pool floaters were not (same user-facing thing,
opposite behavior). The fix:
- Add `BrowserKind::Floater { is_pool }` (warm pane-pool = `is_pool: true`; promoted/visible =
  `is_pool: false`, flipped at the `pane_pool.rs` promote handler).
- Classify BOTH `floating-` and `floating-pool-` into `Floater`.
- `is_live_user_window` stays `matches!(kind, TopLevel { is_pool: false })` → **all** floaters are
  excluded *by type*; the `!starts_with("floating-pool-")` string check is deleted, and
  `counts_as_live_user_window` collapses back into `is_live_user_window` (no label needed).
- **Policy CANONIZED:** floaters do NOT keep the instance alive (invariant **FP-LIFE**,
  `docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` §1.1), aligning the reducer
  count with the win_event quit trigger that already excludes floaters by class. User-confirmed
  2026-06-22.
- Out of scope for this slice (separate concerns, left label-based for now): `host_counts_snapshot`
  (must match the launcher mirror's report-driven count, NOT the keep-alive policy) and the
  `pending_window_creations` prefix classifier (pre-registration — no `BrowserKind` exists yet).

**L7 🟡 — H.6 top-level creation runner built, fully dormant.** `HostState.top_level_creation`
(mod.rs:123), all of `reducer/top_level.rs`, the `EnqueueTopLevelWindow`/`PostCreateWindow` machinery —
implemented + tested, **never dispatched** (every create calls `ui_tasks::post_create_window` directly:
drag.rs:506, floating_pane.rs:353, window/creation.rs:376, window_pool.rs:389/1401; mod.rs:116-121
admits "DORMANT"). It is the designed owner of "user creation in flight" (`InFlightCreation.source:
TopLevelSource`, state.rs:333) — exactly L2's prerequisite, typed instead of prefix-matched. Wire it
WITH L2 or not at all (a half-wired runner is worse).

**L8 🟡 — `orphan_reconcile` is a second drain authority over a third data source.** It recomputes
`begin_drain` (orphan_reconcile.rs:145-147) and independently dispatches `BeginDrain`/`quit_message_loop`
(orphan_reconcile.rs:335,346,406) off the launcher's `HostShouldQuit`, counting from `shadow_window_meta`.
Its in-flight/`freshly_promoted` guards (orphan_reconcile.rs:122-147) are load-bearing and MUST be
preserved when folding its drain branch into `reconcile_quit` (lifecycle spec §10.2). Note the B.9.3
history: launcher-side `HostShouldQuit`/`DrainPoolIfLast` is documented ADVISORY — likely redundant
once the host self-reconciles (do NOT re-propose launcher quit authority — it was tried + demoted).

### Track 2 — HWND/role registry (the drag/redock cluster)

**R5 🟠 — Redock target resolution reconstructs labels from prefixes + side channels.**
`resolve_window_at_cursor` (motion.rs:272+) classifies each HWND under the cursor every drag-frame by
`window-pool-` prefix + `backend_window_id` presence + an `is_main_frame` heuristic
(motion.rs:410-430), with a special "unregistered pool label on the main frame → promoted, renderer is
'main'" case (motion.rs:415-419) and `starts_with("floating-")` exclusion (motion.rs:435). The memory
`reference_redock_onto_main_pool_mislabel.md` records this exact reconstruction silently failing
(redock-onto-main no-op); `ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` names it recurring
regression root cause #1.
→ Owner: a reducer holding `label ↔ outer-HWND ↔ role` so the walk looks role up by HWND.

**R6 🟠 — `window_hwnds` HWND cache is an uncoupled `Mutex<HashMap>` on `AppState`.**
`AppState.window_hwnds: Mutex<HashMap<String,isize>>` (state.rs:822), authoritative for window
resolution (consulted first, lifecycle.rs:260-281), populated by `capture_hwnd_for_label`
(lifecycle.rs:462,492) with no transactional coupling to the `browsers` map or pool promotion. The
fallback capture (lifecycle.rs:468-495) picks "first eligible visible HWND" with an off-screen skip
gated on the `window-pool-` prefix (lifecycle.rs:478) — the class of bug behind PR #1165
(`window_hwnds["main"]` bound to a warm-pool HWND → drag/close no-op, memory
`reference_window_drag_pool_hwnd_bug.md`). Capture, promotion, and HWND-cache writes are three
separate locks → a promoted window's HWND can cache under the wrong label.
→ Owner: a reducer map mutated atomically with `RegisterBrowser`/`PromotePoolWindow` (the
`pane_window_states` reducer, mod.rs:142, already proves the pattern). *Caveat: the cache's EXISTENCE
is defensible (CEF Views hides the main HWND; `host.window_handle()` returns an inner WS_CHILD,
lifecycle.rs:442-454). The violation is that it is authoritative AND uncoupled, not that it exists.*

### Track 3 — Reliable launcher→renderer delivery

**D-count 🔴 — the window-count desync.** Per-renderer reconstruction from a lossy versioned stream
with a log-only gap handler and a dev-gated reconcile path. Fully specified in
`SPEC_WINDOW_COUNT_STALE_ON_VIEWS_CLOSE_2026_06_22.md` §8-11 (the resync-on-gap fix). Owner: renderers
reconcile against the authoritative `list_window_instances` on a detected gap.

**D-srv 🟡 — latent twin.** `srv-events.ts` shares the same lossy `PerSourceTracker` + log-only gap
handling but has ZERO production consumers today; it becomes a desync risk the moment an srv
atom-router subscribes. Apply the same resync-on-gap then, paired with an srv `GetSnapshot`/`Resync`
(unbuilt Phase D). Flagged to prevent a silent future regression.

---

## 4. What is FINE as-is (explicitly not loose ends)

So the consolidation doesn't over-reach:
- The drag reducer (`active_drag` in `HostState`, drag.rs), pane-lifecycle/pane-window-placement
  reducers (`browser_panes`, `pane_window_states`), and the browser registry (`browsers` map, fully
  migrated into `HostState`, H.2.e complete) — correctly reducer-owned.
- The launcher pool/window **mirror** + `DriftDetected{Pool}/{Windows}` (launcher/reducer/pool.rs:15-61,
  window.rs:68-265) — correctly event-sourced and host-authoritative; it's a DETECTOR, not a second
  authority. The loose end is the bespoke `host_counts_snapshot` FEED (L3), not the mirror.
- Agent-document / pane stores fed by the WPS broker ring — self-healing by design (the ring persists
  and replays on subscribe, `useAgentStream.ts:226-289`). Fundamentally different from the monotonic
  `PerSourceTracker` pipes; not a version-gap risk.

---

## 5. Sequencing (dependency-ordered)

1. **D-count (Track 3)** — ship now; it fixes the reported user-visible bug and is low-risk (wires
   existing pieces). Rides with the close-fix (already done).
2. **L4 (typed pane-pool `BrowserKind`)** — the keystone; unblocks L2/L3 and removes ~20 prefix sites.
3. **L3 (single counter)** — collapse the six counts onto `count_live_user_windows` once L4 makes the
   exclusions typed.
4. **L2 + L7 (wire `reconcile_quit` + H.6)** — make the quit/creation decision level-triggered and
   reducer-owned; requires L4's typed `source`.
5. **L1 (retire the win_event parallel authority)** — feed HIDE/DESTROY into the reducer as a
   transition; delete the pixel-coordinate heuristic once `reconcile_quit` observes the transition.
   Do this LAST (it's the live quit path — must not regress #1676).
6. **L8 (fold orphan_reconcile drain into reconcile_quit, preserving guards).**
7. **Track 2 (R5/R6)** — separable; tackle after Track 1 stabilizes. Would also kill the recurring
   drag/redock HWND regressions.

Each step is independently shippable and independently testable.

## 6. Hard constraints (from the lifecycle spec §10 adversarial review — do NOT relitigate)
- `reconcile_quit` must be a PURE decider (`fn(&mut HostState)`); it must NOT call `quit_message_loop()`
  (UI-thread-only; off-thread = silent no-op = the v0.33.492 bug) and must NOT re-lock `host_state`
  (non-reentrant; v0.33.498 deadlock). Exit primitive stays UI-thread; off-UI transitions use
  `PostThreadMessage(WM_QUIT)`.
- Launcher-side single quit authority was tried (B.9.3) and demoted — `HostShouldQuit` is ADVISORY.
  "Single authority" means HOST-side. Don't re-propose launcher authority.
- Preserve `orphan_reconcile`'s in-flight/freshly-promoted guards; the New-Window + last-close corner
  must NOT quit.
- Extend `BrowserKind`; do not invent a parallel `WindowRole` taxonomy.

## 7. Testing (depends on the CI runner)
These consolidations are exactly what the absent CI test runner
(`SPEC_CI_TEST_RUNNER_2026_06_22.md`) must cover: the keystone is a **host-reducer integration test**
asserting `reconcile_quit` over synthetic transition sequences (open → promote → close-last →
new-window-mid-close → …) reaches the right `QuitState`. The end-to-end "tree exits" check stays a
local smoke (CI can't drive live CEF). Land the reducer tests alongside each step above so the
#1676-class regression cannot recur silently.
