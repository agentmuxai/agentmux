# task dev — launcher-driven shutdown gaps

**Date:** 2026-05-06
**Owner:** AgentA
**Status:** spec
**Layer:** crosses Layer 1 (launcher) ↔ Layer 2 (host) — see [`MASTER_REDUCER_STACK_STATUS_2026-05-05.md`](./MASTER_REDUCER_STACK_STATUS_2026-05-05.md) §1.

---

## 1. Problem

`task dev` does not run the `agentmux-launcher` process. The Taskfile's `dev:serve` recipe launches `agentmux-cef` directly:

```bash
cd "$DEV_DIR" && LD_LIBRARY_PATH=. AGENTMUX_DEV=1 ./agentmux-cef --url=http://localhost:5173
```

Portable and installed builds invoke the host indirectly via the launcher. Many cross-process behaviors depend on a launcher being present; in `task dev` they're inert.

**Observed symptom (smoke test 2026-05-06):** closing every visible window of a `task dev` session leaves zombie processes — `agentmux-cef.exe` × 6 + `agentmux-srv-*` × 2 — for the lifetime of the user's login. Identical to the v0.33.643 portable bug PR #702 just fixed, but PR #702's fix can't trigger here because it hooks `Event::HostShouldQuit`, which only the launcher emits.

## 2. What's missing in dev mode

Per master spec §3 / §4 / §6, the launcher owns:

| Subsystem | Owned by | Effect of launcher absence in dev |
|---|---|---|
| `state.windows` mirror | Launcher reducer | Host has no canonical "user-visible windows count" view from outside |
| WRR (Window Reality Reconciliation) | Launcher (`SetWinEventHook` + reducer) | No drift detection (`OrphanInstance`, `OrphanDestroy`, `HwndWithoutBrowser`, etc.) |
| `Event::HostShouldQuit` emission | Launcher reducer's `handle_report_window_closed` + `wrr::apply_hwnd_destroyed` | Reconciler in `agentmux-cef/src/commands/orphan_reconcile.rs` never invoked |
| `window_cleanup_cascade` saga | Launcher saga coordinator | No cleanup cascade fires |
| Cross-process saga dispatch | Launcher → host pipe | Host's saga-action hooks see no commands |
| Launcher event log + ring buffer (Phase D) | Launcher | No persistent event audit trail in dev |
| Window placement / monitor topology (Phase B.9) | Launcher reducer | No off-monitor correction |
| Single-instance enforcement (Phase B.6) | Launcher pipe lock | Multiple `task dev` sessions can collide |

The host-side `on_before_close` cascade (`agentmux-cef/src/client/mod.rs:680`) is the ONLY shutdown path in dev mode. Its Stage-1 close-pool-windows + Stage-2 `quit_message_loop` logic was designed assuming launcher presence in many subtle ways:

- Stage-1 gate `if user_browser_count == 0 && !self.is_browser_pane` reads `state.list_browser_labels()` for the count. The closing browser's `UnregisterBrowser` dispatch happens earlier in the same `on_before_close` body, so by the time the gate evaluates, `keys` should exclude the closing window. In dev mode this should work — but the smoke trace shows it didn't fire (Stage-1 PostMessage(WM_CLOSE) to `window-pool-*` browsers never logged), so something is keeping `user_browser_count > 0`. Hypotheses:
  - The pool labels were promoted (out of `unpromoted_pool`) and now count as user windows.
  - A `browser-pane-*` or other unaccounted browser is still in the registry.
  - Timing: `UnregisterBrowser` is async-dispatched but the snapshot reads pre-dispatch.
- Even if Stage-1 fires, Stage-2 (`if self.browser_list.is_empty()`) gates on the CefClient's internal `browser_list`, which is populated by CEF callbacks, not the reducer's `state.browsers`. PR #702's whole point was to drive `quit_message_loop` directly when the reducer says we're done; without launcher → no `HostShouldQuit` → no direct drive → Stage 2 has to converge on its own.

## 3. Why it matters

Three flavors of pain:

1. **Developer ergonomics.** Every `task dev` session leaves zombies that have to be killed manually before the next one starts (port 5173, srv pipe, AppData lock files). Easy to forget; easy to test against stale state.
2. **Smoke parity.** Anything you smoke-test in `task dev` won't exercise the launcher-driven shutdown path. PR #702's fix can't be smoke-tested in dev — only in portable builds. That's a known good practice (build a portable for shutdown testing) but it shouldn't be the ONLY way to validate.
3. **Phase F follow-ons.** Master spec §9.1 has cross-process saga dispatch as a BLOCKER for `IssueCmd::Host` actions. Until that lands, the launcher's saga coordinator is the only place these dispatches go. Dev mode skips it entirely. As more of the architecture moves to launcher-coordinated flows, dev mode's coverage shrinks.

## 4. Options

### Option A — Run the launcher in `task dev` too

Modify `dev:serve` so it launches `agentmux-launcher` (which spawns `agentmux-cef` for us) instead of `agentmux-cef` directly. Mirror the portable startup contract.

**Pros:**
- True parity. Every shutdown test, drift test, and saga test exercises the same code path as portable.
- No new code paths to maintain — just glue.
- PR #702's reconciler activates naturally in dev.

**Cons:**
- The launcher needs to know to point at Vite (`http://localhost:5173`) rather than the bundled `index.html`. Today the launcher accepts `--url=` only because the dev path goes around it.
- Single-instance pipe lock: launcher's `first_pipe_instance(true)` would block a second `task dev` from running (existing portable instances already grab their own per-version pipe; dev would need its own pipe name).
- Launcher logging path: dev should log to `~/.agentmux/dev/<branch>/logs/` rather than the global `~/.agentmux/logs/`. Already addressed by the data-dir unification work — verify before merging.
- Trap-and-clean: the existing `dev:serve` traps `EXIT` to kill `VITE_PID`. Adding the launcher means we need to kill the launcher (and its child host + srv) on dev-server exit. Cleaner if launcher's normal shutdown path handles it — but if the dev shutdown is unclean (Ctrl+C the task), we need a fallback.

### Option B — Stub launcher events into the host directly

Keep the dev startup as-is (no launcher). Add a `--dev-stub-launcher` flag to `agentmux-cef` that wires up a minimal in-process emulator: emits `HostShouldQuit` when the host's own window count drops to zero, fakes `WindowOpened`/`WindowClosed` mirrors, etc.

**Pros:**
- No process management changes.
- Dev mode stays "single binary, run from anywhere".

**Cons:**
- Two implementations of the launcher's reducer to keep in sync. Every change to launcher logic now needs a stub-equivalent.
- Stub is fundamentally different from the real launcher (no separate process, no WRR via Win32 hooks, no event ring). Smoke parity is illusory.
- This was effectively the v0.33.491–v0.33.494 strategy of trying to drive launcher work from inside the host. Three failed attempts. Not the way.

### Option C — Make `on_before_close` self-sufficient

Refactor the host-side cascade so it doesn't depend on launcher signals. Specifically: when `user_browser_count == 0` post-close, invoke `commands::orphan_reconcile::reconcile_and_drain` directly on the UI thread (bypass `Event::HostShouldQuit`).

**Pros:**
- Single shutdown code path that works in all modes.
- Frees the host from depending on launcher availability for clean shutdown.
- Fully testable via existing 21 planner integration tests.

**Cons:**
- Doesn't address WRR / saga / drift detection — those still need the launcher.
- Risks duplicating shutdown logic if the launcher ALSO emits `HostShouldQuit` — both paths fire.
- The orphan reconciler is currently designed for the post-launcher-emit case (assumes `HostShouldQuit` → reconcile). Repurposing it as the canonical close-cascade entry would inflate its scope.

### Option D — Status quo

Leave dev mode without launcher. Document the gap. Use portable for shutdown testing.

**Pros:** Zero work.

**Cons:** Continued zombie processes. Continued smoke parity gap. Pain compounds as more Phase F lands.

## 5. Recommendation

**Pursue Option A** with a small Option C hedge.

Option A is the principled fix. The launcher already exists, already works in production, and is the source of truth for the shutdown path PR #702 just exercised. Wiring `task dev` through it gives us:
- Real `HostShouldQuit` emission → real orphan reconciliation testing in dev
- Real WRR → drift detection during development (catches a class of bugs we currently can only reproduce in portable)
- Real saga coordinator → cross-process dispatch testing as that work lands

The Option C hedge: while wiring up Option A, also verify that the host's `on_before_close` cascade Stage 2 fires correctly in the simple case (last user window closes, no zombies, no warm pool open). The smoke trace shows it didn't fire in dev — fixing that is independent of launcher presence and should land regardless.

## 6. Implementation sketch (Option A)

### 6.1 Taskfile change

`dev:serve` swaps the final command:

```diff
-                cd "$DEV_DIR" && LD_LIBRARY_PATH=. AGENTMUX_DEV=1 ./agentmux-cef{{exeExt}} --url=http://localhost:5173
+                cd "$DEV_DIR" && LD_LIBRARY_PATH=. AGENTMUX_DEV=1 ./agentmux-launcher{{exeExt}} --dev --url=http://localhost:5173
```

Need to verify `agentmux-launcher` already builds into `$DEV_DIR/`. The `build:host` task currently builds host + launcher per `Taskfile.yml:380`; check the bundle copies both.

### 6.2 Launcher: `--dev` flag

Add `--dev` to the launcher arg parser. Its job:
- Pass `--url=...` through to the spawned host (today the launcher already spawns the host with bundled assets; dev mode swaps to the URL).
- Use a per-branch pipe name (e.g. `\\.\pipe\agentmux-dev-<branch-hash>`) so concurrent `task dev` runs on different branches don't collide.
- Use `~/.agentmux/dev/<branch>/` as the data root (already wired by the recent data-dir unification — verify).

### 6.3 Single-instance handling

The launcher's `first_pipe_instance(true)` lock is per-pipe-name. In dev, the pipe name is per-branch. If the user kicks off a second `task dev` on the same branch, the second launcher fails fast with `ERROR_ACCESS_DENIED` — acceptable.

### 6.4 EXIT trap

`dev:serve` traps `EXIT` and currently kills `VITE_PID`. Extend to also `taskkill /T /F /PID $LAUNCHER_PID`. The `/T` flag kills the whole tree — host + srv go with it. If the launcher exits cleanly on its own (the user hit Ctrl+C, all windows closed), the trap is a no-op.

### 6.5 Smoke test parity

After this lands, the v0.33.643-style zombie scenario (last window closed, warm pool kept alive) should be reproducible in `task dev` and addressable by the same orphan reconciler PR #702 introduced. Add a smoke test note to `docs/specs/SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md` confirming dev-mode applicability.

## 7. Out of scope

- Refactoring `on_before_close` to cover the launcher-absent case (covered by Option C if we choose to land it as a hedge — separate PR).
- Cross-process saga dispatch (master spec §9.1, separate spec).
- Reverse: making the launcher optional for portable too. Master spec §6 keeps the launcher as a hard requirement, by design.

## 8. Tests

Mostly smoke. The existing planner unit tests already cover the reconciler logic. New tests:

1. **Smoke: task dev clean exit.** Start `task dev`, open and close the main window, observe all `agentmux-*` processes exit within ~5s of the last window close. Document this in BUILD.md alongside `task package` smoke instructions.
2. **Manual: task dev orphan reconciliation.** Force a renderer crash (DevTools → "Inspect" → the inspect window crashes the renderer). Confirm `[wrr] HostShouldQuit received — running orphan reconciler` log line appears. (Today this is impossible in dev.)
3. **Manual: per-branch pipe isolation.** Run `task dev` on two different branches concurrently. Both should run; their data dirs / logs / pipes should not collide. The data-dir unification work largely covers this — verify.

## 9. References

- [`MASTER_REDUCER_STACK_STATUS_2026-05-05.md`](./MASTER_REDUCER_STACK_STATUS_2026-05-05.md) §3 (3-level stack), §4 (host scaffolding), §6 (Phase B), §9.1 (cross-process dispatch BLOCKER).
- [`SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md`](./SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md) — the reconciler that this spec wires into dev mode.
- `Taskfile.yml:469-529` — current `dev:serve` recipe.
- `agentmux-cef/src/client/mod.rs:680-880` — `on_before_close` cascade (Stage 1 + Stage 2).
- `agentmux-cef/src/launcher_ipc.rs:416` — `Event::HostShouldQuit` handler (calls `reconcile_and_drain`).
- `agentmux-launcher/src/reducer/window.rs:200-213` — clean-close path's `HostShouldQuit` emission.
- `agentmux-launcher/src/wrr/mod.rs:284-322` — crash-detected close path's `HostShouldQuit` emission (added by PR #702).
