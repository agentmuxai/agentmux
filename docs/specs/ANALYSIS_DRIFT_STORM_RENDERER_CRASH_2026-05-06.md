# Drift-storm renderer crash (post-PR #706 smoke) — architectural analysis

**Date:** 2026-05-06
**Trigger:** v0.33.655 portable smoke. After 1 tear-off, renderer crashed with
`Crashpad_NotConnectedToHandler` ~2.5 s after the promote.
**Symptom:** 603 `hwnd_drift_detected` log lines in <3 s, **all carrying the same
launcher-side `version: 13`** — i.e. one launcher event re-dispatched ~600×.
**Reads against:** [`MASTER_REDUCER_STACK_STATUS_2026-05-05.md`](./MASTER_REDUCER_STACK_STATUS_2026-05-05.md).

---

## 1. What happened (forensics)

```
12:39:24.212  workspace.TearOffTab            ┐
12:39:24.212  [pool] promoting pool window    ├ promote of window-pool-7fa5...
12:39:24.221  [pool] spawning pool window     │  refill: window-pool-cb01...
12:39:24.227  complete_cross_drag (tearoff)   ┘
12:39:24.470  …storm of 603 [fe] [launcher-event] drift entries…
12:39:27.109  ERROR renderer process crashed  Crashpad_NotConnectedToHandler
```

Every entry in the storm:

```json
{"event":"hwnd_drift_detected",
 "kind":"hidden_since_open",
 "label":"window-pool-7fa5f5a3fac34c908756f640c258257a",
 "hwnd":10945738,
 "detail":"Window hidden without ever being foregrounded since open",
 "severity":"warn",
 "version":13}
```

Fixed `version: 13` ⇒ **one** launcher event, amplified somewhere in
launcher → host → renderer.

---

## 2. The two real bugs

### Bug A — launcher reducer correctness: WRR drift state isn't reset on promote

`apply_hwnd_visibility_changed` in `agentmux-launcher/src/wrr/mod.rs:341` emits
`HiddenSinceOpen` whenever a window goes invisible and `foregrounded_since_open`
is still false. `foregrounded_since_open` is only flipped true by
`Command::ReportHwndForegroundChanged`.

A pool window:

1. Is created hidden (off-screen, `WS_EX_TOOLWINDOW`).
2. Sits in `pool.unpromoted` → moves to `pool.queue` after renderer-ready.
3. **Promote**: host moves the HWND on-screen, frontend bootstraps the workspace.

Between steps 2 and 3 the window has never been foregrounded, so
`foregrounded_since_open=false`. Promote moves the HWND but doesn't
synthesise a foreground transition into the launcher reducer. Any
visibility flicker during repositioning (and there are several:
`SetWindowPos`, monitor probing, taskbar reveal, focus-reclaim) re-fires
`HiddenSinceOpen` against an already-promoted user window.

**This is a Layer 1 (Launcher) reducer bug.** Fix is a state transition that
the existing reducer lacks: on `PoolWindowPromoted`, clear the
"open-transient" guards (`foregrounded_since_open` should arguably be
re-armed *to true* — the user explicitly created this window by tearing
off, so the open-transient corrective logic shouldn't apply).

### Bug B — launcher → host → renderer event flood without backpressure / de-dup

Same launcher event (`version: 13`) reaching the renderer ~600×. Three
candidate amplification points; the actual one needs a small
investigation, but each is a separate architectural gap:

1. **Launcher broadcast bus subscriber leak** — every host subscription
   that's added but not properly unsubscribed will receive the event
   N times.
2. **Host JS bridge has no de-dup** — `launcher_event_bridge::dispatch_to_renderers`
   is a thin pass-through; if `apply_event_to_shadow` is invoked
   re-entrantly for the same event, every reentry fans out again.
3. **Pool window pipe / subscription duplication** — pool windows
   register subscribers during their bootstrap; if pool-window
   teardown/promote doesn't clean those up, each retained subscriber
   re-delivers.

**Whatever the amplification, the architectural bug is the same: there is
no flow control on the launcher → host → renderer event path.** PR #706
made promoted pool windows visible to launcher events for the first
time, which exposed this.

---

## 3. How this fits the master spec

| Master spec section | This bug shows |
|---|---|
| §3 Layer 1 Launcher — *single-writer, canonical for windows / WRR / pool* | WRR's `foregrounded_since_open` state machine is incomplete. Promote moves the HWND on-screen but no command updates the WRR mirror. **Missing arm in `apply_*` chain.** |
| §4 Layer 2 Host — *frontend ↔ launcher via host JS bridge (decision §8.9)* | The bridge in `agentmux-cef/src/launcher_event_bridge.rs` is a thin pass-through. PR #706 corrected its filter; it still has no de-dup, no rate limit, no batching. **Architectural gap: backpressure.** |
| §6 Frontend — *slice #6 launcher-event-reducer convergence (designed, not started)* | The renderer-side reducer exists but only owns the InstancePanel view. There's no idempotency check by `(label, version)` in the dispatcher. **A reducer with `seen_versions: Set<u64>` would absorb this flood with O(1) cost.** |
| §5.4 Sagas — *PoolRespawnSaga ✅; WindowCleanupCascade pending; **no PoolPromote saga*** | Promote today is a sequence of side-effects, not a saga: `report_pool_window_removed` → `report_pool_window_promoted` → `report_window_opened` + host UI work + renderer bootstrap. No saga coordinates the WRR-state-update step. **Missing saga arm.** |
| §8.4 *Versioned events + snapshot/replay; subscribers detect gaps and resync* | Versioning exists. Subscriber-side **idempotency** does not. The contract says "detect gaps", not "detect duplicates". |
| §8.7 *WRR via Win32 hooks, no timers* | True for input. But the post-promote OS event sequence (multiple `SetWindowPos` from re-parenting) re-fires WRR detection. Not a timer; same effect. |
| §9.1 Cross-process saga dispatch (BLOCKER for F.5/F.6) | A "host requests launcher to update WRR" call has no transport. Today the host can only emit forward-going events; no command path **back** to the launcher. **Bug A's fix needs cross-process dispatch — or the launcher must self-detect promote from the existing event stream and update its own WRR state.** |
| §9.2 Per-event saga_id correlation (BLOCKER for E.6) | If the drift event were tagged with a saga_id, the renderer reducer could de-dup-by-saga. Today there's no correlation key; the renderer can't distinguish "same event re-broadcast" from "new event with same fields". |

---

## 4. Other gaps the storm reveals (beyond the master's §9 list)

### 4.1 No idempotency contract on the launcher event channel

The master spec §8 lists 13 architectural decisions. **None of them
spell out idempotency.** Subscribers MUST be idempotent for resync /
replay (§8.4) to work — but this isn't documented and isn't enforced.
Renderer-side handlers like `tracker.deliver(evt)` deliver every
arrival; if the host fans an event 600×, the renderer processes it
600×.

**Proposal:** add §8.14: *"Launcher event subscribers MUST be idempotent
under (event_kind, label, version) — duplicates may legally arrive from
re-dispatch, resync, or replay; subscribers de-dup by (kind, label,
version) before applying."* Then bake it into `frontend/util/launcher-events.ts`
and the host bridge.

### 4.2 No "state transitions on promote" spec for WRR

The pool-window state machine (`pool.unpromoted` → `pool.queue` →
promoted) is documented in `dnd:tearoff:pool` comments and recently in
PR #706. But the **WRR companion state** (`foregrounded_since_open`,
`off_monitor`, `last_rect`) doesn't have a parallel transition map.
Promote needs to be a recognised event in BOTH state machines.

**Proposal:** new spec section in the host reducer master plan
covering "WRR ↔ pool state transition matrix" — single table per
event kind × WRR field × required transition. Today this is
implicit and easy to miss (PR #706 didn't catch it).

### 4.3 Renderer overload as an unhandled failure mode

Crash kind: `Crashpad_NotConnectedToHandler` — a CEF generic crash with
no symbol dump. Likely the V8 isolate ran out of stack/heap or the
`fe_log_structured` IPC channel saturated and the renderer thread was
starved.

**No spec covers what the host should do when a renderer crashes
during normal event flow.** F1.B (frontend orphan-cleanup on hard host
failure) covers the *host* dying; the inverse (renderer dying while
host is healthy) only triggers an auto-reload with no diagnostic
trail beyond the Crashpad message.

**Proposal:** explicit failure mode in the host reducer spec — *"on
RendererCrashed, snapshot the recent event ring (last 100), the
launcher-event subscribe count, and the host bridge dispatch count to
a crash-context file."* Today we lose all that context.

### 4.4 Phase F.7 missing from the master roadmap

§9.1–9.7 enumerate open questions; §9.1 names cross-process dispatch
as a F.5/F.6 blocker. **There's no F.7 "host event throttling /
batching / de-dup".** That gap is what made this bug crashable.

**Proposal:** add Phase F.7 — *"host JS bridge resilience: per-renderer
backpressure, version-keyed de-dup, batched dispatch under load."* List
as deferred but tracked.

### 4.5 PR #706 retro: prefix-filtering masked Bug A for months

Pre-PR-706, the bridge excluded all `window-pool-*` labels from
launcher-event dispatch. That filter was *also a backpressure valve* —
promoted pool windows were silently insulated from the WRR drift
re-emission storm. The "InstancePanel drift" PR #706 fixed was the
*intended* behaviour the filter blocked, but removing the filter
exposed the drift storm at full volume.

**Lesson for the master spec:** when the architecture says *"all
top-level browsers receive launcher events"*, that's a load-bearing
claim about volume that should be tested. Adding §8.14 (idempotency)
+ a back-pressure decision would have prevented PR #706's smoke
crash.

---

## 5. Recommended fix order

Prioritised by smallest-blast-radius first.

1. **Frontend de-dup** *(slice #6 starter, 1 PR)*: in the renderer's
   `tracker.deliver()`, drop events whose `(kind, label, version)` has
   already been delivered. Caps memory at O(seen events). Prevents the
   crash regardless of upstream amplification.

2. **Launcher reducer fix for `foregrounded_since_open` on promote**
   *(1 PR, small)*: handler arm — when `PoolWindowPromoted` is applied,
   also set the corresponding window mirror's `foregrounded_since_open`
   to true (or invalidate the open-transient guard). This is a Layer 1
   correctness fix. Stops the storm at source.

3. **Diagnose the actual amplification** *(investigation, 1 day)*:
   instrument `dispatch_to_renderers` with a counter; verify whether
   the host is fan-outing 600× or the launcher is broadcasting 600×.
   Drives whether (4) is needed.

4. **Host bridge de-dup + rate limit** *(Phase F.7 starter, 1 PR)*:
   per-event version cache in the bridge — drop redundant
   `Frame::ExecuteJavaScript` for a `(kind, label, version)` already
   dispatched within window N. Defends against subscriber leaks
   regardless of which side leaks.

5. **Architecture decision §8.14 added to master**: idempotency
   contract for launcher event subscribers. Doc-only — but it's the
   load-bearing decision the storm proved is missing.

6. **Master §9.8 added: Phase F.7 — host bridge resilience**: tracked
   but deferred. Pairs with §9.1 (cross-process dispatch) — both are
   about the host's role as a relay.

---

## 6. Why this is worth reviewing as a spec

The PR #706 fix was correct in isolation but **destabilised in
integration** because the architecture didn't have a flow-control
decision at the launcher → renderer boundary. The master spec is
already excellent at *layer ownership*; this analysis recommends
extending it with **inter-layer contract** decisions — idempotency,
backpressure, transition matrices — that are currently informal and
easy to miss when adding new event flows.

Concrete additions proposed:
- **§8.14** idempotency contract (renderer + host subscriber requirement)
- **§9.8** Phase F.7 host bridge resilience (open question)
- **§4.5** WRR ↔ pool state transition matrix (host/launcher reducer joint state)
- **§5.5** Promote saga (or document why promote isn't a saga and how WRR state stays consistent without one)

Updates proposed:
- **§9.1** note that cross-process dispatch is also needed for the
  inverse direction (host reporting to launcher), and that without it
  the launcher reducer has to self-detect promote-completed events.
