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

## Fix (host side)

Drop the back-of-queue peek. Pass `label_hint=None` for every WM_CREATE.

```diff
- let label_hint = app_state()
-     .get()
-     .and_then(|s| s.peek_back_pending_window_creation())
-     .map(|p| p.label);
- launcher_ipc::report_hwnd_opened(raw_hwnd, class, title, label_hint);
+ launcher_ipc::report_hwnd_opened(raw_hwnd, class, title, None);
```

`AppState::peek_back_pending_window_creation` is preserved as-is for potential future diagnostic use.

## Fix (codex P1 round 2 — drain restored as fallback for hwnd_val=0)

Removing the launcher's drain entirely was too aggressive. `client.rs::on_after_created` only dispatches the explicit `ReportHwndOpened(hwnd, label, Some(label))` when `hwnd_val != 0`. If HWND discovery transiently fails from all 3 sources (Views, host, find_own_top_level_window), the explicit dispatch is skipped — and with no drain, the mirror stays permanently `hwnd: None`. Result: OS destroy events for that HWND find no mirror → no `WindowClosed` → ghost InstancePanel rows the user can't dismiss.

User reported exactly this on 0.33.592 smoke ("only 2 windows visible but list says 7"). Codex caught it as P1 round 2 review.

Two-part fix:

1. **Strengthen host's HWND discovery in `client.rs::on_after_created`.** Add `find_own_top_level_window()` as third fallback (matches the earlier in-function HWND computation at line 296). Reduces `hwnd_val=0` to a truly pathological case + logs an ERROR if it ever happens.

2. **Restore `drain-on-WindowOpened` in `handle_report_window_opened`** as best-effort fallback. The drain may wrong-pick under burst creates, but the `apply_hwnd_opened` repair logic (added earlier in this PR) detects the mismatch when the explicit on_after_created path arrives and repairs. Combined: best-effort link from drain, authoritative repair from explicit dispatch.

Net behavior:
- Burst creates: drain wrong-picks → repair restores truth (transient mis-link, no permanent damage)
- Burst creates with hwnd_val=0: drain links the right HWND (often, by sequence) → no later repair needed
- Single creates: drain picks the only candidate → explicit confirms benign duplicate

## Fix (launcher side — codex P1 follow-up on 0.33.590 smoke)

The host-side fix alone wasn't enough. The launcher's `apply_window_opened` had a drain-on-WindowOpened fallback that used `max_by_key(arrived_at_ms)` — picked the MOST RECENT pending HWND. Under burst creates, the FIRST `WindowOpened(A)` would consume window B's pending HWND (the most recent), then `on_after_created`'s authoritative `ReportHwndOpened(actual_hwnd, A, Some(A))` would hit the double-link path and only emit drift — no repair. The mirror stayed linked to the wrong HWND. Codex caught this on 0.33.590 review.

Two additional changes:

1. **Remove the auto-drain in `handle_report_window_opened`.** WindowOpened no longer attempts to link an HWND. The mirror starts with `hwnd: None`. The explicit `ReportHwndOpened` from `on_after_created` (which has the authoritative label + HWND from `browser.host().window_handle()`) is the sole linking path.

2. **Make `apply_hwnd_opened` REPAIR stale links.** If a mirror is already linked to a different HWND when the explicit `ReportHwndOpened` arrives, overwrite the link to the authoritative HWND and emit `HwndWithoutBrowser` drift for diagnostic visibility. The orphaned prior HWND will be re-attributed when ITS OWN `on_after_created` fires.

```diff
- HwndOpenedOutcome::DoubleLinkedWith(u64),  // emit drift, no state change
+ HwndOpenedOutcome::Repaired(u64),          // overwrite to authoritative + emit drift
```

Trade-off: the diagnostic value of the immediate WM_CREATE label-hint (correlation in `[wrr]` logs) is lost; pending_hwnds is now diagnostic-only (track strays). The authoritative label still arrives via on_after_created within ~50ms.

## Tests

Two new launcher reducer tests for the burst-create scenario:

- `wrr_burst_creates_link_correctly_without_aliasing` — two queued WM_CREATEs + interleaved WindowOpened + on_after_created links, verifies no auto-drain and no aliasing
- `wrr_apply_hwnd_opened_repairs_stale_link` — simulates a stale wrong-HWND link and verifies the explicit on_after_created path repairs it

All 63 host tests + 152 launcher tests pass.

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
