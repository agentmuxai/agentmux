# SPEC — Instance Lifecycle Consolidation

- **Status:** Draft (spec-first; no code yet). **Adversarially reviewed 2026-06-21 —
  see §10; several §3–§7 claims were corrected by that review. Read §10 before
  implementing.**
- **Date:** 2026-06-21
- **Author:** AgentA
- **Tracking:** TBD (open a Discussion for long-term lifecycle work; link PRs here)
- **Supersedes/absorbs:** the scattered "Phase B.9.3 / F.6 / H.5" quit-path commentary;
  folds in the unfixed `docs/specs/window-close-process-cleanup.md`
- **Related:** `docs/retro/b9-3-quit-thread-analysis.md`,
  `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`,
  `docs/specs/SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md` (I1–I6)

---

## 1. Problem

Closing the **last (and only) window** of an instance does **not** shut the instance
down. Observed 2026-06-21 on a portable build: after the user closed the window, the
full process tree stayed alive — **13 processes**:

```
agentmux.exe (launcher) ─────────── still alive (Job Object J0 holder)
├─ agentmux-<ver>.exe (CEF host) ── still alive  ← never quit
│   └─ 8× CEF subprocess (gpu/renderer/utility)
└─ agentmux-srv (sidecar) ────────── still alive
    ├─ agentmux-srv (worker)
    └─ claude.exe (agent) ────────── still running
        └─ agentmux-mcp
```

### 1.1 Evidence (host log of the orphaned instance)

The still-running host's log shows **no quit cascade at all** — none of the WARN-level
markers (`[wrr] stage 1`, `BeginDrain`/`quit_state=Draining`, `quit_message_loop`)
appear after the close. Instead it shows continuous memory heartbeats up to "now" plus
a **window-create** event *immediately after* the close:

```
[wrr] callback event=0x8000 hwnd=0x1004ae
[wrr] EVENT_OBJECT_CREATE app-class hwnd=0x1004ae class=Chrome_WidgetWin_0 title=""
...
"process memory" ws_mb=111.4   (still heartbeating minutes later)
```

A new top-level `Chrome_WidgetWin_0` was **created** right after the user closed theirs.
That is a **warm-pool refill**.

### 1.2 The failure chain

> close last visible window → the last-window gate (`user_browser_count == 0`)
> does **not** observe zero (a pool window is mid-promotion / just refilled) →
> `BeginDrain` is never dispatched → `QuitState` stays `Running` → pool refill is
> **not** suppressed → a fresh pool window is spawned → `browser_list` never empties →
> `quit_message_loop()` is never called → **host never exits** → launcher's supervised
> `host_child.wait()` never returns → Job Object J0 never closes → **the entire tree
> is orphaned**, including a live agent.

The teardown machinery downstream of "host exits" is **correct and robust** (see §3.3).
The single point of failure is the host's decision to quit.

> **✅ Root-cause CONFIRMED (mechanism A — gate never armed), 2026-06-21.** The caveat
> below was checked against the live orphan before writing any fix. **Evidence:** the
> orphan host log (`channels/local-agenta-cef-log-robustness-…/…host-v0.46.6.log`,
> 9,483 lines) contains **zero** occurrences of `BeginDrain` / `drain mode` /
> `stage 1` / `quit_message_loop` / `Draining` — the host never attempted to quit. This
> *rules out* the two alternative theories: (B) a wedged pool window blocking Stage-2
> would have logged `BeginDrain`/`drain mode` first; and the §6.3 abnormal-host-death
> orphan doesn't apply because the host is alive and heartbeating. So the failure is the
> **quit decision never firing** — precisely what level-triggered `reconcile_quit`
> targets. (The two same-day retros below describe the *related but distinct*
> promotion-wedge-→-blank-"CEF"-window symptom — `retro-windows-terminal-window-leak-2026-06-21.md`
> §2.3/§6.2 and `retro-blank-new-window-2026-06-21.md` — which is a usability failure,
> not this failure-to-quit. Worth fixing separately; not this orphan's cause.)

---

## 2. Why this keeps regressing (the real thesis)

The quit decision is the most-patched code in the repo and it still breaks. The reason
is structural, not a one-off bug:

1. **It is edge-triggered.** "Should we quit?" is computed *only* inside
   `on_before_close` (and a second time in `orphan_reconcile`). It fires on the
   close **event**. But pool promotion / window creation is **concurrent** and
   level-changing. A promotion that lands in the window between "last close fires" and
   "gate reads the count" leaves the system in a **non-quitting steady state with no
   event left to re-trigger the evaluation.** Nothing ever re-checks "are we now at
   zero user windows and not draining? → quit."

2. **The decision is duplicated across three sites that must agree:**
   - `agentmux-cef/src/client/mod.rs` `on_before_close` — the live two-stage cascade.
   - `agentmux-cef/src/commands/orphan_reconcile.rs` — recomputes `begin_drain` from
     `safe_to_drain` / `freshly_promoted`, and where it declines it *defers* with the
     comment *"the next `HostShouldQuit` will catch it"* — i.e. it explicitly relies on
     a future edge that, in the last-window case, never comes.
   - `agentmux-launcher/src/saga/window_cleanup.rs` — a saga that brackets the same
     decision for renderers. Its own docstring says the host commands are "log-only…
     no such pipe exists yet," then a later "**CPD-3 update**" in the *same file* says
     they are "now LIVE." That internal contradiction is a fossil record of the churn.

3. **The "user-visible window" predicate is implicit and label-string-based.**
   `user_visibility_snapshot()` returns `(pool_labels, browsers)` and the count is
   `browsers.filter(|l| !pool_labels.contains(l) && !l.starts_with("browser-pane-"))`.
   Whether a window counts toward "keep the app alive" is reconstructed, every time,
   from set-membership + a string prefix. Pool (`window-pool-*`), pane-pool
   (`floating-pool-*`), promoted-but-not-yet-deregistered, `browser-pane-*` sub-views,
   and tear-off/floating windows each need bespoke handling, and each has been a bug.

4. **Near-zero end-to-end coverage.** The `QuitState` reducer transitions are
   unit-tested, and the cleanup saga is unit-tested. But there is **no test that asserts
   the actual goal**: "close the last window ⇒ the process tree exits." So a regression
   in the *gate that feeds* the well-tested machine ships silently — which is exactly
   what happened.

**Fragmentation metric:** ~38 distinct `Phase B.x / F.x / H.x`, `codex #…`,
`reagent #…`, `smoke v0.33.x` markers in the quit path; ~40 lines of race-condition
apologetics inside a single ~460-line `on_before_close`. This is layered patching, not
a designed state machine.

---

## 3. Current architecture (verified map)

### 3.1 Host (`agentmux-cef`) — owns the quit decision + the CEF message loop

| File | Responsibility |
|---|---|
| `src/client/mod.rs` `on_before_close` | **Live two-stage cascade.** Stage 1 (when `user_browser_count == 0 && !is_browser_pane`): dispatch `BeginDrain`, then `PostMessage(WM_CLOSE)` to every `window-pool-*` + `floating-pool-*` browser. Stage 2 (when `self.browser_list.is_empty()`): `quit_message_loop()`. |
| `src/state.rs` | `QuitState { Running, Draining{reason}, Quit }`, `QuitReason`, `user_visibility_snapshot()`, `is_quitting()`. Sole source of truth = `HostState.quit_state`. |
| `src/reducer/quit.rs` | `handle_begin_drain` (Running→Draining, idempotent), `handle_confirm_drained` (Draining→Quit). Emits `QuitDraining` / `QuitReady`. |
| `src/reducer/mod.rs` | `HostCommand::BeginDrain` / `ConfirmDrained`; pool-spawn arm suppressed while `quit_state != Running` (this is what *should* stop refill once draining). |
| `src/commands/orphan_reconcile.rs` | **Second decision site.** Builds a reconcile plan (`begin_drain`, `closes`, `freshly_promoted`); defers when "live user windows or freshly-promoted candidates remain." |
| `src/commands/window_pool.rs` | Pool spawn / promote / `on_pool_window_destroyed` (refill). Refill-during-drain is the loop that keeps the host alive when the gate misfires. |
| `src/launcher_ipc.rs` | `report_panes_reaped`, `report_pool_drain_decision`, `ReportWindowClosed` → launcher. |

### 3.2 Common (`agentmux-common/src/ipc.rs`) — the wire contract

- Host→launcher: `Command::ReportWindowClosed`, `ReportPanesReaped`,
  `ReportPoolDrainDecision`.
- Launcher→host (saga-issued): `Command::ReapPanes`, `Command::DrainPoolIfLast`.
- Launcher broadcast: `Event::WindowClosed`, `PanesReaped`, `PoolDrained`,
  `PoolNotLast`, `HostShouldQuit`, `SrvWindowClosed`.

### 3.3 Launcher (`agentmux-launcher`) — owns the process tree (this part is sound)

| File | Responsibility |
|---|---|
| `src/main.rs` | Creates **Job Object J0 with `KILL_ON_JOB_CLOSE`** (killing the launcher reaps the tree). Runs the **supervised wait loop**: `select!` on `host_child.wait()` / `srv_child.wait()`; classifies clean exit vs signal/group-shutdown vs system-OOM vs crash; relaunches host within a budget on abnormal exit; SIGTERM-then-SIGKILL children on shutdown. |
| `src/srv_spawner.rs` | Spawns/owns the sidecar. |
| `src/mem_supervisor.rs` | `classify_host_exit` (OOM-aware relaunch). |
| `src/saga/window_cleanup.rs` | Narrates the host's close cleanup as a `SagaStarted`/`SagaCompleted` bracket. |
| `src/saga/pool_respawn.rs`, `src/reducer/{pool,saga,window,connection}.rs` | Pool-respawn saga + launcher-side window/pool/saga mirror state. |

**Key property:** the launcher does the right thing **the moment the host process
exits** (code 0 → "shutting down" → SIGTERM children → drop J0 → tree dies). It is
*entirely* gated on host exit. No host exit ⇒ no teardown. This is why the fix belongs
in the host's quit decision, not in the launcher.

### 3.4 Sidecar (`agentmux-srv`) — mostly already handled (corrected by §10.4)

**Correction (§10.4-D, H1/H2):** the `window-close-process-cleanup.md` gap is **already
implemented** in the current tree — `agentmux-srv/src/backend/wcore/tab.rs:99-116`
`delete_tab_inner()` calls `delete_controller(block_id)` ("Kill shell processes FIRST")
**before** deleting DB rows, and `wcore/window.rs:139-161` cascades window→workspace→tabs.
Also, the **last-window** path does *not* use this cascade at all: `backend_close_window`
is in the `else` branch of `on_before_close` (`client/mod.rs:1186-1218`), reached only
for *secondary* windows; on the last window the host quits and **Job Object J0 reaps the
tree**. So LC-7 for the reported 13-orphan bug is delivered by the host-quit fix
(Stages 0–2), **not** by sidecar work. The residual sidecar item is narrower than first
written — see §5.4 (revised) and §10.4.

---

## 4. Design goals & lifecycle invariants

The instance lifecycle should be expressible as a small set of invariants that hold
**by construction**, not by 38 patches keeping each other honest.

- **LC-1 — Single quit authority.** Exactly one component decides "this instance
  should quit," from one source of truth. No second opinion in `orphan_reconcile`, no
  third in a saga.
- **LC-2 — Level-triggered, not edge-triggered.** "Should we quit?" is a pure function
  of current state, re-evaluated after **every** transition that can change the live
  user-window set (close, promote-complete, creation-abort, tear-off, redock). A
  promotion racing the last close can delay quit by one tick but can never strand the
  instance alive.
- **LC-3 — Explicit window taxonomy.** Every registered window/browser carries a typed
  **role** (`UserTopLevel`, `TabPool`, `PanePool`, `BrowserPaneChild`,
  `FloatingPane`, …) at registration time. "Counts toward keep-alive" is
  `role == UserTopLevel`, read off the record — never reconstructed from a label prefix
  + pool-set membership at decision time.
- **LC-4 — Refill obeys quit intent atomically.** The transition to "draining" and the
  suppression of pool refill happen in one reducer step. No refill can be in flight
  that the drain decision didn't see.
- **LC-5 — One exit path.** Collapse the three async bridges (Win32 `PostMessage`, CEF
  `post_task`, tokio `PostThreadMessage`) to a single documented path per platform,
  justified by `b9-3-quit-thread-analysis.md`, with the others deleted.
- **LC-6 — Preserve isolation (I1–I6).** Any change to launcher process/pipe/job code
  is reviewed against `SPEC_MULTI_INSTANCE_ISOLATION_HARDENING_2026_06_03.md`. The Job
  Object `KILL_ON_JOB_CLOSE` topology is **kept** — it is the part that works.
- **LC-7 — Complete teardown.** Quitting an instance kills its shells/agents too
  (folds in §3.4). "Cleaned up" means zero surviving PIDs in J0.
- **LC-8 — Observable + tested.** The end-state ("tree exited") is asserted by an
  automated test, and the decision inputs are logged at one place at WARN.

---

## 5. Target architecture

### 5.1 A single host-side **Lifecycle reducer** (the quit authority)

Promote the quit decision from "computed inside a CEF callback" to a **pure reducer**
over the host window registry:

```
QuitState: Running → Draining{reason} → Quit         (unchanged; already good)

live_user_windows(state) = count of records where role == UserTopLevel
                           and lifecycle == Registered (not closing, not pool)

reconcile_quit(state):
    if state.quit_state == Running
       and live_user_windows(state) == 0
       and no UserTopLevel creation is in flight:
         → emit BeginDrain{ LastWindowClosed }     // suppresses refill (LC-4)
    if state.quit_state == Draining
       and pool_browsers(state).is_empty()
       and all_browsers(state).is_empty():
         → emit ConfirmDrained                     // host then calls quit_message_loop once
```

`reconcile_quit` is called by the reducer **after every** window/pool transition
(register, deregister, promote-start, promote-complete, creation-abort, pane-reap).
This is LC-2: the last-window-vs-promotion race becomes a one-tick delay, never a
permanent stall. `on_before_close` shrinks to: deregister this browser → `reconcile_quit`
→ if state says `Quit`, call `quit_message_loop()`. It stops *deciding*; it only
*reports a transition*.

`orphan_reconcile`'s `begin_drain` branch is **folded into** `reconcile_quit` — but its
in-flight-creation and `freshly_promoted` **guards are preserved, not deleted** (they are
load-bearing; see §10.2 + §10.3). The deferral logic moves; the protection stays.

> **⚠ Threading & exit-primitive contract (§10.1 — CRITICAL).** `reconcile_quit` is a
> **pure `fn(&mut HostState)`** run inside `update()` while `host_state` is locked. It
> may **only** mutate `QuitState` and push events. It must **never** call
> `quit_message_loop()`, `host_dispatch`, `is_quitting()`, `user_visibility_snapshot()`,
> `spawn_pool_window`, or any helper that re-locks `host_state` (the lock is
> non-reentrant — `state.rs:901`; inline quit deadlocks the UI thread — v0.33.498,
> `client/mod.rs:1059-1062`). Transitions arrive on **multiple threads** (UI close;
> tokio IPC promote/ready — `ipc.rs:108`, `window_pool.rs:607/669`), so the **decision**
> may be observed anywhere but the **exit primitive stays exactly where it is today**:
> the UI-thread `on_before_close` Stage-2 site, gated on `browser_list.is_empty()`
> (`client/mod.rs:1186`), now *also* checking `quit_state == Quit`. `reconcile_quit`
> changes only the *predicate*, never the *call site*. The Stage-1 pool-drain executor
> (UI-thread `PostMessage(WM_CLOSE)` to pool browsers — `client/mod.rs:1101-1156`) is
> **kept**: `BeginDrain` only suppresses *future* refill, it does not close already-spawned
> pool windows. When an off-UI transition drives the decision to `Quit`, loop-exit is
> delivered via the proven Win32 `PostThreadMessage(WM_QUIT)` bridge, never a direct CEF
> quit call.

### 5.2 Typed window roles (LC-3) — **extend the existing taxonomy, don't invent one**

**Corrected by §10.3.** Two typed taxonomies already exist and are already wired through
the registry: **`BrowserKind { TopLevel { is_pool }, Pane { block_id } }`**
(`state.rs:240-249`, set in `on_after_created`, `client/mod.rs:434-446`) and
**`WindowKind { FullInstance, Subwindow }`** (`state.rs:70`, wire-mirrored at
`ipc.rs:780`). Inventing a third `WindowRole` enum would recreate the very "three sites
must agree" problem this spec exists to kill.

Instead: **extend `BrowserKind`** to cover the pane-pool (`floating-pool-*`) distinction
it currently lacks, and migrate the remaining `label.starts_with(...)` lifecycle
predicates onto `kind` matches. `UserTopLevel` = `TopLevel { is_pool: false }`;
`TabPool` = `TopLevel { is_pool: true }`; `BrowserPaneChild` = `Pane {..}`.

Two hard constraints from history (§10.3):
- **Role must be assigned from the label at registration**, *not* from the per-client
  `is_browser_pane` flag — `CreateWindowTask` reuses an existing browser's client, so the
  flag misclassifies top-levels as panes (the v0.33.586 regression, `client/mod.rs:417-446`).
- **Role must be *mutated* on every lifecycle transition**, not just set once: promote
  (pool→user, mirror the `is_pool=false` flip in `pool.rs:104`), tear-off
  (Subwindow→FullInstance), redock (→child). §7.3's transitions are the spec; "set at
  registration" alone is wrong.

### 5.3 Launcher: keep the topology, demote the saga

- **Keep** Job Object J0 `KILL_ON_JOB_CLOSE` + the supervised `host_child.wait()`
  teardown verbatim (LC-6). No change to isolation-critical code.
- The `window_cleanup` saga becomes **purely observational** (renderer bracketing) and
  is documented as such — resolve the docstring/CPD-3 contradiction by deleting the
  "live launcher→host drain command" path if the host now self-reconciles (LC-1). The
  host does not need the launcher to tell it to drain.

### 5.4 Sidecar cascade (LC-7) — **re-scoped: audit + preserve the workspace guard**

**Corrected by §10.4 (H1/H3).** The cascade is **already implemented**
(`wcore/tab.rs:99-116` kills controllers before DB delete; `wcore/window.rs:139-161`
cascades). So Stage 4 is **not** "implement it" — it is **"audit + add the missing
test."** The one thing the original wording got dangerously wrong: it described the
**unguarded** `wcore::close_window` path. The *live* `CloseWindow` handler routes through
`server/service.rs:954-1031`, which deletes the workspace/controllers **only if no other
live window points at the same workspace** (`service.rs:1010-1025`). Two windows *can*
share a workspace (`CreateWindow` accepts an existing `workspace_id`; `SwitchWorkspace`
repoints — `service.rs:867`). Any cascade work **must** go through that shared-workspace
guard, never the raw `wcore` functions, or closing window A kills window B's agents.

Note on "agents are GLOBAL": that means *definitions/auth/transcripts* are globally
visible (cross-channel work), **not** that running processes are shared. Controllers are
**per-`block_id`** (`blockcontroller/mod.rs:202,226`), so killing a block's controller
kills only that block's `claude.exe`. The real risk is workspace-sharing, above — not
global-agent processes.

---

## 6. Implementation stages (each behind tests; ship incrementally)

> Spec-first per decision 2026-06-21; this is the implementation order once we proceed.

- **Stage 0 — Safety net (no behavior change).** **Revised by §10.4 (C1/M2):** the
  keystone is a **host-reducer integration test** (§7.2), not the e2e "tree exits" test
  (§7.1, which is infeasible as a CI gate — there is no CI test runner and CI cannot
  launch CEF). Stage 0 = (a) **add a CI job that actually runs `cargo test` + `vitest`**
  (none exists today — `.github/workflows/` has zero test runners, so "stays green" is
  currently meaningless); (b) the reducer-gate tests that reproduce **both** races —
  last close with a `TabPool` promotion in flight, **and** last close racing an in-flight
  *user-window* creation (the §10.2 corner) — red→green against the minimal targeted fix.
  The PID-poll e2e is a **local-only, non-CI smoke**, not the gate.
- **Stage 1 — Typed roles (LC-3).** Introduce `WindowRole`; migrate the lifecycle-path
  predicates off label strings. Pure refactor; covered by Stage 0 tests.
- **Stage 2 — Level-triggered reconcile (LC-1/LC-2/LC-4).** Add `reconcile_quit`; call
  it after every relevant transition; reduce `on_before_close` to transition-reporting;
  delete `orphan_reconcile`'s drain branch.
- **Stage 3 — One exit path (LC-5).** Collapse the async bridges; delete the dead ones;
  re-confirm against `b9-3-quit-thread-analysis.md`.
- **Stage 4 — Sidecar cascade (LC-7).** Land `window-close-process-cleanup.md`.
- **Stage 5 — De-fossilize.** Strip the obsolete phase-marker commentary now that the
  invariants are the documentation; demote `window_cleanup` saga to observational.

Stages 0–2 fix the reported regression. 3–5 are the durability/cleanup payoff.

---

## 7. Test plan (the missing coverage is the point)

- **7.1 End-to-end "tree exits" — LOCAL-ONLY smoke, NOT a CI gate (§10.4-C1).** Launch a
  host+launcher (dev layout), open one window, close it, assert the **process tree is
  gone** within a deadline (poll J0 / PIDs). Implement as a `tools/tests/*.ps1|.mjs`
  smoke (the existing `task dev`-only harness family), explicitly documented as flaky
  (warm-pool refill timing, dynamic CDP port, AV-scan races) and **never a merge gate**.
  CI cannot run this: no test runner exists and GitHub-hosted CI cannot launch CEF.
- **7.2 Reconcile-gate integration tests (host reducer) — THE keystone gate.**
  - last `UserTopLevel` deregister with **no** creation/promotion in flight ⇒ `BeginDrain`.
  - last close with a `TabPool` promotion **in flight** ⇒ no premature quit, then
    `BeginDrain` on the next reconcile tick (the reported regression, pinned).
  - **last close racing an in-flight *user-window* creation (`window-<uuid>` in
    `pending_window_creations`, not yet registered) ⇒ host stays `Running`; quits only
    after that creation aborts or completes-then-closes (§10.2-C1 — currently untested
    and the most dangerous corner: a vacuous gate here = "quit while the user's new
    window is loading").**
  - `Draining` + last pool/browser gone ⇒ `ConfirmDrained` exactly once.
  - refill request while `Draining` ⇒ suppressed (already covered; keep).
- **7.3 Role taxonomy.** `BrowserPaneChild` / `TabPool` / `PanePool` never count toward
  `live_user_windows`; `UserTopLevel` always does; tear-off→`UserTopLevel`,
  redock→child transitions update the count correctly.
- **7.4 Sidecar cascade.** After window close, the workspace's shell controllers are
  killed before DB rows are deleted; no orphaned `claude.exe`/shell PIDs.

---

## 8. Risks & non-goals

- **Risk: refactoring shutdown is high-blast-radius.** Mitigation: Stage 0 lands the
  e2e + unit safety net *first*; every later stage is gated on it staying green. No
  big-bang.
- **Risk: touching launcher job/pipe code violates I1–I6.** Mitigation: we explicitly
  **do not** touch the Job Object / pipe / single-instance topology (LC-6); the fix is
  host-side. Any incidental launcher change is reviewed against the isolation spec.
- **Risk: macOS/Linux parity.** The reconcile logic is platform-agnostic (operates on
  the registry); only the Stage-3 exit primitive is per-platform. Keep the existing
  platform branches there, delete only the redundant ones.
- **Non-goal:** redesigning the warm-pool itself, the saga coordinator's durability, or
  cross-window docking. This spec only makes "should this instance be alive?" a single,
  level-triggered, typed, tested decision — and finishes the sidecar cleanup.

---

## 9. Open questions

1. Should `reconcile_quit` live in the existing host reducer (`reducer/quit.rs`) or a
   new `reducer/lifecycle.rs` that owns the window-registry + role + quit transitions
   together? (Leaning: new module, so the taxonomy and the decision are co-located.)
2. Is there any legitimate case where the host should stay alive with **zero**
   `UserTopLevel` windows (e.g. a background-agent-only mode)? If so, `reconcile_quit`
   needs an explicit "keep-alive" reason rather than a hard zero-gate. (Believed: no
   today — verify before deleting the deferral paths.)
3. Does any consumer actually depend on the launcher→host `DrainPoolIfLast` command, or
   is it fully redundant once the host self-reconciles? (Determines whether Stage 5 can
   delete it outright.) **§10.3 leans "redundant" — `HostShouldQuit` is already documented
   advisory — but confirm before deleting.**

---

## 10. Adversarial review findings (2026-06-21)

Four independent red-team passes (CEF threading, race semantics, regression archaeology,
test/sidecar feasibility), each grounded in the real code. Findings that survived
scrutiny are recorded here with evidence and the resulting amendment. **These supersede
the first-draft prose in §3–§7 where they conflict.** Severity: 🔴 Critical / 🟠 High /
🟡 Medium.

### 10.1 Threading — the level-trigger must not own the exit primitive

- 🔴 **The reducer runs on the dispatching thread, not the UI thread.** `host_dispatch`
  locks `host_state` and calls `update()` inline (`state.rs:901-904`). Promote/ready
  transitions are dispatched from **tokio workers** (`ipc.rs:108`; `window_pool.rs:607`
  `PoolWindowReady`, `:669` `PopAndPromoteFrontPoolWindow`). A `reconcile_quit` that
  calls `quit_message_loop()` when it reaches `Quit` would fire it **off the UI thread** →
  **silent no-op** (CEF requires UI-thread; the v0.33.492 failure,
  `b9-3-quit-thread-analysis.md`). This is the *same* bug the two-stage design exists to
  avoid, relocated into the reducer.
- 🔴 **Non-reentrant lock.** `parking_lot::Mutex` held across `update()` (`state.rs:901`)
  — `reconcile_quit` must not call anything that re-locks (`is_quitting`,
  `user_visibility_snapshot`, `spawn_pool_window`, `host_dispatch`), or it self-deadlocks
  the UI thread (v0.33.498 deadlock: `client/mod.rs:1059-1062`).
- 🟠 **Stage 1 is *work*, not a decision.** Closing already-spawned pool browsers via
  UI-thread `PostMessage(WM_CLOSE)` (`client/mod.rs:1101-1156`) must be **kept**;
  `BeginDrain` only suppresses *future* refill (`window_pool.rs:244`). The "`on_before_close`
  shrinks to deregister→reconcile→quit" sketch erased this actor.
- **Amendment:** the §5.1 threading contract (now inlined there). `reconcile_quit` =
  pure decision; the exit primitive stays at the UI-thread Stage-2 site reading
  `quit_state == Quit`; off-UI transitions deliver loop-exit via `PostThreadMessage(WM_QUIT)`.

### 10.2 Race semantics — "creation in flight" is a pre-registration concept

- 🔴 **A role cannot answer "is a user creation in flight" — the in-flight window isn't
  registered, so it has no role.** Real user creates enqueue `PendingWindowCreation
  { label, kind, parent_instance_id }` (**no source field** — `state.rs:104-108`) into
  `pending_window_creations` *before* the browser registers (`creation.rs:364`). The H.6
  `top_level_creation` runner that *looks* like the right source is **dead in production**
  (only tests dispatch `EnqueueTopLevelWindow`) and drops `source` into `InFlightCreation`
  anyway (`top_level.rs:71-76`). So the gate must read `pending_window_creations`, which
  is exactly what `orphan_reconcile` does (`orphan_reconcile.rs:287-305`) — the thing §5.1
  proposed to delete.
- 🔴 **The headline corner is currently ungated:** "New Window" clicked + last existing
  window closed in the same instant ⇒ if the gate is vacuous, `reconcile_quit` fires
  `BeginDrain`, suppresses the in-flight window's completion, and **quits out from under a
  window the user asked for.** Pinned by existing `plan_pending_window_creation_blocks_drain`
  (`tests.rs:940`); must have a Stage-0 reducer test (added to §7.2).
- 🟢 **The reported race (close vs pool promote) genuinely self-heals** under level-trigger:
  `PopAndPromoteFrontPoolWindow` is atomic under one lock (`pool.rs:97-121`) and the
  promoted label is already in `browsers`, so it's always on the user side of the count —
  no "in neither set" tick. A later deregister re-ticks the decision. This is the clean win.
- **Amendment:** add `source`/`role` to **`PendingWindowCreation`** (set by each caller:
  user paths → user; pool/pane/floating → background); gate `reconcile_quit` on "no
  *user* entry in `pending_window_creations`." Carry the guard onto reconcile's **actions**
  (esp. zombie reap — `orphan_reconcile.rs:165`), not just the drain decision. Add
  `cleanup_failed_promote_orphan` (`window_pool.rs:1108`) and pool-spawn-abort to the
  reconcile trigger set.

### 10.3 Regression archaeology — this partly re-litigates settled history

- 🔴 **Launcher-side single authority was already tried and demoted.** Phase B.9.3 added
  a launcher reducer emitting `Event::HostShouldQuit` from `state.windows.is_empty()` —
  exactly "single authority, re-evaluated on state change." Smoke v0.33.491/492/493/494
  all failed to *deliver* the quit to the host UI thread (`post_task` never drained;
  `quit_message_loop` off-thread no-op; `PostThreadMessage` ignored). Conclusion: keep the
  **host-local** gate; `HostShouldQuit` is **documented ADVISORY** (`ipc.rs:1057-1069`).
  → "Single quit authority" (LC-1) must mean **host-side**, and Q3's `DrainPoolIfLast`/
  launcher path is very likely redundant. The spec should *cite* this, not present it as new.
- 🟠 **Two taxonomies already exist — extend, don't invent** (folded into §5.2):
  `BrowserKind` (`state.rs:240`) + `WindowKind` (`state.rs:70`, `ipc.rs:780`).
- 🟠 **Constraints any refactor MUST preserve:** (1) `quit_message_loop` UI-thread-only,
  Stage-2 deferral kept (§10.1); (2) `user_visibility_snapshot` stays a single atomic
  snapshot, and the exit-gate vs launcher-drift filters are *deliberately different*
  (`state.rs:1076-1078`) — don't collapse; (3) `QuitState` monotonic, no `Draining→Running`
  (`quit.rs:14`) — so an over-eager `BeginDrain` on a transient zero-count permanently
  freezes a live session → keep the in-flight/`freshly_promoted` guards; (4) role assigned
  from **label**, not the `is_browser_pane` client flag (v0.33.586); (5) macOS fast-exit
  (`runtime.shutdown_background()` + `process::exit(0)`, `SPEC_MACOS_WINDOW_CLOSE_LIFECYCLE_2026-06-04`)
  must not be reverted when collapsing exit bridges (LC-5).
- 🟡 **Re-framing:** much of what §2 calls "38 patches keeping each other honest" is 38
  *named regression fixes*, each preventing a reproduced failure. The consolidation should
  *absorb* those constraints as invariants, not treat the design as undesigned layering.

### 10.4 Test feasibility & sidecar — the safety-net premise needed rework

- 🔴 **C1: no CI test gate exists.** `.github/workflows/` (5 files) runs **zero**
  `cargo test`/`vitest`/`task test`. The only CDP harness (`input-bench-report.yml`) is
  `workflow_dispatch`-only and needs a self-hosted runner that **does not exist**.
  CI cannot launch CEF (no Chromium/display/binaries); the CDP port is dynamic
  (`lib.rs:719`). → the Stage-0 "tree exits" e2e cannot be the gate (revised §6, §7.1).
- 🟠 **M2: the feasible keystone** is a host-reducer integration test over a fake registry
  (pure `reconcile_quit`, zero CEF/display) — promoted to §7.2. Plus a prerequisite:
  **add a CI job that runs `cargo test` + `vitest`** so anything can "stay green."
- 🟠 **H1/H2/D: §3.4/§5.4 were stale** — the sidecar cascade is already implemented
  (`wcore/tab.rs:99-116`), and the last-window path relies on J0, not the cascade. Stage 4
  re-scoped to audit (revised §3.4, §5.4).
- 🟠 **H3: the real cleanup risk is workspace-sharing, not global agents.** Controllers are
  per-`block_id` (`blockcontroller/mod.rs:202`); the guard to preserve is "delete workspace
  only if no other live window points at it" (`service.rs:1010-1025`). Added to §5.4 + §7.4.
- 🟡 **M1: launcher teardown is two real impls** (`run_windows` Job Object vs `run_unix`
  SIGTERM/SIGKILL/`PR_SET_PDEATHSIG` — `main.rs:305/401/445`). §8's "one per-platform
  primitive" understates this; any quit-path change is reviewed against both.

### 10.5 Net effect on the plan

The **structural thesis holds**: one host-side, level-triggered, typed, tested quit
decision is still the right target, and the close-vs-promote race (the clean win, §10.2)
genuinely dissolves under it. What changed:

1. `reconcile_quit` **decides only**; the UI-thread exit primitive and Stage-1 drain
   executor stay put (§10.1).
2. The in-flight gate reads the **pre-registration `pending_window_creations` queue**
   (with a new source field), not a role; its guards are **preserved**, not deleted (§10.2).
3. **Extend `BrowserKind`**, don't add `WindowRole`; cite the B.9.3 host-side demotion
   (§10.3).
4. Stage 0's gate is the **host-reducer integration test** (+ a first-ever CI test job);
   the e2e is a non-gating local smoke (§10.4).
5. Sidecar work is **audit + workspace guard**, not a reimplementation; it's orthogonal to
   the headline orphan (§10.4).
6. **Verify the root cause** against the two same-day promotion-wedge retros before
   touching the gate (§1.2 caveat).
