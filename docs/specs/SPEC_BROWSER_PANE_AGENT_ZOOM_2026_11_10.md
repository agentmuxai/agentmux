# Spec: an agent can zoom a browser pane it drives

**Status:** active — approved 2026-11-10; the zoom action, the scaled mouse and the click reasons (§4) ship in PR #6999. Not built yet: the pane-header badge, `fit` with a `ref`, and the screenshot scale line (§5). Verified 2026-11-11. Owner: Korp. Task #47393 (this spec), #47394 (the build).
**Operator:** "agents need to be able to zoom on the browser pane, there are times their clicks cant make it through bc things are off sccreen".
**Checked against:** `origin/main` @ `d2e371838`.

## 1. What fails today

A click by ref scrolls its target to the centre (`CLICK_POINT` in `act.rs`) and sends the mouse events there. It never checks what is at that point, and the Browser tool has no zoom or viewport control. The panes agents drive are small (half of an agent's pane). The cases scrolling cannot fix, from QTest's tally ("pane too small", ranks 4 and 4b) and the sessions on App Store Connect, Play Console, Partner Center and GitHub Support:

| Case | Scrolling fixes it? |
|---|---|
| A dialog or fixed side panel taller or wider than the pane | No: its own contents are cut off by the viewport. |
| A sticky header or footer covers the point the click lands on | No: centring helps only if the target is not inside a short viewport. |
| The page lays out for a narrow window (a collapsed menu, a hidden column) | No: the control is not there. |
| The target sits in a container that does not scroll and clips it | No. |

The click then reports done, or fails with a bare error, and does not say why.

## 2. Decision: one mechanism, two ways to ask

**Recommendation: a `zoom` action backed by DevTools device-metrics emulation on the pane's own browser, not by Chromium's page zoom.** One mechanism covers both ideas in the task: a zoom factor is a viewport `paneWidth/f × paneHeight/f` CSS pixels scaled by `f` into the pane; "a desktop viewport scaled into the pane" is the same thing asked by width.

Why not Chromium's page zoom (`SetZoomLevel`, what Ctrl+Wheel uses today, `browser_panes/zoom.rs`): it lives in the profile's `HostZoomMap`, keyed by the site. An agent zoom would change the person's zoom for that site when the pane uses `personal`, would be shared by the agent's other panes on that site, would outlive the pane, and would change `devicePixelRatio`, which some sites read. Emulation is per browser and ends with the pane's DevTools session, so it touches no profile state. CEF keeps the DevTools client it uses for `ExecuteDevToolsMethod` attached for the browser's lifetime; that is the one assumption to confirm in the first live run (an override set in one call still in effect in the next). If it does not hold, the host re-applies the override before each Browser call.

| Option | Fixes cases 1 to 3 | Costs |
|---|---|---|
| Page zoom (HostZoomMap) | Yes (zoom out also widens the layout viewport) | Person's zoom changed, persists, shared across panes, changes devicePixelRatio. |
| Emulated viewport, by factor | Yes | The page reflows once, like a window resize; text is smaller in screenshots at low factors. |
| Emulated viewport, by width | Yes, and the only one that picks the layout (a 1280-wide desktop page in a 600-wide pane) | Same. |
| Neither | Case 4 (a clipped container) | The click must say so (§4). |

## 3. The control

`Browser zoom` on a pane the agent drives (also on a pane handed to it; the person's own panes are refused, as for other actions):

- `factor`: 0.25 to 2.0 (or `"50%"`). `width`: a desktop viewport in CSS pixels, 320 to 3840, scaled to fit the pane. `fit`: zoom out until the page's width fits the pane (or, with `ref`, until that element is fully inside it), never below 0.25. `reset`.
- The answer says the layout viewport and the factor (`viewport 1280x720 CSS, shown at 47%`). While the factor is not 100%, **every** Browser answer for that pane ends with the same line, so a stale zoom is never a surprise.
- Per pane, not per site. Reset when the pane closes, is released or handed to the person, or the owner changes. The pane header shows the factor as a small badge; clicking it resets. Nothing is written to the profile; the person's own zoom is unchanged.
- The emulated size follows the pane: the host recomputes it when the pane is resized or the window moves to a screen with another scale (the pane's layout code already reports both), so a resize never leaves the page at a stale width.
- Container agents get it unchanged: it is a MuxAPI action that neither writes nor executes on the host.

## 4. A click that cannot land says why

In `CLICK_POINT`, after the scroll: take the target's rect, intersect it with the viewport and with every clipping ancestor (overflow other than visible); if nothing is left, the answer is `not clicked: <name> is outside the visible area by 340 px (clipped by <container>)`. If something else is at the centre of what is visible (`elementFromPoint` is not the target or inside it), try the centres of the visible part's four quadrants; if all are covered, `not clicked: covered by <tag "text"> (a sticky header?)`. Both end with `try Browser zoom (fit, or factor 0.5)`. A click that lands on an ancestor or descendant of the target stays a click. Never a silent miss. This is new and does not overlap the Browser pass 1 spec (which is about what changed after a click); it ships in the same PR as the control.

## 5. Coordinates

- Refs, clicks and snapshot rects are CSS pixels of the page's own viewport; DevTools applies the emulation scale to input and to `getBoundingClientRect` consistently, so ref clicks need no conversion at any factor.
- `UI screenshot` captures the pane's native pixels. Its answer already states a scale (pass 1 spec §2.2); it now states `factor × device scale`, so a point read off the image converts to CSS pixels by dividing by it.
- AgentMux's own chrome zoom changes the pane window's size in device pixels, not the page: the emulated viewport is computed from the pane's current size in CSS pixels every time (pane size ÷ device scale ÷ factor), so the two never multiply into each other.

Tests: unit tests for the viewport arithmetic (pane size, device scale, chrome zoom, factor, width, fit, the 25% to 200% clamp), for the click reasons on a fixed DOM (clipped by a container, covered by a sticky bar, outside by N px, a covered point rescued by another quadrant) and for the answer line. One live run against a local page in a `task dev` build: a tall fixed dialog in a small pane, a sticky footer over a button, a page that hides its menu under 700 px; `fit` and `width 1280` make each reachable, a ref click at 50% and at 200% with chrome zoom at 125% lands on the right button, a screenshot scale reads back to the right CSS pixel, and the person's zoom for the site is unchanged afterwards.

## 6. Where, and questions

Host: a `zoom.rs` beside `act.rs` (the emulation call, the per-pane state, the resize hook), `CLICK_POINT` and its caller (§4); MuxAPI: the action and its description in `crates/mcp`, the note appended to answers; frontend: the header badge. One PR for the control and one for §4 if review prefers.

1. **`fit` and `width` defaults:** `width` without a number means 1280. Is 1280 the right desktop default?
2. **Hand-off:** zoom resets when the person takes the pane over. Should it instead stay until they reset it (they asked for the pane to show everything)? Recommendation: reset, because a person usually wants the real size back.
