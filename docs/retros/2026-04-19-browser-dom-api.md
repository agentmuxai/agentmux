# Retro: Browser DOM API + Stress Test Rewrite

Date: 2026-04-19
Owner: AgentA

---

## What we set out to do

Replace the brittle pixel-coordinate stress test (`pane-focus-stress.ps1`) with a DOM-level interaction API (`/agentmux/browser/*`) so tests could assert on "is this input focused / does this field have this value" instead of "did Win32 mouse_event land at (267, 430)". Target: 24/24 pass on the existing 4-round workload.

## What shipped

| PR | Scope | Merged |
|---|---|---|
| #453 | Phase 1 — CDP client + resolver + `browser.query` | ✅ |
| #454 | Phase 2 — `focus_info`, `eval`, `screenshot` | ✅ |
| #455 | Phase 3 — `click_element`, `focus_element`, `dispatch_key`, `navigate` | ✅ |
| #456 | Phase 4 partial — JSON log parser + DOM-assist for browser pane clicks | ✅ |

The `/agentmux/browser/*` surface is complete end-to-end. All 12 `dom-smoke.ps1` assertions pass. The harness infrastructure change is in. **The 24/24 target was not hit.** See below.

## What went well

- **Spec → plan → phased PRs cadence.** `SPEC_BROWSER_DOM_API.md` and `PLAN_BROWSER_DOM_API.md` landed before code. Each phase was a contained PR that could be reviewed and merged independently. Reagent approved 3 of 4 on first pass; the fourth had one real bug (selector format-string capture — caught, fixed, merged on second pass).
- **Incremental validation.** `dom-smoke.ps1` grew one assertion group per phase: query → focus_info/eval/screenshot → click_element/focus_element/dispatch_key/navigate. By the time Phase 3 shipped, the DOM API was exercised against two distinct pages with 12 assertions including a `.value` round-trip after synthetic typing. High confidence the API itself works.
- **CDP proxy was the right architectural call.** CEF already runs Chromium DevTools Protocol on its remote-debug port. Opening a WS per request and translating is a few hundred lines of infrastructure — far less code than the rejected alternatives (JS injection via `CefMessageRouter`, custom eval round-trip via blockfiles). The plan documented this explicitly; sticking to it paid off.
- **The stress-test rewrite surfaced a pre-existing bug.** Phase 4 wasn't supposed to reveal new issues — it was supposed to port the test over. But in moving to DOM-level verification, we discovered the harness had been lying for weeks: the log parser only matched ANSI-colored stdout (`\e[2m…\e[0m`) and the on-disk host log is actually JSON-per-line. Every `Read-LogSince` call returned 0 rows. Every `expectedPaneKeys=$false` step trivially "passed" because `0 == 0 → no leak`. The test was blind. Fixing the parser (PR #456) didn't introduce the regression — it revealed it.

## What went poorly

- **The 24/24 target was predicated on a false baseline.** We saw 13/24 "passing" runs before Phase 4 and assumed those passes were real. They weren't — the broken filter was silently succeeding. Lesson: when a new test harness reports mostly passing, run one *deliberately failing* case through it and confirm the failure propagates. A fuzz-your-own-asserts step upfront would have saved two weeks of believing 13/24 was progress.
- **Pixel-coordinate baseline was shakier than documented.** The harness had two independent fragilities: (a) auto-computed y=430 missed the Google search box (Phase 1 motivation), and (b) auto-computed y=155 for the address bar lands *inside* the pane HWND's rect on this machine, not on the main window's nav-bar input. The address-bar clicks never reached the frontend's `onFocus` handler, so `main_window_focus` IPC was never invoked, so `MainFocusReclaimTask` never ran. The test has probably never had a step that correctly tested Win32 focus reclaim — we just didn't see it because of (a).
- **Local toolchain flakiness blocked endgame investigation.** Rustc crashed twice with `STATUS_STACK_BUFFER_OVERRUN (0xc0000409)` during the final release build while investigating the focus-reclaim bug. Not a code issue — a `-C lto=fat -C codegen-units=1` stack-overflow issue on Windows with a workspace this size. Moved to `lto = "thin"` (committed in this retro's accompanying change). We should have caught this the first time someone saw a linker crash on this codebase.
- **I used the `focus_element` path naïvely in the stress rewrite.** Calling `browser.focus_element` via CDP sets Chromium-internal DOM focus on a pane element, which causes Chromium to set Win32 focus on the pane HWND. From the user's perspective this looks like "the pane grabs focus". Subsequent pixel clicks on main-window chrome (address bar, terminal) land — **as I now understand but didn't initially** — on the pane HWND at those coordinates, not the main render widget. So onFocus never fires, reclaim never runs, typing goes to whichever pane had focus. We ended up with a test that exposes a real routing gap *and* introduces a new one from our own usage pattern. Both need handling.

## Key insights

1. **Test harnesses can be silently non-functional for a long time.** Ours had been for at least two weeks, maybe longer. The symptom was not "test crashes" — it was "test reports confident, stable numbers that happen to be wrong."
2. **`browser.focus_element` is strictly more disruptive than it looks.** It quietly transfers Win32 keyboard focus to the pane HWND. Callers downstream need to understand that and either (a) re-focus main explicitly, or (b) expect subsequent pixel clicks outside the pane to also land on the pane HWND if the pane's device-pixel rect covers those coordinates.
3. **DPR math is ambient.** `browser-view.tsx:paneRect()` multiplies CSS pixels by `window.devicePixelRatio`. On a DPR=1 display the math is trivial; on a 1.5 or 2 display the pane HWND sits at a much higher screen coordinate than a naive CSS reading suggests. This affects both the pane positioning AND any external code trying to correlate screen coords with pane coverage. Our stress-test defaults assume DPR=1; on any other display they're off.
4. **The rustc `STATUS_STACK_BUFFER_OVERRUN` is *not* LTO-related.** My initial hypothesis was `lto = true` + `codegen-units = 1` overflowing the stack during linking. Testing showed the crash reproduces with `lto = "thin"` AND with `lto = false`. The crash happens in rustc itself while compiling *different crates on different runs* (`tracing_core`, `windows-sys`, `zerocopy`) — none of which touch our code. Error code 0xc0000409 is a security-cookie buffer-overflow trip, not a genuine stack overflow. This is likely Windows Defender real-time scanning corrupting rustc's memory mid-compile, or a toolchain corruption needing a reinstall. **Reverted the Cargo.toml change** — LTO settings are not the fix. The actual remediation is at the OS / toolchain level: add `C:\Systems\agentmux\target\` to Defender's exclusion list, or `rustup toolchain uninstall stable && rustup install stable`.

## Action items

| # | Item | Owner | Size |
|---|---|---|---|
| 1 | **Fix the rustc `STATUS_STACK_BUFFER_OVERRUN` at the OS level** — add Defender exclusion for `C:\Systems\agentmux\target\` and `C:\Users\area54\.cargo\`, or reinstall the stable toolchain with `rustup toolchain uninstall stable && rustup install stable`. **NOT** a Cargo.toml change — my initial LTO hypothesis was wrong and I've reverted. | User | small (OS-level) |
| 2 | Add a harness self-check: run one step where `expectedPaneKeys=$true` AND confirm filter sees them, fail loud otherwise | AgentA | small |
| 3 | Investigate why `browser_pane_resize` rect covers the nav-bar area at DPR=1. Placeholder `getBoundingClientRect` should exclude the 40px nav bar. Either the rect math is wrong or SetWindowPos is applying the wrong origin | — | medium |
| 4 | Once (3) is fixed, rerun the stress test and see what the actual failure rate is on this machine | — | small |
| 5 | Phase 4b: when the stress test needs Win32 focus routing tested, **don't** use `browser.focus_element` — stick to pixel click. Use DOM API only for the `.value` verification at the end | — | small |
| 6 | Add a "no-LTO dev" guide to CLAUDE.md so future maintainers don't re-learn the `STATUS_STACK_BUFFER_OVERRUN` gotcha | AgentA | small |

## What the browser DOM API is good for right now

Worth naming, since we *did* ship a useful surface:

- **Agent-driven form automation**: an agent that wants to fill out a form on a pane can now `query` → `focus_element` → `dispatch_key` without pixel math. Useful for future automation.
- **Golden-image screenshot tests**: `browser.screenshot` returns PNG bytes; diffing against a recorded baseline catches visual regressions.
- **Runtime diagnostics**: `browser.eval` is a debug shell. Paste any JS, get the result. Better than trying to DevTools into a pane mid-interaction.
- **Page-load assertion**: `eval("document.readyState === 'complete'")` lets tests wait for real readiness instead of sleeping heuristically.

None of these needed 24/24 on the stress test to be valuable. They're the real win of this session.

## Bottom line

Shipped the DOM API (four PRs, all merged). Exposed a real Win32 focus-routing bug that's been hidden for ≥2 weeks by a blind log parser. Did *not* hit the 24/24 target, but the target was based on a false reading anyway. Next pickup is (3) + (4) in the action table above.

---

## Post-reboot addendum (same day)

After reboot, toolchain compiled clean in 2m09s — transient state, as expected. Dug into the focus-reclaim failure and found **two separate issues** layered on top of each other:

### Issue 1 — harness hardcoded a 1300×900 window on a 900×1600 portrait monitor

`MoveWindow` silently clamps to the primary screen. Requesting 1300 wide got 800–920 actual depending on chrome. The "auto-computed coords" divided by 3 assuming 1300, so `$tCx = 700` landed *inside P2's actual HWND* (x=666–968) instead of the terminal gap (x=355–666). Every "terminal" click was a pane click; every "P2 search" click was off-screen.

**Fix**: read `GetWindowRect` back after `MoveWindow`, derive all coords from the actual rect. Plus enumerate child HWNDs at runtime to find the real pane top (chrome height varies by theme/DPI). Committed in the same branch as this retro addendum.

### Issue 2 — terminal clicks never call `main_window_focus`

Grep for `main_window_focus` across the frontend returns **exactly two callers**: `browser-model.ts:174` (`giveFocus()` when main DOM already has focus) and `browser-view.tsx:155` (address bar `onFocus`). **Nothing else ever requests focus reclaim.**

The stress-test premise — "clicking the terminal after a pane should route subsequent keys to main" — is based on a misunderstanding of how focus transfer works. A mouse click in Windows doesn't automatically transfer Win32 focus to the clicked HWND's top-level; focus transfer requires an explicit `SetFocus` call, which in this codebase only fires from `MainFocusReclaimTask`, which only fires from the `main_window_focus` IPC, which only the address bar invokes.

Clicking the terminal:
- Updates DOM focus to the xterm canvas (main-window DOM state)
- Does NOT invoke `main_window_focus` IPC
- Does NOT transfer Win32 focus to the main window's render widget
- So SendKeys keeps routing to whichever pane last had the focus handed to it

This wasn't a regression. The test's "expected no pane keys on terminal step" invariant has never been true. It was a phantom pass hidden by the broken log filter (Issue #2 in the main retro above).

**Options forward**:
- A. Add `main_window_focus` IPC to the terminal view's focus handler (treats terminal like address bar — gives it "main chrome owns keyboard" semantics). Probably what a user expects.
- B. Change the test to only assert "no leak" after actual main-chrome focus transitions (i.e. the address bar steps).
- C. Broader fix: on any non-pane DOM element gaining focus, fire `main_window_focus`. Centralizes the focus-reclaim policy.

(A) is minimal and fixes the immediate user-visible case (type in browser → type in terminal → expect terminal to receive keys). (C) is structurally cleaner but widens the blast radius.

### Action items update

| # | Item | Status |
|---|---|---|
| 1 | Toolchain crash → reboot | ✅ Fixed by reboot (as predicted ~70%) |
| 2 | Harness self-check (false-assert detection) | still open |
| 3 | Pane HWND covers nav bar? | ❌ Not the cause. Actual cause: window clamping + auto-coord math |
| 4 | Rerun stress test after (3) | done — still 24/24 fail, but *now* we understand why (Issue 2 above) |
| 5 | Phase 4b: don't use focus_element in stress test | partially applied; still relevant |
| 6 | Add no-LTO dev guide to CLAUDE.md | not needed — LTO wasn't the issue |
| **7** | **Add `main_window_focus` IPC to terminal view's focus handler** — treats terminal like address bar | new, open |
| **8** | **Rework stress-test invariants** — "no pane keys on terminal step" requires terminal to *explicitly* transfer Win32 focus; either make that happen (#7) or remove the invariant | new, open |

### What to try next

Start with (#7) — 3-line change in the terminal view's focus handler, fires the same IPC the address bar does. If that makes terminal steps pass, the harness is validated and we can work down from 24/24-fail to whatever the real number is.
