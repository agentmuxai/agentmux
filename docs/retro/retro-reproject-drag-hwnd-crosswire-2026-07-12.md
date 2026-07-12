# Retro: dragging Window 2 moves Window 3 (HWND cross-wire after crash-reproject)

2026-07-12. Reported live, mid-session, on v0.53.2: 3 windows open at start
(main + 2 recreated windows). Dragging the second window by its title bar
visibly moved the third window instead.

## Investigation (read-only, against the live instance — nothing killed or modified)

- `tasklist` found the live host, PID 1896, single process for all 3 windows.
- CDP target list (`node .verify_step4p2_cdp.mjs 9222 --list`) showed **both**
  non-main windows' boot URLs carry `&restoring=1` — i.e. both came from
  **crash-reproject**, not a cold boot. This immediately narrowed the search
  to the reproject code path (`SPEC_PILLAR1_HOST_REPROJECT_DESIGN`).
- `window.api.listWindowInstances()` gave the host's own label↔windowId
  bookkeeping for the two extra windows.
- Ground truth via an inline PowerShell/C# `EnumWindows` probe (real HWND,
  PID, rect, title) confirmed all 3 windows share one process and gave the
  authoritative title↔label↔HWND table:

  | Title | Label | HWND |
  |---|---|---|
  | Window 2 | `window-cc3cae59856d426c94057731952c8fa7` | `66866` |
  | Window 3 | `window-fbc0dc2faa5a4dd2bc52b0fd78a09bcf` | `66870` |

- Read `frontend/app/hook/useWindowDrag.win32.ts` in full. The native drag
  path (default) is a single fire-and-forget `start_window_drag` IPC per
  drag gesture, carrying `label: ownWindowLabel()` — and `ownWindowLabel()`
  reads `windowLabel` from **that renderer's own URL**. Each renderer can
  only ever report its own, correct label. **The frontend cannot be the
  source of a cross-window mixup here** — this pointed the investigation
  entirely at the host's label→HWND resolution.
- Read the host handler, `commands::window::motion::start_window_drag`
  (`agentmux-cef/src/commands/window/motion.rs:220`): resolves
  `resolve_window_hwnd(state, label)`, falls back to
  `find_own_top_level_window()` if that's null, then drives a manual
  Win32 move loop on whatever HWND it got.
- Read `resolve_window_hwnd` (`agentmux-cef/src/commands/window/lifecycle.rs:311`):
  step 1 is a `state.window_hwnds` cache lookup (label → HWND) — this is
  the authoritative source when populated correctly.
- Read `capture_hwnd_for_label` (`lifecycle.rs:493`), the function that
  populates that cache. This is where the bug lives.

## Root cause

`capture_hwnd_for_label(state, label)` runs once per window, triggered by
that window's own `set_window_init_status { status: "ready", label }` IPC
call (`commands/backend.rs:369`) — i.e. independently, per-renderer, with
**no ordering barrier between windows**.

Its fast path (`browser.host().window_handle()`) is frequently NULL in CEF
Views mode at this point (the function's own doc comment: *"may be non-NULL
by this point even in CEF Views mode"* — an admission it often isn't). When
the fast path misses, it falls back to:

```rust
// Fallback: pick the first eligible visible HWND not already mapped.
let known: HashSet<isize> = state.window_hwnds.lock().values().cloned().collect();
for hwnd_raw in find_all_own_windows() {
    if known.contains(&raw) { continue; }
    ...
    state.window_hwnds.lock().insert(label.to_string(), raw);
    return;
}
```

This picks **whichever visible top-level HWND of this process isn't yet
claimed**, in `EnumWindows` (Z-order) enumeration order — not the HWND that
actually belongs to `label`. The function's own comment states the
correctness assumption plainly: *"Reliable because windows are opened
sequentially (pool windows are hidden before promotion)."* That assumption
holds for ordinary, one-at-a-time window creation. It does **not** hold when
two windows are created back-to-back and both reach their fallback path
before either has registered.

Crash-reproject is exactly that scenario. `reproject_from_snapshot`
(`agentmux-cef/src/commands/window/creation.rs:503`) loops over every
snapshotted window and calls `open_window_with_kind` synchronously in a
tight `for` loop; each call does nothing but **post** a `CreateWindowTask`
to the UI thread (fire-and-forget — the surrounding code's own comment: *"Ok
here means only that a CreateWindowTask was successfully POSTED... not that
the window actually exists yet"*). So for a 2-window reproject, both
`CreateWindowTask`s land on the UI thread's queue within the same tight
loop, both windows come into existence within milliseconds of each other,
and both renderers can plausibly finish loading and fire their own
`ready` IPC within the same short window. When that happens:

1. Window 2's `capture_hwnd_for_label` fast path misses, calls
   `find_all_own_windows()`. At that instant window 3's HWND may already be
   visible but not yet claimed. It picks *whichever HWND `EnumWindows`
   yields first* — which, by Z-order, has no guaranteed relationship to
   "the window whose renderer just signalled ready."
2. Window 3's `capture_hwnd_for_label` runs moments later, same fallback,
   and claims whatever's left.
3. Depending on timing, label↔HWND gets crossed: `window_hwnds["window-cc..."]`
   (Window 2's label) ends up holding Window 3's real HWND, or vice versa.
4. Every subsequent `resolve_window_hwnd(state, "window-cc...")` call
   returns the wrong (cached, "validated" via `IsWindow`) HWND forever —
   this isn't a one-off glitch, it latches. Dragging "Window 2" from then on
   moves Window 3, consistently, exactly as reported.

This is the *same bug class* the codebase has already named and partially
defended against elsewhere — `find_own_top_level_window`'s own doc comment
calls it *"the root of the recurring 'wrong window' bug class (#1165, #1166,
the 2026-05-30 browser-pane parent bug)"*, and `resolve_window_hwnd_strict`
exists specifically because the EnumWindows-fallback family is unsafe
whenever more than one top-level window can plausibly match. `capture_hwnd_for_label`'s
fallback is a sibling of that same family, exercised at window-creation time
instead of window-action time — and it wasn't hardened the same way,
because until crash-reproject existed, multiple windows being created
close enough together to race it was rare (normal multi-window sessions are
opened one drag/click at a time, by a human, seconds apart).

## Would tonight's architecture work have fixed this?

**No.** Everything shipped tonight — the crash-reproject fast/slow-path
mechanism itself (Pillar 1 Step 4), the `Client.windowids` leak-class fixes,
pool adoption, srv recycle-on-crash, the UI-thread liveness probe — operates
on **backend window-id bookkeeping** (srv rows, `Client.windowids`,
label_remap for parent/child relationships) or **process-level supervision**
(host/srv liveness). None of it touches `window_hwnds`/`capture_hwnd_for_label`,
which is purely an **in-process, host-local Win32 concern**: which physical
HWND does a given frontend label correspond to. Reproject's `label_remap`
guarantees the *backend* identity of a recreated window is correct
(new srv row correctly superseding the old one); it says nothing about,
and never touches, the *native HWND* binding.

If anything, tonight's work is what made this bug **easy to hit**: crash-reproject
is precisely the mechanism that creates multiple windows in rapid,
back-to-back succession from a single driver loop — the one precondition
`capture_hwnd_for_label`'s fallback comment assumed wouldn't happen ("windows
are opened sequentially"). This is a **latent bug the reproject feature
newly exposes at meaningfully higher frequency**, not a bug reproject
introduced or a bug reproject's own correctness logic would have caught —
they're orthogonal subsystems that happen to compose badly under load.

## Fix direction (not implemented — investigation/retro only, per this session's scope)

Not attempted yet; flagging the shape of the fix for a follow-up:

- `capture_hwnd_for_label`'s fallback needs a way to attribute a *specific*
  newly-created HWND to a *specific* label, not "first unclaimed one in
  Z-order." The natural fix is to thread the actual HWND through from
  `CreateWindowTask`'s own `CreateWindowExW` return value (the UI thread
  already knows exactly which HWND it just created for which label — this
  is available at creation time, not resolved after the fact) rather than
  reconstructing the mapping later via `set_window_init_status`'s fallback.
- Failing that, at minimum the fallback should serialize: hold a lock (or
  process reproject creations one at a time with a completion barrier)
  across the "enumerate → claim" section so two windows' fallbacks can never
  interleave.

## Related

- `agentmux-cef/src/commands/window/lifecycle.rs` — `capture_hwnd_for_label`,
  `resolve_window_hwnd`, `resolve_window_hwnd_strict`, `find_own_top_level_window`
  (all read this session).
- `agentmux-cef/src/commands/window/motion.rs` — `start_window_drag`.
- `agentmux-cef/src/commands/window/creation.rs` — `reproject_from_snapshot`.
- `docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md` §P1 —
  prior documentation of the same EnumWindows-fallback hazard class.
- `SPEC_PILLAR1_HOST_REPROJECT_DESIGN` / `SPEC_PILLAR1_STEP4` — the
  crash-reproject mechanism whose window-creation loop is the trigger.
