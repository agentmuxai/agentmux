# SPEC — Park-and-blank for non-demotable window closes (renderer-zombie commit leak)

- **Status:** Implemented, live-verified (see §5 results in PR)
- **Date:** 2026-07-09
- **Author:** AgentA
- **Sibling of:** `SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md` (same root mechanism — CEF 148
  Views parks browsers on every close/destroy sequence; that spec fixed the *accounting*
  consequence, this one fixes the *memory* consequence).
- **Reported by:** user — pagefile/commit growth 7→18.5GB with one page open, build ≥0.51.0
  (i.e. AFTER the PR #1939 + PR #2000 leak fixes — this is a distinct leak).

---

## 1. The measured leak

Controlled churn experiment (2026-07-09, isolated instance, fresh channel, 3 cycles of
open-5-windows/close-5):

| Stage | CEF processes | Private commit | System commit | Zombie workspace pages |
|---|---|---|---|---|
| Fresh instance | 9 | 540 MB | 18.76 GB | 0 |
| After cycle 1 | 14 | 1,145 MB | 19.43 GB | 3 |
| After cycle 3 | 20 | 1,755 MB | 20.08 GB | 9 |
| After kill | 0 | — | 18.23 GB (baseline 18.15) | — |

~90 MB commit per closed window, linear, fully attributed (releases on process exit). The
zombie pages are worse than the private numbers suggest: each keeps the FULL workspace app
running invisibly — xterm WebGL canvases (SwiftShader software-GL on current Windows builds =
CPU shared-memory surfaces = pagefile-backed commit visible in NO process's private bytes),
live websockets to srv, timers.

## 2. Root cause

`CloseWindowTask` routes a closing `window-*` window three ways:
1. **Demote** (`window-pool-*` label AND pool below `POOL_DEMOTE_CAP = target+2`) — correct:
   parks off-screen, hides from taskbar, **reloads to the lightweight `pool=1` boot page**
   (`window_pool.rs:797-805`), re-enters the pool. No leak.
2. **Round-5 destroy** (everything else) — `close_browser(1)` + native `DestroyWindow`. On CEF
   148/Windows the browser is PARKED anyway (live-verified in the quit-gate work:
   `on_before_close` never fires), and — the gap this spec closes — **the parked browser keeps
   its workspace page loaded forever**. Round-5 kills the HWND but never unloads the content.
3. Round-3 fallback (strict-HWND failure) — same parked outcome.

So every close beyond the demote cap, and every foreign `window-{uuid}` close (cold-path,
tear-off, reproject — they can't demote: the pool handshake gates on the `window-pool-` prefix,
`window_pool.rs:817`), leaks a full running workspace.

## 3. Design 1 — park-and-blank (this spec)

For non-demotable `window-*` closes, don't attempt the destroy CEF won't honor. Do exactly what
demote does, minus the pool bookkeeping, plus a blank navigation:

```
park_and_blank_window(state, label) -> bool   // window_pool.rs, next to demote
  1. resolve_window_hwnd_strict — failure → false, nothing mutated (same discipline as
     demote step 1 / round-5; caller falls through to the old round-5 path).
     Explicit main-HWND guard, parity with round-5.
  2. Park: SetWindowPos off-screen (POOL_OFFSCREEN_X/Y, SWP_NOSIZE) + set_taskbar_hidden(true)
     (which also fully hides the window) + evict the window_hwnds cache entry.
  3. Blank: browser.main_frame().load_url("about:blank") — the SAME call demote's step 5 uses
     on an already-parked window (proven to work there). Tears down the workspace app: WebGL
     surfaces, websockets, timers all release. MUST run BEFORE step 4 (get_browser resolves
     through state.browsers — the ordering lesson from the quit-gate spec, learned live).
  4. unregister_after_parking_close(state, label) — the shared parking-close discipline from
     PR #2043: UnregisterBrowser dispatch + quit-watchdog arm.
```

Call site: `CloseWindowTask`, after the demote attempt, before round-5. Round-5 remains as the
strict-HWND-failure fallback and as the (working) path for floaters, whose owned-popup
`DestroyWindow` DOES fire `on_before_close` (#1957 mechanism) — floaters are not routed through
park-and-blank.

**Effect:** a closed non-demotable window becomes an inert, invisible, `about:blank` parked
browser (~20-30MB) instead of a hidden full workspace (~90-150MB+shared). Renderer *count*
still grows with churn (CEF won't give the process back — rounds 2-5 of the July retro proved
no close API reaches a parked Views browser), but each zombie is cheap and idle.

## 4. Explicitly out of scope (Design 2 — the named follow-up)

**Pool adoption via relabel:** `RelabelBrowser` exists in the reducer; relabeling a foreign
`window-{uuid}` to `window-pool-*` and demoting it properly would make closed windows reusable
warm inventory and actually bound the renderer count. Rejected for this PR: it walks straight
into the L4 label-prefix minefield (~20 prefix-classification sites,
`SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md`) and needs cross-process label propagation
(launcher mirror, srv window ids). Do it after L4's typed `BrowserKind` lands, if ever.

Also out of scope: raising `POOL_DEMOTE_CAP` (parked-blank overflow is now cheap, cap keeps the
*useful* pool bounded), and the SwiftShader-vs-hardware-GL question (orthogonal; park-and-blank
shrinks the zombie regardless of GL backend).

## 5. Verification

Repeat the §1 churn experiment on a portable build:
1. Closed windows appear as `about:blank` CDP targets — zero `workspaceId=` zombie pages.
2. System-commit growth per cycle drops from ~0.65GB to near-flat (renderer processes still
   accumulate but blank; expect ≤ ~30MB/window).
3. Close-all-then-main still exits the full tree via the clean quit gate (no regression of
   PR #2043 — parked-blank browsers are unregistered, so `registered` reaches 0; process exit
   reaps them like every parked browser today).
4. Demote path unchanged: first closes still enter the pool and reopen instantly.
