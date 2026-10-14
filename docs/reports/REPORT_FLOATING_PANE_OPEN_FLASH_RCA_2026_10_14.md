# Report: a floating pane flashes white and its logo flickers when it opens

**Date:** 2026-10-14
**Status:** analysis. Cause 2 (the logo) is fixed on a branch and verified; cause 1 (the white) is located, not yet fixed.
**Platform:** Windows, CEF 154, `main` @ `78f171c56` (includes #4767).
**Related:** `SPEC_WINDOW_OPEN_LATENCY_AND_WARM_POOLS_2026_10_13.md` (the pools), `SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md` (the splash and the tear-off snapshot), `docs/analysis/cef-white-flash-retro.md` (the original startup flash).

## 1. The report

Opening a widget as a floating pane ("Open in Floating Pane" from the widgets bar) shows a white flash and the pulsing logo flickers. Opening it as a new window looks solid.

## 2. How it was measured

A recorder (lab scripts, outside the repo) records the new window's own composed surface with `PrintWindow(PW_RENDERFULLCONTENT)` at 45–60 frames per second from the moment it becomes visible, so it sees the window even when it opens behind the operator's other windows. Per frame it records mean brightness, the share of pure-white pixels, and whether the splash logo is visible. A second tool lines the frames up with the host log. Two traps found on the way, both fixed before any number below was taken:

- The recorder must be DPI-aware. At 125% scaling an unaware process is given scaled window sizes and captures only the top-left 80% of the window.
- The recording is of the window's surface, not of the screen; it cannot see a flash that only the display compositor adds.

Everything below is from isolated portables of `main`, never the operator's instance. Spaced-out runs always open warm; a **cold** open is forced by opening two floating panes back to back (the second finds the pool empty).

## 3. What the user sees

| Open | First frames | Logo |
|---|---|---|
| Floating pane, warm (a pooled window promoted) | dark from the first frame | one period, ~180 ms |
| **Floating pane, cold** (no pooled window) | **1–3 frames of pure white, 35–90 ms** | **appears, vanishes for ~80–120 ms, appears again** |
| New window | 2 frames of black (30–50 ms), then ~150–250 ms of dark | one period, ~250 ms |

Ten cold floating opens on `main`: white in 10 of 10; the logo vanished and returned in 10 of 10.

## 4. Why floating panes get it more than windows

A new window comes from the window pool, which holds two or more ready windows. A floating pane comes from the **pane pool, which holds one**. After a promote the pool is ready again after 0.47–1.4 s (median ~0.85 s: the refill waits for the floater's reveal, then spawns a window and waits for its renderer). Another open inside that second, a first open right after launch, or a pool shed under memory pressure (older builds shed at Warn; #4767 sheds only at Critical) takes the cold path. Opening several widgets in a row, as a bench or an impatient user does, hits it constantly.

## 5. Cause 1: the white (cold path only)

The cold path (`open_floating_pane_window` → `floating_pane.rs::create_popup`) creates the popup, then calls `ShowWindow(SW_SHOWNOACTIVATE)` at once, then embeds the CEF browser (host log: HWND created +14 ms, browser created +28 ms, window visible and white at +50–60 ms, first paint reported at +250 ms). For the first 35–90 ms the visible window has an embedded browser that has not composited a frame, and it shows white.

Already in place and **not** enough:
- the window class has a dark background brush (`#1e1e2e`, #1662 polish), and `BrowserSettings.background_color` is dark (`floating_pane.rs:692`, `:929`);
- `DwmExtendFrameIntoClientArea` is called while the window is still hidden, the order the original retro found necessary.

Ruled out by experiment: **GPU compositing startup**, one of the three causes in `cef-white-flash-retro.md`. With `--disable-gpu-compositing` the white appeared in 5 of 5 cold opens, unchanged.

What is not yet established is which surface is white (the embedded child window before its first frame, or the outer window's glass region). The original retro's recipe for the same symptom is to create the window hidden and show it only when the page is ready. The warm path never shows it because the promoted window's page has already painted.

## 6. Cause 2: the logo vanishes (cold boots)

`initApp` ("Init Bare", `app-init.ts`) sets `document.body.style.opacity = "0"` and `visibility: hidden`, and restores them when `initMux` finishes. The splash (`#startup-loading`) is a child of `<body>`, with `opacity: 1 !important` and `visibility: visible !important`. Those override the splash's own values, but **an ancestor's `opacity: 0` hides the whole subtree whatever the child says**. (`index.html`'s comment shows the splash was meant to survive this guard. A child can override an ancestor's `visibility`, which is what `!important` does there, but never its `opacity`.)

On a cold boot the static splash from `index.html` paints first, then `initApp` hides `<body>`, then `initMux` restores it. The recorded timeline matches the host log:

| Time (ms after the open request) | Frames | Host / page log |
|---|---|---|
| 76–115 | logo appears | static splash painted before any script has run |
| ~266–302 | | `initApp-start`, then "Init Bare" |
| ~320–350 | logo gone | body hidden |
| ~384 | | "Init AgentMux" |
| ~420–450 | logo back | `initMux` done, body restored |
| ~700–770 | logo fades out | reveal gate settled |

A warm floater never shows this: its pool page ran "Init Bare" while hidden, long before it was shown.

**Verified by a fix.** Hiding the app root (`#main`) instead of `<body>` leaves the splash visible. Alternating fresh labs, five cold opens each, twice: current `main` lost the logo in **10 of 10** opens (an ~80–120 ms gap between two logo periods); with the fix **0 of 10** did. Two of the fixed runs painted the logo late (the static splash waits on page load under CPU load), and once it appeared it never vanished.

## 7. Fixes

| | Change | Status |
|---|---|---|
| F1 | Hide `#main`, not `<body>`, in `app-init.ts` (three places) and in `error-display.ts`'s `mountRoot` | Done on a branch; the 10/10 → 0/10 result above |
| F2 | Cold floater: create hidden, show on first paint (or cloak until then), as the original retro's recipe does | Not done. Cost: the cold window appears ~200 ms later than today, with no white. To measure before shipping. |
| F3 | Fewer cold opens: refill the pane pool when a window is promoted, not after its reveal; or hold two. Each pooled window is ~55 MB of renderer. | Not done; trades against the memory work |

F1 is frontend-only and independent of the others. F2 needs a host build to verify. Of F3, "refill on promote" costs CPU during the reveal, which is why it was deferred; the measured 0.47–1.4 s ready-again time is the number to beat.

## 8. Side findings

- A new window shows ~30–50 ms of black and then ~150–250 ms of dark before its logo. Not a flicker, but the same "nothing yet" gap.
- Six windows opened and closed in one lab session left six parked `about:blank` windows (a renderer each): the churn leak, `SPEC_WINDOW_CHURN_RENDERER_LEAK_2026_10_13.md` §3.1.
- A new local build triggers a Windows Firewall prompt for its server binary. It appears over the area under test and is not a product issue.

## 9. Reproducing

Lab scripts (memlab workspace): `wincap.ps1` (recorder), `flashcap.mjs` (open and record), `logopulse2.py` (logo timeline), `ab-flash.sh` (fresh lab, N cold opens), `flashsum.mjs`, `loglines.mjs`. Run `wincap` under Windows PowerShell 5.1.
