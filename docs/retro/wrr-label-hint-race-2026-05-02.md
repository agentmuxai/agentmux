# Retro: WRR back-of-queue `label_hint` race — 2026-05-02

## Summary

`wrr/win_event.rs::handle_event::EVENT_OBJECT_CREATE` peeked the back of `pending_window_creations` to label OS-level WM_CREATE events. The peek assumed at most one window create in flight at any time. When users create multiple windows in succession (rapid status-bar clicks, scripted creates, drag-tear-off bursts), multiple pending entries queue up, and back-of-queue returns the **same latest label for every WM_CREATE** until the queue drains.

Worse still: the launcher's drain-on-WindowOpened fallback (`agentmux-launcher::wrr::reducer.rs::handle_report_window_opened`, ~line 810) only drains pending HWNDs whose `label_hint.is_none()`. A wrong hint means the hint-bearing pending entry is invisible to the fallback. The launcher ends up with aliased mirror entries (multiple labels mapped to one HWND, or vice versa) and emits `HwndDriftDetected { kind: HwndWithoutBrowser }` errors.

User-visible symptom (0.33.589 smoke session, 2026-05-02):
> "5 windows opened successfully ... but the list grew to 9 or 10 ... while closing the 5 windows, closing one caused the status bar panel list to collapse more than 1 entry, like it was bound somehow."

That's the alias signature exactly.

## How we found it

1. Initial misdiagnosis treated this as the spec'd "2026-05-02 freeze" (cross-state pane × top-level race). Wrote PR #6 with a `any_pane_closing()` gate. Smoked on 0.33.589 — gate never fired (no panes were `Closing`), symptoms reproduced. See [`h7-freeze-fix-retro-2026-05-02.md`](./h7-freeze-fix-retro-2026-05-02.md).
2. Re-grepping the host log for `main_window_focus` per-window led to a second misread: I claimed the status-bar URL path was missing `workspaceId` and called THAT the bug. User clarified "5 windows opened successfully" — invalidating that theory too, since most status-bar creates work fine.
3. User noted the panel-collapse-on-close behavior. That was the alias fingerprint.
4. Re-read `wrr/win_event.rs:259-275` (the peek-back-and-label code). Comment literally says "by the time we get here the back-of-queue label is the right one for the window we're seeing" — which is only true when at most one create is in flight.
5. Confirmed the launcher's drain fallback at `reducer.rs:810` filters `label_hint.is_none()`. Wrong hints actively block the fallback's recovery path. Drop-the-hint isn't just simplest, it's the only correct option given the existing fallback design.

## Why it stayed latent so long

- Single-create scenarios (typical "open window once, use it") never queue more than one pending entry — back-of-queue is correct in that case.
- Windows that sit in the queue briefly (sub-millisecond) and pop in FIFO order also tend to win the race.
- The drift events fire as `Warn`, not `Error`, for `HiddenSinceOpen` (the most-common downstream symptom). `HwndWithoutBrowser` is `Error` but only fires on the explicit double-link path, not on the more-common silent-alias path.
- The InstancePanel "list grows" symptom is per-`WindowOpened`-event; the panel can't tell that two rows alias to one HWND until you close one and observe the cascade.

## Fix

Drop the back-of-queue peek. Pass `label_hint=None` for every WM_CREATE. The launcher's existing drain-on-WindowOpened fallback (originally written for pool windows that don't push pending entries) handles all WM_CREATEs uniformly:

```diff
- let label_hint = app_state()
-     .get()
-     .and_then(|s| s.peek_back_pending_window_creation())
-     .map(|p| p.label);
- launcher_ipc::report_hwnd_opened(raw_hwnd, class, title, label_hint);
+ launcher_ipc::report_hwnd_opened(raw_hwnd, class, title, None);
```

Trade-off: the diagnostic value of the immediate label-hint (correlation in `[wrr]` logs) is lost. The authoritative label still arrives via `on_after_created` → `ReportWindowOpened` → drain fallback within ~50ms typically. Worth it.

`AppState::peek_back_pending_window_creation` is preserved as-is (5 lines, no harm) for potential future diagnostic use; it gets a "never used" warning that joins an existing group.

## Tests

Pre-existing reducer tests for `handle_report_window_opened`'s drain-on-WindowOpened fallback already cover the `label_hint=None` path (used by pool windows). All 63 host tests + 150 launcher tests pass on the change.

## Smoke validation

Required before declaring fix successful: 0.33.59x build, rapid-fire ~10 status-bar new-window clicks, verify:
- All 10 windows visually appear
- InstancePanel grows by exactly 10
- Closing one window decrements by exactly 1
- No `HwndWithoutBrowser` drift in the log
- No aliased entries in `--diag wrr` snapshot

## Related

- [`h7-freeze-fix-retro-2026-05-02.md`](./h7-freeze-fix-retro-2026-05-02.md) — the misdiagnosis that preceded this fix.
- [`next-steps-2026-05-02.md`](./next-steps-2026-05-02.md) — Phase 2 in that doc was "Investigate `HwndWithoutBrowser` collision." This PR resolves it.
- `docs/specs/SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md` — the "freeze" spec was wrong about the trigger (pane state); the actual trigger is concurrent `pending_window_creations` entries.
