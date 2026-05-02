# Smoke Test Report — 0.33.586 (H.2 browsers ratchet complete)

**Date:** 2026-05-02
**Build:** `agentmux-0.33.586-x64-portable` (main, post-PR-#660 merge)
**Tester:** user
**Author:** AgentA

---

## TL;DR

Smoke test on 0.33.586 found:

1. **The original 2026-05-02 freeze recurred** — windows registered in CEF but never reached foregrounded state. `WRR-DRIFT [Warn] HiddenSinceOpen` + `pending=25` IPC backpressure climbing. Same fingerprint documented in `SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md`. **NOT a regression from PR #660** — this is the pre-existing CEF v146 wedge that **PR #6 (H.6 top-level runner + H.7 cross-state invariants) is designed to fix**. PR #5 is a prerequisite.

2. **A real bug introduced by PR #655 H.2.a (browser parallel writes)** — top-level windows getting misclassified as `BrowserKind::Pane { block_id: "" }` because `BrowserKind` was determined from `client.is_pane` (a flag on the shared `AgentMuxClient`) instead of from the LABEL prefix. **Will be fixed in PR #5** as a prerequisite cleanup.

The freeze is not gated on the misclassification — `BrowserKind` is currently read only by `PromotePoolWindow` to clear `is_pool`. Nothing else acts on it. So fixing the misclassification doesn't fix the freeze. **The freeze fix needs PR #6's cross-state invariant.**

---

## What the user did

1. Launched 0.33.586 portable
2. Created browser panes (~7 panes opened, then closed)
3. Opened multiple top-level windows in succession
4. First windows opened normally; later windows registered but did not appear visually
5. Reported "new windows no opening" — confirmed not a crash, just a no-op (later clarified)

---

## What the logs show

### Reducer activity (host log, target=`host-reducer`)

19 `BrowserRegistered` events total:
- 1× `main` — `TopLevel { is_pool: false }` ✓
- 3× `window-pool-*` — `TopLevel { is_pool: true }` ✓
- 7× `browser-pane-*-N` — `Pane { block_id: <uuid> }` ✓
- 4× `window-<uuid>` (early) — `TopLevel { is_pool: false }` ✓ (correctly classified)
- 4× `window-<uuid>` (late) — `Pane { block_id: "" }` ✗ (misclassified — bug #2)

Balance: 19 register / N unregister. No error events. No drift warnings (drift logging was retired in PR #660 H.2.c flip).

### Launcher WRR (target=`wrr`, level=`Warn`)

```
[1777730529] v0.33.586 [ipc] WRR-DRIFT [Warn]
    HiddenSinceOpen label=Some("window-fc907ae25a634b3c9779b7b368984eef")
    hwnd=Some(4129996): Window hidden without ever being foregrounded since open
    (conn_id=1)
[1777730529] v0.33.586 [ipc] WRR-POS hwnd=... pending=25
[1777730529] v0.33.586 [ipc] WRR-POS hwnd=... pending=26
```

**Same exact signature** as the freeze documented in `SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md`:
- HiddenSinceOpen drift on a created top-level window
- `pending` counter on WRR-POS climbing without draining
- Indicates the host's IPC outbound queue is backing up because the UI thread is stuck

User-perceived: window create RPC fires, CEF creates the browser, but the window never visibly appears. Sometimes the whole host hangs (per past investigations); this time it appeared the UI was still partly responsive but new-window operations no-op'd.

### Process state at smoke time

All processes still `Responding=True` per Powershell tasklist — no crash, no hard hang of host. The state is consistent with "CEF internal pipeline wedge that doesn't fully kill the UI thread but blocks new window creates."

---

## Bug #1: The freeze (pre-existing, NOT introduced by PR #660)

### Diagnosis (already documented)

`SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md` traced this freeze to:
- Two AboveNormal CEF threads in `EventPairLow + Unknown` lock-wait
- Inside CEF's async browser-creation pipeline, after `window_create_top_level` returns
- **User-confirmed:** does NOT reproduce without browser panes — pane HWND tree dispatching nested-pump messages during top-level creation triggers a cross-process LPC deadlock with the pane's renderer process

### Fix path

`SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md` plan §"Phase H.7" specifies a **cross-state invariant** to be added in PR #6:

```rust
HostCommand::EnqueueTopLevelWindow { request, source: TopLevelSource::User } => {
    let any_pane_in_transition = state.panes.values()
        .any(|p| matches!(p.lifecycle, PaneLifecycle::Closing { .. }));
    if any_pane_in_transition {
        return reject_error("pane mid-transition; retry shortly");
    }
    // ... proceed
}
```

**This is a probe.** The hypothesis is that the deadlock triggers specifically when top-level creation overlaps a pane in `Closing`. If true, refusing the operation breaks the trigger condition. If not, the invariant gets relaxed to "any pane present" (more aggressive — would block the use case but at least confirms scope).

### Why PR #5 is a prerequisite, not the fix

PR #5's scope:
- **H.1.d** — drop `PaneStateMachine` writes (pane lifecycle goes through reducer only)
- **H.1.e** — delete `PaneStateMachine` struct
- **H.3** — drag state → reducer
- **H.4** — pool state → reducer
- **H.5** — quit lifecycle → reducer

After PR #5:
- Pane lifecycle is fully reducer-mediated (no parallel write to legacy)
- `state.panes` HashMap in the reducer is the sole source of truth
- The cross-state invariant in PR #6 can read `state.panes.values()` and be sure it sees the authoritative pane state

**Without PR #5**, the cross-state invariant in PR #6 would be reading from a parallel-mirrored state (same data, but two writers — fragile). With PR #5, there's one source of truth.

### Why PR #6 is the actual fix

PR #6's scope:
- **H.6** — event-driven top-level window creation runner (no watchdog, no timer per user directive)
  - All top-level creates flow through `EnqueueTopLevelWindow`
  - Reducer manages an in-flight slot, queues subsequent requests
  - Reacts only to observable CEF callbacks: `on_after_created`, `on_render_process_terminated`, `on_before_close`
- **H.7** — cross-state sagas + invariants
  - `NewWindowSaga` supervises each top-level creation
  - **The freeze invariant**: `EnqueueTopLevelWindow` rejects when any pane is `Closing`
  - User-initiated creates fail-fast with a visible error message; pool refill (background) queues

The cross-state invariant **directly intercepts the trigger condition** — it refuses to issue `post_create_window` while a pane is mid-transition, which is the scenario the original investigation traced to the CEF deadlock.

### What if the H.7 invariant doesn't fix the freeze?

Per the spec's escape hatch:
- Relax to "any pane present (Live or Closing)" — refuses ALL window opens while a pane is alive. Diagnostic — narrows the trigger condition.
- If even that doesn't help, the deadlock surface is wider than panes, and we'd need different mitigations (e.g., serializing all CEF browser creates regardless of state — basically an unconditional global mutex around `post_create_window`).

The probe ships in PR #6. If smoke confirms it works, we keep it. If not, we widen.

---

## Bug #2: Top-level windows misclassified as `Pane`

### Symptom

```
"event":"BrowserRegistered","label":"window-209770e1e46d40a488ea936657a944de",
    "kind":"Pane { block_id: \"\" }"
```

A `window-*` label classified as `Pane` with empty `block_id`. Should be `TopLevel { is_pool: false }`.

### Root cause

`agentmux-cef/src/client.rs::on_after_created` lines 214-228 (added in PR #655 H.2.a):

```rust
let kind = if self.is_pane {
    let block_id = label
        .strip_prefix("browser-pane-")
        .and_then(|rest| rest.rfind('-').map(|i| rest[..i].to_string()))
        .unwrap_or_default();
    crate::state::BrowserKind::Pane { block_id }
} else if label.starts_with("window-pool-")
    && self.state.unpromoted_pool_labels.lock().contains(&label)
{
    crate::state::BrowserKind::TopLevel { is_pool: true }
} else {
    crate::state::BrowserKind::TopLevel { is_pool: false }
};
```

The first branch checks `self.is_pane` (a flag on `AgentMuxClient`). For client-shared top-level windows whose client was inherited from a pane (via `first_browser()` lookup in `CreateWindowTask::execute`), `is_pane=true` even though the label is `window-*`.

When `is_pane=true` AND label starts with `window-`:
- `label.strip_prefix("browser-pane-")` returns `None`
- `unwrap_or_default()` makes block_id = `""`
- Kind becomes `Pane { block_id: "" }`

### When does this happen?

`CreateWindowTask::execute` reuses an existing browser's CEF Client:
```rust
let client = self.state.first_browser().and_then(|(_, b)| b.host().map(|h| h.client()));
```

`first_browser()` is non-deterministic (HashMap iteration order). If the first browser in the iteration happens to be a pane, the new window inherits that pane's client → inherits `is_pane=true`.

Note: the existing `is_pane` flag was always wrong in this case — even pre-migration, pane-callbacks would run for top-level windows in this scenario (line 268 in `client.rs`: `if self.is_pane { on_after_created_pane(...) }`). It's a latent bug going back to before Phase H. PR #655 H.2.a just made it more visible by routing the (wrong) flag into the BrowserKind metadata.

### Fix

Classify by LABEL prefix, not by `is_pane`:

```rust
let kind = if let Some(rest) = label.strip_prefix("browser-pane-") {
    let block_id = rest.rfind('-').map(|i| rest[..i].to_string()).unwrap_or_default();
    crate::state::BrowserKind::Pane { block_id }
} else if label.starts_with("window-pool-")
    && self.state.unpromoted_pool_labels.lock().contains(&label)
{
    crate::state::BrowserKind::TopLevel { is_pool: true }
} else {
    crate::state::BrowserKind::TopLevel { is_pool: false }
};
```

Label is the source of truth — `browser-pane-*` is a pane, anything else is top-level.

### Why this doesn't fix the freeze

`BrowserKind` is currently read only at `agentmux-cef/src/reducer.rs:979` (`PromotePoolWindow` handler — sets `is_pool: false` on promotion). Nothing else reads it. The freeze symptom is unrelated to BrowserKind metadata. Fixing the misclassification produces correct metadata but doesn't change runtime behavior in any code path that today affects window visibility.

The is_pane bug for non-BrowserKind paths (line 268 `if self.is_pane { on_after_created_pane(...) }`) is a SEPARATE pre-existing issue that this fix doesn't address. That can be tackled later if it manifests as a real symptom.

---

## PR #5 plan

Includes:

1. **Misclassification fix** (small, safe — doesn't affect freeze)
2. **H.1.d** — drop `PaneStateMachine` writes (refactor `BrowserPaneManager::create/close/drain` to query reducer instead of mutating `PaneStateMachine`)
3. **H.1.e** — delete `PaneStateMachine` struct
4. **H.3** — `active_drag` → reducer (drop `AppState.active_drag` field)
5. **H.4** — pool state → reducer (drop `AppState.window_pool` + `unpromoted_pool_labels` + `window_pool_respawn_in_flight`)
6. **H.5** — quit lifecycle → reducer (drop `AppState.is_quitting`)

After PR #5: pane lifecycle, drag, pool, and quit state are all in the host reducer. `state.panes` is the authoritative source of truth, ready for PR #6's cross-state invariant.

## PR #6 plan (the actual freeze fix)

1. **H.6** — top-level window creation runner
   - New `HostCommand::EnqueueTopLevelWindow { request, source }`
   - Reducer manages in-flight slot + queue
   - Effect handler dispatches `post_create_window` for each request
   - Subscribes to `on_after_created`, `on_render_process_terminated`, `on_before_close` for completion signals
   - **NO watchdog, NO timer** — event-driven only
   - User-initiated creates fail-fast on contention; background (pool refill) queues
2. **H.7** — cross-state invariant + sagas
   - **The freeze fix:** `EnqueueTopLevelWindow` rejects when any pane is `PaneLifecycle::Closing`
   - `NewWindowSaga` supervises creation lifecycle
   - `PaneCreateSaga` symmetric for pane creation

After PR #6: opening a top-level window while a pane is mid-close gets a visible error toast instead of a silent freeze. If pane state is the trigger, the deadlock disappears. If not, the H.7 escape hatch widens the invariant to all pane states for further diagnosis.

---

## What about PR #7?

H.8 (durability — SQLite log of host reducer state) + H.9 (wire-promote selected events for cross-process saga subscribers). Both are operator-visibility / observability work. Doesn't fix anything itself but makes future debugging much easier (think `agentmux --diag windows` showing the state of in-flight creations + pane lifecycle).

---

## Decision log

| Item | Decision |
|---|---|
| Roll BrowserKind misclassification fix into PR #5 | Yes — small, safe, prerequisite for clean PR #6 cross-state invariant readability |
| Document smoke results | This file |
| Skip PR #5? Jump straight to PR #6? | No. PR #5 makes pane state the sole source of truth; PR #6's invariant reads `state.panes.values()` and that needs to be authoritative. Skipping PR #5 means the invariant reads parallel-mirrored state — fragile. |
| Should we widen the invariant in PR #6 if the "Closing only" probe doesn't fix the freeze? | Yes per spec — relax to "any pane present." Diagnostic step. |
| Codex out of credits — implications? | Reagent only as bot reviewer. Codex caught subtle concurrency bugs reagent missed (atomicity in `take_browser_hwnd`, ordering in `list_browser_labels`). Mitigation: more thorough self-review on atomicity, idempotency, stale comments, ordering. |

---

End of report.
