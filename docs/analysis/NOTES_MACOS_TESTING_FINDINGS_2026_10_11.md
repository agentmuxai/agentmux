# macOS testing findings log, 2026-10-11

**Date:** 2026-10-11
**Status:** living. A running log of what was seen while testing the macOS window-paint work (#4615, #4657, #4658, #4688, #4690); entries are added as they are found and marked when fixed.
**Author:** AgentO
**Trigger:** Repo owner, 2026-10-11: *"make sure you record issues during testing, always save notes to file"*, after side findings had only been mentioned in passing in chat.
**Related:** `ANALYSIS_MACOS_WINDOW_PAINT_2026_10_10.md` (the measurements and recommendations these came out of).

Every entry says what was done, what was seen, whether it reproduced, and what is known or not. Entries marked **unverified** have not been reproduced.

---

## F1. Tearing off a tab that holds a browser pane crashes the host (existing bug)

**Root cause (found 2026-10-12): the test Mac's local CEF framework predates agentmuxai/cef#11; the app code is right.** Fixed for dev and local packaging by making `scripts/verify-cef-framework-darwin.sh` check for #11.

- Since #4599 (2026-10-10) a browser pane in the default jar keeps the default Profile in every window. A secondary window's own page runs on a unique off-the-record Profile (`commands::create_isolated_request_context`). So a pane in a secondary window puts two Profiles in one widget. A tear-off lands the tab in a pool window, which is a secondary window.
- In stock CEF, `CefWidgetImpl::AddAssociatedProfile` adds the widget as an observer of `ThemeServiceFactory::GetForProfile(profile)` once per new `Profile*`. `ThemeServiceFactory` redirects an off-the-record Profile to its original's service (`kRedirectedToOriginal`), so the second Profile adds the same observer again: `Observers can only be added once!`. The stack names `ModalDialogHostObserver` only because identical `ObserverList::AddObserver` instantiations are folded into one symbol.
- agentmuxai/cef#11 (merged 2026-10-10, fork commit `6fba204`) counts observed ThemeServices instead. The pinned runtime `cef-macos-arm64-154.0.8037.58-codecs-r2` carries it, and #4599 relies on it.
- `task dev` on macOS takes the framework from `~/cef-build/darwin/aarch64` when one is there. On the test Mac that framework was built from the fork on 2026-09-29, before #11. The framework check looked only for `BeginWindowDrag`, which it has, so it passed, and the bundle step's warning about #11 never printed.

**Proof:**
- Old framework, no tear-off: a browser pane created (through the page) in an ordinary second window aborted the host 20 ms after `browser_view_create`, with the same FATAL and stack (third macOS crash report, 2026-10-12 02:30 local). The pane in the main window, whose page also runs on the default Profile, was fine.
- Pinned r2 framework (SHA-256 matches its release note), same dev build: a browser pane in a pool window (`own_jar=false`) returned its `OverlayController` and the host stayed up.
- Symbols: r2 has two local `std::map` symbols keyed by `raw_ptr<ThemeService>` (the map #11 adds); the old framework has none.

**Who is affected:** builds bundling a fork framework from before #11, that is `task dev` and a local `task package:macos` on a Mac with an old `~/cef-build/darwin/aarch64`. The released v0.59.20 DMG has r2's code: its framework's `__text` hash equals r2's. A locally packaged 0.59.20 DMG on the test Mac has the old framework's code, so that build crashes the same way.

**Seen:** 2026-10-11 07:46 local, a `task dev` build of `main` plus #4688 (so without #4657's pane move and #4690), CEF 154.0.8037.58, macOS 26.5.2, Apple M2.

**What was done:** a tab holding a browser pane (an `https://example.com` page) was grabbed by its tab-strip entry and released a short way below the strip. The test driver (a synthetic mouse drag) had meant to move the window by its header and landed on the tab. The tab was torn off into a new window.

**What happened, in the host log:**

1. `tab tear-off on release`, then `tear-off used warm pool` (`tear_off_pool_promote`): the tab moved to a pre-warmed pool window.
2. The pane was closed in the old window (`browser_pane_close`, `BrowserPaneClosing` / `BrowserPaneClosed`, `close_browser(force=1)`, then the deferred `OverlayController` destroy at `on_before_close`). The log does not show which window the next `add_overlay_view` call was for.
3. About 0.35 s later the host aborted:

```
FATAL:base/observer_list.h:371] NOTREACHED hit. Observers can only be added once!
```

with the stack (top frames):

```
base::ObserverList<web_modal::ModalDialogHostObserver, ...>::AddObserver
CefWidgetImpl::AddAssociatedProfile(Profile*)
CefBrowserViewImpl::AddedToWidget()
views::View::AddChildViewAtImpl
CefOverlayViewHost::Init(views::View*, scoped_refptr<CefView>, bool)
CefWindowView::AddOverlayView(...)
window_add_overlay_view(...)            // our call
agentmux-cef  (a UI task)
```

The app restarted itself and restored its windows. macOS wrote a crash report (`~/Library/Logs/DiagnosticReports/agentmux-cef-2026-10-11-074649.ips`; a copy was kept in the agent workspace).

**Reproduced:** twice, both times by accident.

**Second occurrence, 2026-10-11 22:22:06 UTC (15:22 local), on a plain `main` dev build** (the merged #4615, #4657, #4658 and #4688; not #4690, so the macOS pane changes of #4690 are not involved). The build had been left idle with Tab 2 holding one browser pane (`example.com`). Another agent's scripted mouse drags, aimed at a different window that sat under it at the same position, landed in this window's tab strip and tore Tab 2 off. Same crash, same stack (`CefWidgetImpl::AddAssociatedProfile` from `CefBrowserViewImpl::AddedToWidget` from `CefOverlayViewHost::Init` from `CefWindowView::AddOverlayView`), a second macOS crash report of the same shape.

Host log, 22:22:05.7 to 06.5 UTC: a pool window was created and promoted for the torn-off tab (`tear_off_pool_promote`), the page there registered its backend window, then `BrowserPaneCreateRequested` for the same block id with a new label (`...-2`, window `window-pool-3ba64...`) at 06.464, `browser_view_create succeeded` at 06.464, `[on-after-created] registering browser via reducer` for the new pane at 06.521, and the `FATAL ... observer_list.h:371 Observers can only be added once!` at 06.5315, ~11 ms after the new pane's browser was created. The pane for the old window (`...-1`) was still being closed (`on_before_close: no backend window ID registered ... after 5 retries`).

So in both occurrences the crash comes right after the pane for the same block is created in a window a tab was just torn off into, while the pane's old copy in the source window is still closing. Not yet known: whether the shared `pane jar` profile of the old pane and the new one is the object added twice, or whether the new window already holds a pane (answered 2026-10-12, see the root cause at the top of F1). Evidence kept in the agent workspace: both macOS crash reports, the host-log sequence and the fatal stack.

**Known / not known:**
- The crash is inside CEF's Views code while our `add_overlay_view` call attaches a pane's `BrowserView` to a window. The assertion says the same associated-profile observer was added to the widget twice, so the view or its profile was attached to that widget already.
- It is **not** caused by the macOS pane changes in this series: the build had none of #4657 or #4690, only the edge-resize fix, which only changes mouse-event routing.
- **Unverified:** whether it needs the warm-pool window (a pool window may already hold the pane's profile association), a pane that was just closed in the old window (the deferred destroy racing the new create), or just any tab move with a pane. A pane that is already in a pool window, or a tab without a pane, was not tried. **Answered 2026-10-12:** none of these; any default-jar pane in a secondary window crashes on a framework without cef#11, and the closing old copy plays no part.
- `close_browser(force=1)` and `OverlayController` destroy are deliberately deferred in `browser_pane/creation_views.rs` (`detach_browser_pane_view`) because of an earlier crash of the same family (`weak_ptr.h:250`, tab tear-off); this one may be the same race from the other side.

**Other agent's data point (2026-10-11):** Masty reported 7 tab tear-offs (window tabs and pane tabs, none holding a browser pane) on a dev build of `main` with no crash. That fits the crash needing a tab with a browser pane in it, but is not proof.

**To do:** ~~reproduce on `main` with the log level raised; find which object is attached twice~~ done, see the root cause above. Left: replace the stale framework in `~/cef-build/darwin/aarch64` on the test Mac with r2 (a shared folder outside any repo, so it needs the owner's go-ahead). A real tab tear-off on r2 has not been tried; the pane-in-a-pool-window run above is the same path without the drag.

---

## F2. Browser panes reported themselves invisible after a window move (unverified)

**Seen:** 2026-10-11, same dev session as F1, a build with #4688. Immediately after a header drag moved the window (the move was real: it went from `@300,201` to `@394,237`), `CGWindowListCopyWindowInfo` listed both pane windows with `alpha=0`, and they were still `alpha=0` 3 s later. A tab switch brought them back to `alpha=1`.

**Reproduced:** no. Later header drags on rebuilt builds, including one with #4690, did not show it again, though several of those drags did not actually move the window. Nothing in the host's window-drag code sets a window's alpha (`ui_tasks/drag.rs`); the only writers are `set_window_transparency` and the pane-hole mask path.

**Not known:** whether the panes were really invisible on screen (screen capture needs a permission that was not granted, so it could not be seen) or only reported as such; what set the alpha.

**To do:** if it is seen again, record `winlist` output right after the move and the host log lines containing `[opacity]` or `pane_hole_mask`.

---

## F3. The window-paint report is out of date

`ANALYSIS_MACOS_WINDOW_PAINT_2026_10_10.md` (#4615) was written before the fixes. It still lists terminal refits (§6.1, M1) and the pane geometry task (§6.2, M2) as open. They are now fixed:

| Report item | Shipped as | Result |
|---|---|---|
| M1, cost-aware terminal refits | #4658 | fast drag over 4 terminals: 16–19 of ~24 frames before, 19 in every run after; with each refit made 6 ms costlier, 63–70 before, 67–70 of ~81 after |
| M2, pane geometry task | #4657 | fast drag, edge pane: p90 off the window edge 300–330 pt → 95–150 pt; settled 200–255 ms → 27–67 ms after the last resize |
| (new, not in the report) pane follows the window | #4690 | edge pane p90 off the window edge 90–120 pt → 5 pt |
| (new, not in the report) | #4688 | with a browser pane open, no window edge or corner could start a resize (a mouse hook skipped AppKit's resize tracking). Present since the hook was added; reproduced on a `main` dev build, and reported by the owner on the installed 0.59.16 |

**To do:** a short "what shipped" section in the report, and strike M1 and M2 from its open list. M3 (a real-mouse drag) was in effect done by the owner testing the build by hand: "very smooth".

---

## F6. A dev window sat on its startup splash because its Vite server exited

**Seen:** 2026-10-11 ~13:24 local, `task dev` of `main` on a fresh branch (data dir `agento-tearoff-repro`), started detached from an agent shell with `nohup ... < /dev/null`. Vite came up (`ready in 2665 ms` on port 5340), the app window opened and the page loaded, but the UI never rendered: `#main` had no children after 190 s, 250 resources loaded, none pending, nothing in the page's console. Reloading failed with `ERR_CONNECTION_REFUSED` on `localhost:5340` for every module: nothing was listening. The `task dev` process and its launcher were still alive, so the app had no code server and `task dev` neither noticed nor restarted it. The host log then showed `Load error` once a second.

**Repeated:** three restarts, same result each time, 45 s to ~5 minutes after the window opened. A first run of the same build, started as an ordinary background job of the agent shell, kept its Vite for ~25 minutes.

**Cause (confirmed by the fix):** Vite shuts down when its stdin closes (it skips this only when `CI=true`). `nohup task dev < /dev/null` gives Vite a closed stdin; an ordinary background job keeps it open. Starting `task dev` with its stdin held open, `tail -f /dev/null | nohup task dev > log 2>&1 &`, kept Vite up for more than 3 minutes and the UI rendered. Other agents' dev builds on the machine were not the cause (an early suspicion, since another agent's `task dev` was running each time).

**Not fully fixed by that:** the `tail -f /dev/null | nohup task dev` window kept its Vite for the first few minutes, but about an hour and a half later its Vite was gone again (the `tail` process was gone too, so Vite's stdin had closed once more; why `tail` ended is unknown, possibly cleaned up with the agent's shell session). The app and window were still running, only the code server was missing. Starting Vite alone in its own session (`CI=true`, `start_new_session=True`, stdin from /dev/null) restored it without touching the window, and it held. A bare `nohup npx vite ... &` from the agent shell died silently.

**Gap in `task dev`:** it does not notice that its own Vite has gone, and the window shows a splash or a reload loop with no explanation. A health check on the Vite port, or `CI=true` for Vite, would have saved ~40 minutes.

---

## F7. A page-driving script that creates a block right after the window opens fails

`BackendEndpointNotSetError: object.CreateBlock: backend endpoint not set yet`, from a CDP script that called `createTab`/`createBlock` on a window whose backend endpoints were not injected yet. Wait for `document.title` to show a tab before driving the page.

---

## F8. After a crash, the restored torn-off window opens over the main window

After the tear-off crash (F1) the app restarted itself and restored its windows. The torn-off window came back empty ("Window 2 - Tab 1", `restoring=1`) at (100,100), 1200x800, in front of the main window at (300,201), covering most of its tab strip. A test that drags in the main window's tab strip then hits the wrong window (this is how a second agent's scripted drags ended up in the wrong window, F1). It was moved aside with the host's `set_window_rect`.

---

## F4. Left open on purpose (not built; recorded for later)

- **Terminal text during a drag.** A terminal refit is still a ~4.5–7.5 ms GPU wait; #4658 spreads them out but cannot remove them. Removing them means keeping terminal text at its old size until the drag ends. Product decision.
- **WebGL contexts.** More than 16 live terminals loses contexts and the page can reload up to three times; hidden terminals hold a context while painting nothing (report M4).
- **No macOS minimum window size** (`minimum_size` is `cfg(windows)`); the layout collapses panes near 770 px wide (report M5).
- **Intel Macs.** Several existing places read an `NSRect` (a 32-byte struct) through plain `objc_msgSend` (for example `ui_tasks/pane_geometry.rs`), which needs `objc_msgSend_stret` on x86_64. Only arm64 is packaged. The window-border check added in #4688 does use the Intel-safe call.
- **ProMotion (120 Hz).** Every measurement was at 60 Hz; a 120 Hz display halves the frame budget.
- **First-paint gap on macOS** (report M6): the Windows paint gate was never ported and the gap was never measured.

---

## F5. Testing notes (for whoever measures next)

- **Synthetic mouse input works** from a process with Accessibility permission (`CGEventPost`); it found F1 and the #4688 bug. Edge presses must land within a few points of the real border; a window that has moved needs its position re-read first. Screen capture does not work without Screen Recording permission, so pane contents cannot be seen, only window frames and alpha.
- **A scripted drag that grabs the wrong thing can tear off a tab.** The header's draggable area and the tab strip are close together; move the press point into the empty strip beside the tabs, and never run synthetic drags on a window someone is using.
- **A dev window started as a background job of an agent shell is killed after ~30 minutes.** Start it detached, but never with `< /dev/null`: Vite exits when its stdin closes (F6). Use `tail -f /dev/null | nohup task dev > log 2>&1 &` and, for a window that must stay up for hours, also run Vite alone in its own session with `CI=true` (F6 has the details).
- **Fast drags matter.** A 30 pt/frame drag showed pane lag (300 pt p90) that a 4 pt/frame drag (60 pt p90) understated.
- The probes (`scripts/perf/macos-paint/`) and the numbers behind F3 are in #4615.
