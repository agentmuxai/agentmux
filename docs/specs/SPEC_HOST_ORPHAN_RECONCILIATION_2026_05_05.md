# Host Orphan-Instance Reconciliation — 2026-05-05

**Owner:** AgentA
**Status:** spec
**Layer:** 2 (host) — coordinates with Layer 1 (launcher) via existing event bus
**Master ref:** [`MASTER_REDUCER_STACK_STATUS_2026-05-05.md`](./MASTER_REDUCER_STACK_STATUS_2026-05-05.md), specifically §4.3 (host reducer scope), §4.4 (browsers/pool scaffolding), §9.1 (cross-process dispatch blocker).

---

## 1. Problem

Closing every user-visible window of a running v0.33.643 instance leaves the host process alive indefinitely, holding warm-pool browsers + the launcher's IPC connection. Six hours after the close in the user's session, `tasklist` still shows:

- `agentmux.exe` (launcher) PID 22600
- `agentmux-srv-0.33.643` PIDs 22744, 22588
- `agentmux-0.33.643.exe` (CEF host) PIDs 9044, 4088, 18236, 24844, 6200, 23176

The launcher log captures the cascade attempt at `17:16:35Z`:

```
[saga] starting saga_id=1 name=window_cleanup_cascade
[ipc] WRR-DRIFT [Warn] OrphanInstance label=Some("main") hwnd=None:
  Last user-visible window closed; host still alive (likely holding warm pool)
[ipc] DRIFT Windows: host=2 mirror=0
[ipc] DRIFT Pool: host=0 mirror=2
[saga] saga_id=1 name=window_cleanup_cascade IssueCmd::Host dispatched cmd=ReapPanes
[saga] saga_id=1 name=window_cleanup_cascade IssueCmd::Host dispatched cmd=DrainPoolIfLast
[saga] saga_id=1 name=window_cleanup_cascade Done — emitting SagaCompleted
```

Saga ran, dispatched correctly, completed — but the host never quit.

## 2. Root cause

Three pieces interact:

**a) The `window_cleanup_cascade` saga** (launcher) reacts to `Event::WindowClosed` by issuing `ReapPanes` + `DrainPoolIfLast`. The latter is a *query* — `agentmux-cef/src/saga_dispatch.rs::LiveActionRunner::drain_pool_if_last` returns a boolean, doesn't drain anything itself.

```rust
fn drain_pool_if_last(&self, label: &str) -> bool {
    let unpromoted = self.state.unpromoted_pool_labels_snapshot();
    let labels = self.state.list_browser_labels();
    let user_count = labels
        .iter()
        .filter(|k| !unpromoted.contains(k.as_str()) && !k.starts_with("browser-pane-"))
        .count();
    let label_present = labels.iter().any(|k| k == label);
    user_count == 0 || (user_count == 1 && label_present)
}
```

**b) The actual drain** lives in `agentmux-cef/src/client/mod.rs::on_before_close`, gated on the same `user_browser_count == 0`. When the user closes a window:
1. Compute `user_browser_count` from `list_browser_labels()` filtered to non-pool / non-browser-pane labels.
2. If zero, dispatch `BeginDrain { reason: LastWindowClosed }` and `PostMessageW(WM_CLOSE, ...)` to every `window-pool-*` browser.
3. Stage 2: when `browser_list` empties, call `quit_message_loop()`.

**c) The orphan condition.** The launcher's WRR mirror reports `Pool: host=0 mirror=2` — the host's reducer thinks the pool is empty (`state.pool.queue.len() == 0`), but the host's `browsers` map still owns 2 `window-pool-*` entries. These got promoted out of the pool by an earlier user action (open a tab → pull from pool → label gets dropped from `unpromoted_pool_labels` to indicate it's now a "real" window) but the corresponding window's close handler never fired (CEF crash, OS-level window kill, missed `on_before_close`, etc.). So they're "ghost user windows": still in `browsers`, no longer in `unpromoted_pool`, no live HWND.

Plug those into the saga's query:

```
labels = [<the 2 ghost window-pool-* labels>]
unpromoted = {}        # they're promoted
filtered  = [both]     # no browser-pane prefix
user_count = 2
was_last  = false
```

→ Saga emits `Event::PoolNotLast` → no cascade run → host stays alive.

The user-visible cascade in `on_before_close` has the same mistake on the same close: `user_browser_count == 2`, gate fails, no `BeginDrain`, no quit.

**d) `HostShouldQuit` is only diagnostic.** The launcher reducer DOES detect the orphan-instance state and emits `Event::HostShouldQuit` (`agentmux-common/src/ipc.rs:1057-1069` — the comment explicitly says "ADVISORY, not a hard command"). The host receives it in `agentmux-cef/src/launcher_ipc.rs:416`:

```rust
Event::HostShouldQuit { .. } => {
    tracing::warn!(target: "wrr",
        "[wrr] HostShouldQuit received — host close cascade should be in flight");
}
```

Pure log line. The handler comment cites three prior attempts (`v0.33.491–v0.33.494`) at making this event drive real work that all failed (CEF post_task drops, direct UI-thread call is UB, PostThreadMessage(WM_QUIT) ignored). So the policy was downgraded to "trust the host's own cascade" — which works in the happy path but, as we've now seen, doesn't recover from orphan ghosts.

## 3. Why this isn't a missing reducer

The reducer architecture is in place (master spec §3, §4): launcher canonical for `windows`/`pool`/`instance_registry`; host owns `browsers`/`window_pool` as deliberate scaffolding (master spec §4.4, FFI sync constraints). The launcher's mirror correctly observes the divergence. The gap is on the **action side**: the host has the corrective event but no action wired to it, and the master spec §9.1 calls cross-process dispatch a BLOCKER for the principled fix:

> F.5's PoolRespawnSaga emits IssueCmd::Host as log-only no-op. There's no launcher→host pipe yet. **Spec needed.** Sketch in [SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md]; needs formal write per phase-fg-roadmap §3.

This spec proposes a **bounded, near-term fix** that lives entirely inside `Event::HostShouldQuit`'s existing cross-process delivery channel — no new pipe, no new dispatch primitive — until the principled cross-process dispatch lands.

## 4. Decision

Make `Event::HostShouldQuit` actively reconcile the host's `browsers` map against the launcher's authoritative window mirror, then re-run the existing close cascade. The event already crosses processes via the broadcast bus; we're upgrading its handler from "log only" to "log + reconcile + cascade".

**Out of scope:** generalizing this beyond shutdown (e.g., reconciling mid-session orphans). The concern under master spec §9.2 (per-event saga_id correlation, deferred ~300 LOC) is upstream — this spec doesn't depend on it. We act on the *existence* of `HostShouldQuit`, not on a saga_id match.

## 5. Design

### 5.1 New host-side reconciliation step

Add `agentmux-cef/src/commands/orphan_reconcile.rs` with a single entry point:

```rust
pub fn reconcile_and_drain(state: &Arc<AppState>) {
    let snapshot = collect_orphan_pool_browsers(state);
    if snapshot.is_empty() {
        tracing::info!(target: "wrr",
            "[orphan-reconcile] no orphans — host is consistent; cascade should already be in flight");
        return;
    }
    tracing::warn!(target: "wrr",
        "[orphan-reconcile] reaping {} orphan window-pool-* browser(s)", snapshot.len());
    for (label, hwnd_opt) in snapshot {
        post_close_or_fallback(label, hwnd_opt);
    }
    // The Stage-2 hook in client::mod.rs::on_before_close fires
    // quit_message_loop when browser_list empties. Each orphan close
    // funnels through there, so we don't need to call it from here.
}
```

Internals:
- `collect_orphan_pool_browsers(state)` — snapshot under lock, return `Vec<(String, Option<HWND>)>`. An entry is **orphan** when:
  - label starts with `window-pool-`
  - label is NOT in `state.unpromoted_pool_labels` (i.e., promoted out)
  - launcher's `shadow_window_meta.contains(label)` is false (i.e., launcher's mirror has dropped it)
- Drop the lock immediately. Do NOT touch CEF inside the lock (master spec §4.4 — snapshot-and-drop discipline).
- `post_close_or_fallback` mirrors the existing two-stage cascade: `PostMessageW(WM_CLOSE, ...)` if hwnd is non-null; otherwise `cef::post_task(UI, ClosePoolBrowserTask)` as the fallback path that already exists in `client/mod.rs:817-826` (codex #601 P1 fix).

### 5.2 Wire `HostShouldQuit` to the reconciler

Replace the log-only handler in `agentmux-cef/src/launcher_ipc.rs:416`:

```rust
Event::HostShouldQuit { .. } => {
    tracing::warn!(target: "wrr",
        "[wrr] HostShouldQuit received — running orphan reconciler");
    crate::commands::orphan_reconcile::reconcile_and_drain(state);
}
```

The handler runs on the IPC reader's thread. The reconciler must NOT call CEF directly from here — it goes through `cef::post_task(UI, ...)` for any browser-touching work. (This is exactly what previous attempts got wrong per the comment at `launcher_ipc.rs:421-428`. The reconciler isolates the `post_task` calls so the IPC handler stays cheap and stays off the UI thread.)

### 5.3 Idempotency

`HostShouldQuit` is documented idempotent (`agentmux-common/src/ipc.rs:1064`). The reconciler must preserve that:
- Snapshot is read-only — taking it twice yields the same result modulo intervening real closes.
- Two `HostShouldQuit` events in flight produce two snapshot+post_close passes; the second pass finds an empty orphan list (browsers already closing) and returns early.
- A user-opened new window between the two events lands in `unpromoted_pool=false` AND `shadow_window_meta=true` (launcher mirror tracks it), so it's NOT classified as orphan and stays alive.

### 5.4 Race: user opens new window during reconcile

The existing `Event::HostShouldQuit` doc string already calls this out (`agentmux-common/src/ipc.rs:1062-1066`):

> the host's handler should re-check state.browsers before actually quitting (the user could open a new window in the same dispatch tick race window)

Two distinct races have to be handled:

**Race A — Stage-2 quit:** Our handler doesn't quit directly — it just closes orphans. The Stage-2 quit in `on_before_close` re-evaluates `browser_list.is_empty()` after every close, so a new user window opened mid-reconcile keeps `browser_list` non-empty and Stage 2 stays parked. Benign by construction.

**Race B — promotion before mirror echo:** A pool window can be promoted (`promote_pool_window` removes the label from `unpromoted_pool` and queues `ReportWindowOpened` to the launcher) BEFORE the launcher's `WindowOpened` event echoes back to populate `shadow_window_meta`. A shadow-only orphan check would classify the freshly-promoted live window as orphan and `WM_CLOSE` it (codex #702 round-1 P1).

**Race C — zombie HWND (the original v0.33.643 case):** A promoted pool window's HWND is destroyed without the host's `on_before_close` ever running (CEF crash, OS kill, missed callback). Host's local `window_meta` was inserted in `on_after_created` and is only cleared by `on_before_close`, so the local entry is stale. Launcher's `apply_hwnd_destroyed` removes the label from its mirror and emits `WindowClosed` — so shadow correctly drops the label, BUT host's local meta keeps it. A round-2 attempt at fixing Race B by unioning shadow + local meta would skip exactly this case (codex #702 round-2 P1).

**Discrimination via HWND validity (round 3, current design):** Race B and Race C both produce labels in `browsers` that are absent from shadow and not in `unpromoted_pool`. The classifier returns both as *candidates*. The orchestrator then applies a Win32 `IsWindow(hwnd)` check on each candidate's underlying HWND:

- `IsWindow == 1` (live HWND) → freshly-promoted, Race B → SKIP. The next `HostShouldQuit` (or the launcher's `WindowOpened` echo populating shadow) will resolve this naturally.
- `IsWindow == 0` (destroyed HWND) → zombie, Race C → CLOSE.
- `host()` returns `None` or `window_handle().0.is_null()` → also treated as dead → CLOSE.

This puts the discrimination in exactly one place — the orchestrator's `hwnd_is_dead_or_missing` filter — and uses the OS as the source of truth for "is this HWND alive". No new state, no time-based wait. Non-Windows targets fall back to the null-handle check (best-effort; v0.33.643 is Windows-specific).

### 5.5 What we don't change

- `drain_pool_if_last` query semantics in `saga_dispatch.rs` stay as-is. Once the reconciler clears orphans, the next `WindowClosed` (for the orphan's eventual close) re-runs the saga and the query returns `was_last=true` correctly.
- `BeginDrain` reducer command stays as-is.
- The two-stage cascade in `on_before_close` stays as-is. We feed inputs into it via `PostMessageW(WM_CLOSE)`, same channel the cascade already uses for orderly drains.
- `state.pool.queue` (host reducer) stays the source of truth for *unpromoted* pool. Orphans are by definition *promoted* — the reducer's `state.pool` is already correctly empty for them.

## 6. Tests

`agentmux-cef/src/commands/orphan_reconcile.rs::tests`:

1. **`reconcile_no_orphans_logs_only`** — populate state with one promoted user window (in `browsers`, not in `unpromoted_pool`, IS in `shadow_window_meta`). Run reconciler. Assert: zero close calls dispatched.
2. **`reconcile_one_orphan_posts_close`** — same as above but `shadow_window_meta` does NOT contain the label. Assert: one `PostMessageW`/post_task dispatched for that label.
3. **`reconcile_idempotent_under_repeat`** — call reconciler twice with same state. Assert: two close dispatches total *(not duplicated for the same label)*. Wait — the second call's snapshot still sees the orphan (close hasn't completed yet), so it WILL re-post. Document this: the cost is harmless duplicate WM_CLOSE, which Windows coalesces. Test documents the expected behavior, not "no duplicate work".
4. **`reconcile_skips_browser_pane_labels`** — confirm `browser-pane-*` is excluded (panes drain via a different cascade).
5. **`reconcile_skips_unpromoted_pool`** — pool windows still in `unpromoted_pool` are skipped (not orphans, just warm-pool members the user hasn't pulled).
6. **`reconcile_handles_null_hwnd`** — orphan with `hwnd_opt = None` falls through to `post_task(close_browser)`. Mirrors the existing fallback path.

Wire into `agentmux-cef`'s test bin. Mock the `post_task` and `PostMessageW` paths through a test trait (same pattern `SagaActionRunner` uses in `saga_dispatch.rs`).

End-to-end smoke (manual, not automated — CEF integration tests aren't currently set up):

7. Reproduce the user's v0.33.643 case: launch portable, open + close a window twice (so a pool slot gets promoted-then-orphaned), close the visible window. Confirm host process exits within 5s of last close.

## 7. Migration

Single PR. No new event types, no schema changes, no IPC contract change. Reducer is unchanged. Master spec §4.3's "Phase F (host reducer migration)" item is not affected — this fix lives in the scaffolding layer (master spec §4.4) deliberately, since `browsers`/`window_pool` aren't on the migration path.

Bump patch.

## 8. Master-spec follow-ups

After this lands:

- **Update master spec §4.4** — note that `Event::HostShouldQuit` is now an active reconciler hook, not just a log line. The "scaffolding" mental model still holds; we just wired one of the existing cross-boundary signals to do real work.
- **Master spec §9.1** (cross-process dispatch blocker) is unchanged. This fix is a pragmatic bypass for ONE event-driven flow; the principled fix (saga-issued `IssueCmd::Host` actually delivering work) still needs `SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md` to land. When that lands, the orphan-reconciliation logic should be revisited — the saga's `DrainPoolIfLast` could become a real action-runner that calls `reconcile_and_drain` directly, removing the `HostShouldQuit` shim.
- **Optional next**: extend the `--diag wrr` tool to expose orphan count as a watchable metric. Master spec §3 mentions cross-process observability; this is one more dimension.

## 9. References

- `agentmux-cef/src/launcher_ipc.rs:416` — current diagnostic-only handler.
- `agentmux-cef/src/saga_dispatch.rs:354-371` — `drain_pool_if_last` query.
- `agentmux-cef/src/client/mod.rs:680-826` — two-stage close cascade.
- `agentmux-common/src/ipc.rs:1057-1069` — `HostShouldQuit` event definition + advisory semantics.
- `agentmux-launcher/src/wrr/mod.rs:274` — `OrphanDestroy` drift kind detection.
- `agentmux-launcher/src/reducer/connection.rs:48` — orphan-instance transition fires `HostShouldQuit`.
- Master spec §4.4 — why `browsers`/`window_pool` are scaffolding indefinitely.
- Master spec §9.1 — cross-process dispatch as the eventual principled fix.
- `SPEC_SAGA_DURABILITY_2026-05-01.md` — saga durability layer (orthogonal; reconciler is stateless).
- `SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md` — the principled fix this spec defers to.
