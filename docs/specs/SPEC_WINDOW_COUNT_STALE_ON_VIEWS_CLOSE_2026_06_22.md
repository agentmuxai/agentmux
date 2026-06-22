# SPEC — Stale window count "(N)" after closing a Views window

- **Status:** Draft → implementing (Part 1 close-fix done + smoked; Part 2 delivery-resync designed)
- **Date:** 2026-06-22
- **Two causes:** Part 1 (§1-7) — a missing *emit* (the close was never reported). Part 2 (§8-11) —
  a lossy *delivery* (the report was dropped on the wire and never reconciled). The user-visible
  count is only correct once BOTH are fixed.
- **Author:** AgentA
- **Sibling of:** #1676 / `SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md` / Discussion #1680
  (same root cause; this is the per-window *count* symptom, distinct from the last-window *quit*).
- **Reproduced by:** user (this machine) + another agent (another machine) — open 2 windows, close
  one → status bar next to the version still shows `(2)` though only 1 window remains.

---

## 1. Symptom

With 2 windows open the status bar shows `(2)` (StatusBar.tsx renders `({windowCount()})` when
`windowCount() > 1`). Close one window → 1 remains, but the bar **still shows `(2)`** — the count
does not decrement.

## 2. Root cause (same as #1676)

The count flows:

```
StatusBar "(2)"  ←  windowCountAtom  ←  setWindowCountAtom(state.instances.length)   (launcher-event-reducer.ts:104)
                     state.instances  ←  launcher WindowOpened / WindowClosed events
                     WindowClosed     ←  launcher_ipc::report_window_closed(label)   (host)
                     report_window_closed  ←  client::on_before_close (client/mod.rs:888)
```

The Views window close **HIDES/recycles** the window (warm pool) instead of destroying the browser,
so **`on_before_close` never fires** (confirmed for #1676, and here: closing the window logged
`Unregistered browser: floating-*` for its tear-offs but **no** `Unregistered browser: window-*`).
Therefore `report_window_closed` never fires → the launcher never emits `WindowClosed` → the
frontend's `state.instances` keeps the closed window → `windowCountAtom` is stale.

`on_before_close` was the **only** caller of `report_window_closed`. The window ✕
(`getApi().closeWindow()` → `close_window` RPC) does the close but does **not** report it — it relied
on `on_before_close` to do so. On a Views recycle-on-close that link is dead.

## 3. Fix

Report the close from the path that *knows* the user closed the window — the `close_window` RPC
(`commands/window/lifecycle.rs`) — instead of relying on the dead `on_before_close`:

```rust
pub fn close_window(state, args) -> Result<...> {
    let label = args.get("label")...unwrap_or("main");
    // The Views recycle-on-close path won't fire on_before_close, which is the
    // only other caller of report_window_closed — so report the logical close
    // here so the launcher emits WindowClosed and the frontend's window count
    // ("(N)") decrements. Idempotent on the launcher (no-op on unknown/already-
    // removed labels), so harmless if on_before_close DOES fire (real destroy).
    // Skip browser-pane-* (same gate as on_before_close, client/mod.rs:887).
    if !label.starts_with("browser-pane-") {
        crate::launcher_ipc::report_window_closed(label.to_string());
    }
    ... existing close (post_close_window / WM_CLOSE fallback) ...
}
```

- **Scope:** `close_window` only (the window ✕). `close_window_by_label` (tear-off merge + floater
  close) targets windows with *real* HWNDs whose `WM_CLOSE` → `on_before_close` *does* fire, so they
  already report; not changed here (revisit if a floater-count symptom shows up).
- **Idempotency / double-report:** `report_window_closed` is paired with `WindowOpened` and the
  launcher silently no-ops unknown/already-removed labels (codex P2 #577), so an extra report (if
  `on_before_close` later fires on a genuine destroy) is harmless.

## 4. Edge cases to verify in smoke

1. **Decrement:** 2 windows → close 1 → `(2)` → `(1)` (the bug).
2. **Recycle → reopen:** after the close, open another window → does `report_window_opened` re-fire
   (pool promote / on_after_created) so the count goes back to `(2)`? (If recycle reuses the browser
   WITHOUT re-emitting `WindowOpened`, the count could undercount on reopen — verify; if broken,
   escalate to §6 alt.)
3. **Last window:** close the final window → count → `(0/1)` AND the instance still quits cleanly
   (the #1676 path — must not regress).
4. **Tear-offs:** floaters open/close during the sequence don't skew the *window* count.

## 5. Out of scope / non-goals
- The quit-on-last-window behavior (#1676, merged) — must not regress; this only fixes the count.
- Floater/tear-off counting semantics (floaters are sub-windows, not instances).

## 6. Alternative considered (if §4.2 fails)
Drive `windowCountAtom` from the **OS-visible window count** the win_event hook already computes for
the quit gate (`count_visible_user_windows`) — report it to the frontend on every
HIDE/SHOW/CREATE/DESTROY. Robust (reflects actual visible windows, no event-pairing/recycle issues)
but a larger change (new host→frontend reporting path). Prefer the minimal §3 fix unless reopen
undercounts.

## 7. Verification
Build a portable, reproduce (2 windows → close 1 → expect `(1)`), then exercise §4.2-4.4. The live
repro instance from the bug report is a ready baseline.

---

# Part 2 — the deeper cause: lossy per-renderer event delivery (the "3 vs 4" desync)

## 8. Second symptom + root cause

Smoking the Part-1 fix surfaced a DISTINCT bug: with 3 windows open, one window showed `(3)`
(correct) while two showed `(4)`. The host accounting was CORRECT — `report_window_opened: 4`,
`report_window_closed: 1` → net 3 (Part 1's close-fix working). The DISPLAY disagreed **per-renderer**.

Root cause: `windowCountAtom = state.instances.length` is reconstructed **independently in every
renderer** from a VERSIONED but LOSSY launcher event stream (`window.__agentmux_launcher_event(json)`;
host side `agentmux-cef/src/launcher_event_bridge.rs`). The frontend consumer
(`frontend/util/event-buffer.ts`, `PerSourceTracker.deliver`) detects two conditions:
- **gap (missed events):** `event-buffer.ts:243-247` → the default `onVersionGap` handler
  (`event-buffer.ts:162-168`) **only `console.warn`s** ("version gap: expected X, got Y"), then
  advances `lastVersion` and accepts the new event — **the skipped events are gone forever**.
- **stale (out-of-order):** `event-buffer.ts:235-240` drops (fine — idempotent).

There is **no production `onVersionGap` callback** — every live tracker (`launcher-events.ts:49-56`,
`srv-events.ts:71-78`) uses the log-only default. So a dropped `WindowClosed` leaves that renderer's
`knownEntries` (→ `state.instances` → `windowCountAtom`) permanently over-counting. The healing
machinery EXISTS — `reconcileKnownEntriesFromSnapshot` (`launcher-event-reducer.ts:146` →
`ReconcileFromSnapshot`, `launcher-event/reducer.ts:272-315`, wholesale add/remove) backed by the
authoritative `list_window_instances` RPC — but its only caller (`InstancePanel.tsx:91-103`) is gated
behind `!launcherEventsActive()`, i.e. **dev-mode only; it never runs in production**.

So Part 1 fixes a missing-EMIT cause; Part 2 fixes a dropped-ON-THE-WIRE cause. Both are needed.

## 9. Fix — resync-on-gap (reducer-authoritative reconciliation)

The authoritative window-instance state lives in a reducer (the launcher's `state.windows`, exposed
via `list_window_instances`). Renderers must RECONCILE against it when they detect they've fallen
behind, instead of trusting a lossy incremental stream forever.

1. Wire a real `onVersionGap` for the launcher `PerSourceTracker` (`frontend/util/launcher-events.ts:49-56`;
   the hook exists at `event-buffer.ts:103`, fired at `:246`).
2. In the callback: re-pull `getApi().listWindowInstances()` → `reconcileKnownEntriesFromSnapshot(snapshot)`
   (`launcher-event-reducer.ts:146`). The `ReconcileFromSnapshot` arm already does the wholesale
   add/remove that heals an over- or under-count.
3. Bypass the dev-only gate (`InstancePanel.tsx:91`) for the gap-triggered reconcile — the whole
   point is to reconcile AFTER events have been flowing.
4. **Race guard (codex P1 #733):** snapshot `launcherEventVersion()` before the RPC; discard the
   reconcile if a newer event landed while the RPC was in flight (don't clobber fresh state with a
   stale snapshot). Reconcile only on a DETECTED gap, never unconditionally.

This is the minimal robust fix — it wires existing pieces (`onVersionGap` hook + `ReconcileFromSnapshot`
+ `list_window_instances`), no new host reporting path.

### 9.1 Alternative (rejected for now): versioned snapshot push
Have the bridge push full authoritative instance snapshots (versioned) so a missed incremental
self-heals on the next snapshot. More robust (no pull RPC, no race window) but a larger host-side
change. Defer unless resync-on-gap proves insufficient (e.g. gaps frequent enough to cause an RPC
storm).

## 10. Latent twin
`srv-events.ts` uses the SAME lossy `PerSourceTracker` with the same log-only gap handling, but
currently has ZERO production consumers (`subscribeSrvEvent` unused). It becomes the same desync risk
the moment an atom-router subscribes — apply the same resync-on-gap pattern then, paired with an srv
`GetSnapshot`/`Resync` RPC (the unbuilt Phase D flow). Flagged so it isn't a silent future regression.

## 11. Relationship to the broader audit
Per-renderer reconstruction from a lossy stream is one instance of a wider single-source-of-truth
gap. The full ranked audit (host quit decision bypassing the reducer, 6× window-count computations,
pane-pool typing, the HWND/role registry, the unwired `reconcile_quit`/H.6 runner) is in
`SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md`. This count fix is Track 3 (reliable delivery) of
that document.
