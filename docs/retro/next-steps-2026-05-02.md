# Next steps after the H.7 misdiagnosis — 2026-05-02

Companion to [`h7-freeze-fix-retro-2026-05-02.md`](./h7-freeze-fix-retro-2026-05-02.md). Read that first.

## What we're actually solving

**Not** a freeze. A **no-op create**: the user clicks "open another window," the host registers the window (InstancePanel row appears), but the window itself never becomes visible. 4 of 7 user-initiated windows in the 0.33.589 smoke session reached `main_window_focus`; 3 did not.

## Guiding principle

**Stop probing with code. Start probing with logs.** PR #6 was a code probe based on an unverified hypothesis. The host log on `0.33.589` already has the data needed to root-cause why 3 of 7 windows never foreground. Diagnostic-first; code only after a specific failure mode is identified.

## Phase 1 — Root-cause "windows never foreground" (highest priority)

The user's actual symptom on 0.33.589 was 3 of 7 user-initiated windows that never reached `main_window_focus`. This is concrete and reproducible.

### Step 1.1 — Identify the 3 failed windows

In the existing host log at `~/Desktop/agentmux-0.33.589-x64-portable/data/logs/agentmux-host-v0.33.589.log.2026-05-02`:

```bash
LOG=~/Desktop/agentmux-0.33.589-x64-portable/data/logs/agentmux-host-v0.33.589.log.2026-05-02
# All user-initiated window labels
for L in $(grep -oE '"label":"window-[a-f0-9]+' "$LOG" | sort -u | sed 's/"label":"//'); do
  if [[ "$L" == window-pool-* || "$L" == "main" ]]; then continue; fi
  GOT_FOCUS=$(grep -c "main_window_focus.*$L" "$LOG")
  echo "$L  focused=$GOT_FOCUS"
done
```

Expect 4 with `focused=1+` and 3 with `focused=0`. Note their labels.

### Step 1.2 — Bisect each failed window's lifecycle

For each `focused=0` label, grep the log and answer:

- Did `[on-after-created] registering browser via reducer` fire? (CEF callback ran)
- Did the page load? (Look for `Injected IPC port`)
- Did frontend init fire? (`[frontend] Initializing as new window`)
- Did `register_backend_window` fire? (Frontend completed init handshake)
- Did `[ipc] main_window_focus` arrive? (Frontend asked host to foreground)
- If yes to focus call: did `[main-focus-reclaim] Win32 SetFocus` succeed?

The first "no" in this chain locates the failure.

### Step 1.3 — Hypothesize from where the chain breaks

| First "no" | Likely cause | Fix direction |
|---|---|---|
| `on-after-created` | CEF create failed silently | Inspect cef-debug.log for the label |
| `Injected IPC port` | Frame-creation hook didn't fire | Audit `client.rs::on_after_created` for the failed-window codepath |
| `[frontend] Initializing` | Page load didn't complete | Network/cert issue; check page URL |
| `register_backend_window` | Frontend init crashed | Frontend console errors in same log |
| `main_window_focus` | Frontend never decided to focus | `app-init.ts` — focus-on-mount logic |
| `Win32 SetFocus` failed | OS rejected focus (window destroyed mid-call) | Lifecycle race; likely real and worth fixing |

## Phase 2 — Investigate `HwndWithoutBrowser` label collision

In the same log:
```
HwndDriftDetected { kind: HwndWithoutBrowser, label: Some("window-b4d929d9601f43a7a1b7a4c4da92e412"),
  hwnd: Some(2884770), detail: "ReportHwndOpened label_hint=window-b4d929d9601f43a7a1b7a4c4da92e412
  already linked to a different hwnd=5375424" }
```

The same label was registered for two distinct HWNDs. This is a real concurrency bug. Hypotheses:

1. `wrr/win_event::handle_event` peeks the back of `pending_window_creations` to label OS-level WM_CREATE events. If WM_CREATE fires for a window AFTER its corresponding pending entry was popped (by `on_after_created`), the peek returns the NEXT pending entry — wrong label.
2. CEF reused an HWND from a freshly-destroyed window before the destroy callback fully processed.

Diagnostic: grep the log for the affected label window-b4d929 from `[create-window] task entered UI thread` through both `BrowserRegistered` and `ReportHwndOpened`. Compare timestamps to `pending_window_creations` enqueue/dequeue events to see which queue entry was active when each WM_CREATE fired.

## Phase 3 — Reconsider the H.7 hypothesis (only if Phase 1 implicates panes)

The "freeze" framing tied this to pane state. With the corrected framing (no-op create), pane state is just one of many things that COULD interact with create. Only revisit the H.7 axis if Phase 1's root cause involves a pane lifecycle event in the failed-window timeline. Otherwise abandon it — H.7 was solving the wrong problem.

## Phase 4 — Should PR #6 be reverted?

Tally of evidence:
- PR #6's gate **never fires** in normal operation (no panes were `Closing` during smoke). Inert in practice.
- PR #6's pool-refill kick is harmless given the capacity check that landed in 0.33.589.
- PR #6's `any_pane_closing()` helper has no other callers; it's dead weight.

Two options:

- **Leave inert.** No user-visible harm, no maintenance cost beyond reading a few unused functions. Avoids a revert PR's churn.
- **Revert.** Cleaner main; removes misleading "wfr:gate" log target that no production code emits to.

Recommendation: leave inert until either Phase 1's root cause is known (might revive parts of PR #6) or a contributor is annoyed enough to revert.

## Phase 5 — Resume Phase H runner work (eventually)

The H.6 top-level window runner is still architecturally desirable independent of the freeze. After Phase 1-4 stabilize, pick up the deleted `agenta/h6-toplevel-runner-wiring` design:

- Wire `AppState::host_dispatch_with_effects` (executor for `EffectKind::PostCreateWindow`, `CloseOrphanBrowser`, `SpawnPoolWindow`)
- Migrate the 4 producer call sites (`open_window_with_kind`, `open_window_at_position`, `spawn_pool_window`, main entry) to dispatch `EnqueueTopLevelWindow` via the new executor
- Wire `client.rs::on_after_created` to dispatch `TopLevelCallbackFired`
- Drop the redundant H.7 producer-site checks from PR #6
- Single chokepoint for top-level creation; observable via `HostEvent::TopLevelCreation*`

This unblocks PR #8 (H.8 durability) and PR #9 (H.9 wire-promote events) per the original Phase H plan.

## Memory updates already applied

- [`reference_log_paths.md`](../../C--Systems/memory/reference_log_paths.md) — added portable-mode log path table
- [`feedback_build_workflow.md`](../../C--Systems/memory/feedback_build_workflow.md) — corrected "portables run concurrently"
- [`MEMORY.md`](../../C--Systems/memory/MEMORY.md) — same correction in Build Workflow section

## What NOT to do

- Don't open another freeze-fix PR without first running the Phase 1 grep. Code probes without log evidence have a 0/2 hit rate this cycle.
- Don't widen the H.7 gate speculatively per spec §5 — that's another untested probe.
- Don't revert PR #6 reflexively. It's inert, not harmful.
- Don't add more `wfr:gate` warnings without callers — they pollute log search.
