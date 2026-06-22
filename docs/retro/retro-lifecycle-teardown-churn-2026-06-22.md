# Retro: "Close last window ⇒ orphaned process tree" — how a solved problem became the most-churned code in the repo

- **Date:** 2026-06-22
- **Severity:** High (recurring; orphaned process trees incl. live agents; "lifecycle corruption")
- **Status:** Root *history* established; current fix still open (see §8). Tracking Discussion on the repo.
- **Scope:** the window-close → host-quit → launcher-teardown → process-reap chain on Windows (with macOS/Linux parity notes).

---

## 0. TL;DR — how did the launcher's Job Object "lose control"?

**It didn't. The Job Object never regressed.** J0 (`KILL_ON_JOB_CLOSE`, `agentmux-launcher/src/main.rs`) is sound and has been since it landed (#497/#570). What broke is the **trigger chain upstream of it**:

```
close window → on_before_close (CEF browser close) → host quits
            → host_child.wait() returns → launcher drops J0 → KILL_ON_JOB_CLOSE reaps the tree
```

The launcher reaps **the instant the host process exits** — it is *entirely* gated on `host_child.wait()`. **No host exit ⇒ no teardown, by design.** And the host stopped exiting because **`on_before_close` stopped firing** on the Windows window-close path.

**The single regression that severed the chain:** commit **`66a02bdb` (2026-03-29 16:32)** — a commit whose message is entirely about *window drag* (`fix(cef): JS-driven window drag via mouse delta + SetWindowPos`) — **drive-by rewrote `close_window` on Windows**, replacing the authoritative `host.try_close_browser()` with `find_own_top_level_window()` + `PostMessageW(WM_CLOSE)`. It dropped the browser handle entirely (`state` → `_state`). From then on, closing a window on Windows depended on *resolving an HWND* and posting `WM_CLOSE` to it — and **the main window is a CEF Views window whose `window_handle()` returns NULL on Win32** (it always has been — see §2), so HWND resolution falls back to a fragile `EnumWindows` guess. When that guess is wrong/void, `WM_CLOSE` hides the frame **without closing the CEF browser**, so `on_before_close` never fires, the host never quits, and J0 never reaps.

The regression was **latent for months** (a single window + a lucky `EnumWindows` match masked it) and only detonated as multi-window, tear-off, and the warm-pool generalization made HWND resolution unreliable.

> **Correction to the prior spec.** `SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md` §1.2 concluded the root cause was "mechanism A — the quit *gate* never arming (`BeginDrain` never fired)," implying `on_before_close` *did* run. That conclusion was incomplete: it checked for drain markers but not for `on_before_close` itself. Two smoke runs on 2026-06-22 show **zero `on_before_close` / zero `Unregistered browser`** — `on_before_close` never runs at all. The deeper cause is the severed close path (`66a02bdb`), not (only) the gate.

---

## 1. The invariant that should hold

> When the user closes every visible AgentMux window, the host process tree should exit cleanly.

Non-trivial because **AgentMux is not one-process-per-window**: the host owns many CEF browsers — visible top-level windows AND a hidden warm pool (`window-pool-*`, `floating-pool-*`) plus browser-pane children. "Did the last *user-visible* window close?" is therefore a judgment call, and every layer that has tried to make that call has gotten it wrong at least once.

---

## 2. The "solid" era — and there was never a Views migration

- **`e61a6df7` `feat: CEF IPC bridge — Phase 2` (2026-03-29 08:30):** the first `close_window` closed the browser directly:
  ```rust
  if let Some(host) = browser.host() { host.try_close_browser(); }  // authoritative CEF close
  ```
  Chain: `try_close_browser()` → CEF closes the browser → `on_before_close` → `browser_list` empties → `quit_message_loop()`. **This worked on a Views window** because it operates on the registered *browser handle*, not an HWND.
- **The main window has been a CEF Views window since the first commit** (`window_handle()` → NULL on Win32; native windows are opt-in only). **Nothing migrated to Views.** The popular "a Views migration severed it" theory is false — the window type never changed; the *close mechanism* did.

---

## 3. The regression and why it stayed hidden

- **`66a02bdb` (2026-03-29 16:32)** — same day, ~8h later. A window-drag commit silently swapped the close mechanism on Windows to `find_own_top_level_window()` + `PostMessageW(WM_CLOSE)`. Non-Windows kept `try_close_browser` (and still does — the divergence that makes this Windows-specific).
- **Latent:** with a single global window, `find_own_top_level_window()` (`EnumWindows` PID match) returned the right frame, and the OS frame's `WM_CLOSE`→`WM_DESTROY` still tore the single browser down. The planted bug only detonates once HWND resolution can return the *wrong* or *dead* window.
- **What made it exploitable, chronologically:**
  - `4e6e0006` (2026-03-29) multi-window registry → `find_own_top_level_window` can now return the wrong window.
  - `#565` (2026-04-27) tear-off → `close_window_by_label` spreads the WM_CLOSE-by-HWND pattern.
  - **`bc7b6054` (2026-05-27) `fix(floating-pane): class-aware fallback when CEF Views hides main HWND`** — explicit acknowledgement that `"main"`'s `window_handle()` is NULL and close/drag must fall back to a class-aware `EnumWindows` guess. **This is the exact fragile fallback in today's evidence** (`[win-resolve] … class-aware EnumWindows fallback`).
  - **`#1133` + `SPEC_WINDOW_HWND_CACHE_STALE_FIX_2026_05_28.md` (v0.39.1):** the title-bar close button silently stopped working — `resolve_window_hwnd` returned a *stale* HWND, `PostMessage(…, WM_CLOSE)` "posts into a dead window," and the spec states verbatim **"`on_before_close` never fires."** *Same close path, same signature as today* — patched at the cache layer (IsWindow liveness), root left intact.
- **The host's own code documents the failure mode** (`client/mod.rs`, Phase B.9.3): it prefers `PostMessage(WM_CLOSE)` but warns that if the handle is null it must fall through to `close_browser` via `post_task` "so the close still happens. **Otherwise `self.browser_list` never empties and Stage 2 never fires.**"

---

## 4. The *second*, independent failure mode (so the chain has ≥2 single points of failure)

Even when `on_before_close` *does* fire, the quit can still fail to start: the **last-window gate is edge-triggered and races warm-pool refill.** Closing the last window triggers a pool refill, so `browser_list` / `user_browser_count` never reaches 0, `BeginDrain` never fires, and the host never quits. Named in **B.9.3 / #601** (2026-04-29, "Cause A"), again in **orphan-reconcile / #702** (2026-05-05, deferral comment *"the next `HostShouldQuit` will catch it"* — which, on the last window, never comes), and it's what `SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION` §1.2 measured (no drain markers in 9,483 log lines). PR #1676 is the first attempt to make this gate **level-triggered**.

**The bug is fragile because the chain has multiple independent SPOFs, and each has been patched in isolation.**

---

## 5. The launcher-authority detour (tried → demoted — do NOT re-litigate)

Phase **B.9.3 (#601)** added a launcher reducer emitting `Event::HostShouldQuit` from `state.windows.is_empty()` — "single authority, re-evaluated on state change." It **could not deliver the quit to the host's UI thread**, across four smoke builds (`docs/retro/b9-3-quit-thread-analysis.md` / `b9-3-lifecycle-analysis.md`):

| Build | Approach | Result |
|---|---|---|
| v0.33.491 | tokio → `post_task(UI)` → `quit_message_loop` | task body **never executed** |
| v0.33.492 | tokio → `quit_message_loop()` directly (off UI thread) | silent UB / no-op |
| v0.33.493 | minimal `post_task` (only `quit_message_loop`) | task never ran |
| v0.33.494 | Win32 `PostThreadMessage(WM_QUIT)` | OS accepts, **CEF's custom pump ignores WM_QUIT** |

**Conclusion (settled):** keep the **host-local** quit decision; `HostShouldQuit` is **documented ADVISORY** (`agentmux-common/src/ipc.rs:1057-1069`). Any new "launcher decides quit" proposal must reckon with this — the hard part was never *deciding*, it was *delivering the quit to the UI thread*.

---

## 6. Complete PR / churn catalog

The quit/teardown path has been **re-architected ~4 times**, each leaving a layer of compensating patches. The current spec puts the fragmentation at **~38 distinct `Phase B.x/F.x/H.x`, `codex #…`, `reagent #…`, `smoke v0.33.x` markers** + ~40 lines of race-condition apologetics inside a single ~460-line `on_before_close`.

| Cluster | PRs | What |
|---|---|---|
| **0 — OS process-binding** | #54 (2026-03-07), #144, #274, **#302** (window-close-kills-shells; *first* label-mismatch-in-`on_before_close` bug) | Pre-state-machine lifetime binding; first "orphan" + first label-predicate bug. |
| **1 — Job Object foundation** | **#497** (Windows Job Objects), **#570** (state-machine redesign + launcher J0 `KILL_ON_JOB_CLOSE`) | The kill-on-close primitive — the "should be solved now" moment. **Never regressed.** |
| **2 — Phase B launcher parent + reducer** | #571–#598 (2026-04-28/29) | Launcher becomes srv-sibling parent, IPC, pure reducer, single-instance, window-state mirror. Several immediate hotfix-the-prior-PR commits (#572, #598). |
| **3 — B.9.3 launcher-authority quit (tried→demoted)** ⭐ | **#601** (+ smoke v0.33.491–498, codex/reagent #601 rounds), #582, #605 | The headline back-and-forth (see §5). Diagnosed the pool-refill cascade; landed the `PostThreadMessage(WM_QUIT)` workaround; authority demoted to advisory. |
| **4 — F.6 window-cleanup saga** | #629 (F.1 + `pending_window_creations`), **#637** (F.6 saga; round-2 codex P1 before merge), #714 | Launcher saga brackets cleanup. Its docstring is the "fossil record": "log-only… no pipe yet" then a later "CPD-3 update… now LIVE" in the *same file*. |
| **5 — Phase H host-reducer authority** | #654–#661 (PRs 1–5, multiple codex/reagent rounds each), **#662** (H.7 freeze-fix for a freeze the H series introduced), #722 | Re-architecture #4: quit/window state moved into a host-side reducer *because* launcher authority failed. |
| **6 — Orphan reconciliation (decision site #2)** | **#702** (kills v0.33.643 zombie; ~15 intra-PR fix commits), #705, #706, #664 (drain-on-WindowOpened add→remove→restore) | A *second* place the quit decision is recomputed (`orphan_reconcile.rs`), deferring with "next `HostShouldQuit` will catch it." |
| **7 — Supervision / crash recovery** | #945, #1120, #1121, #1229 | Auto-restart + crash budgets layered on top, interacting with quit. |
| **8 — macOS/Linux parity** | **#1268** (macOS `drop(runtime)` wedge → `shutdown_background()` + `process::exit(0)`; must NOT be reverted), #1286 (Linux `PR_SET_PDEATHSIG`) | Two *distinct* teardown impls beside the Windows J0. |
| **9 — Pool generalization (re-broke it)** | #1595, #1609, #1610, #1612 (2026-06-20), #1639/#1652/#1654 | Routed *every* new-window/tear-off through pool promotion cross-platform → re-surfaced failure-to-quit + blank-window symptoms. |
| **10 — 2026-06-21 incident + current** | #1647 (terminal-window leak: missing `CREATE_NO_WINDOW`), #1650 (blank new window: default-layout welded to first-launch), **#1676 (OPEN)** | Three distinct root causes, three retros. **#1676** = the current attempt: pure level-triggered `reconcile_quit` + first-ever quit-gate tests; **decision-only, not yet wired**; + an uncommitted close-path fix (restore `try_close_browser` on Windows). |

**Recurring themes:** (1) quit authority migrated location twice (OS-bind → J0 → launcher-reducer → host-reducer); (2) the *same* pool-refill-vs-last-window race recurs under new symptoms each time; (3) heavy *in-PR* churn (add→revert→restore within single PRs); (4) symptoms patched at the layer they surfaced, never at the `66a02bdb` root.

---

## 7. Why "a solved problem" keeps coming back (root analysis)

1. **The root regression was never reverted.** `66a02bdb` swapped a working browser-handle close for an HWND-guess close as a drive-by inside a drag PR. Every subsequent fix treated downstream symptoms.
2. **The chain has ≥2 independent SPOFs** (close-doesn't-fire-on_before_close; gate-doesn't-arm) and **2 decision sites that must agree** (`on_before_close`, `orphan_reconcile`) plus an advisory third (launcher saga). Fixing one SPOF leaves the other.
3. **Implicit predicates.** "Is this a user window?" was reconstructed from label-prefix + pool-set membership (drift-prone) rather than the authoritative `BrowserKind::is_pool` flag.
4. **Near-zero end-to-end coverage.** There is **no CI test runner** (`.github/workflows/` runs zero `cargo test`/`vitest`) and no "close last window ⇒ tree exits" test — so every regression shipped silently.
5. **Windows-only divergence.** macOS/Linux kept `try_close_browser`; only Windows took the HWND-guess path — so the bug hid on the platform with the most users *and* the most window-management complexity (pool, tear-off, floaters).

---

## 8. Current state & the open question

- **#1676 (open):** level-triggered `reconcile_quit` decision + `is_pool` gate + tests. Reagent-approved; **decision-only**, not wired to the live close path.
- **Uncommitted close-path fix** (`lifecycle.rs`): restores `try_close_browser` on Windows (i.e. reverts the `66a02bdb` mechanism to the `e61a6df7` original).
- **⚠ Open:** **two smoke runs of a build carrying the close-path fix still showed zero `on_before_close`** and the tree still orphaned. So either `close_window` isn't taking the new `get_browser(label) → try_close_browser` branch (label-routing / `get_browser("main")` returning the wrong thing), or `try_close_browser` on the *current* multi-window Views setup no longer fires `on_before_close` the way it did in `e61a6df7`. **This is the live unknown to resolve before any fix lands.** (`close_window` currently logs nothing about which branch it takes — first action: instrument it.)

---

## 9. Recommended path forward (for discussion)

1. **Instrument `close_window`** (which label, did `get_browser` hit, which branch) and re-smoke — resolve the §8 open question with evidence, not a third blind patch.
2. **Fix at the root:** close via the authoritative **browser handle** (`try_close_browser`) on all platforms — revert the `66a02bdb` divergence — *if* §1 confirms it fires `on_before_close`.
3. **Make the gate level-triggered + reducer-owned** (#1676 direction): the decision belongs in the reducer's `UnregisterBrowser` transition, not ad-hoc in `on_before_close`.
4. **Add a deterministic backstop that does NOT depend on the host gracefully quitting** — the one piece that would make this class of bug impossible: a launcher watchdog that force-closes J0 when no user-visible window remains for a grace period. *But* honor §5: the hard part is **detection** (the launcher only learns of closes via `ReportWindowClosed`, which is starved by the same severed chain). Detection must be independent — e.g. host OS-level `EVENT_OBJECT_DESTROY` re-evaluation, or a heartbeat window-count.
5. **Stand up CI tests** (`cargo test` + `vitest`) and a local-only "tree exits" smoke — so the next regression can't ship silently (§7.4).
6. **Collapse the duplicate decision sites** into one and retire the advisory launcher saga's dead paths.

---

## 10. Lessons

- **Drive-by mechanism swaps in unrelated PRs are how "solved" problems regress.** `66a02bdb` changed the close mechanism inside a *drag* PR; it was invisible in review and latent for months.
- **Patching at the symptom layer compounds.** ~38 markers later, the root (`66a02bdb`) was still there.
- **A sound primitive (J0) is worthless if its trigger chain is fragile.** "We have a Job Object" felt like "solved," but the kill only fires on host exit, which depends on the most fragile path in the app.
- **Verify the *whole* chain, not the first absent marker.** The prior spec stopped at "no `BeginDrain`" and missed "no `on_before_close`." Always confirm which link broke.
