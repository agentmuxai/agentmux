# Spec: Agents place their panes: open floating (browser included), with a size and position, then resize, move and dock them

**Date:** 2026-11-22
**Author:** Loap@narko
**Type:** Spec (task #48507; the build is #48508 and starts after the operator approves this)
**Status:** active — approved 2026-11-22 with the recommendations of section 4; built in three phases. Phase 1 (options on opens, answers) is merged (PR #8144). Phase 2 is built in two PRs: 2a (`resize`, `move` and `place`, and `floating` and `rect` in `Pane layout`) and 2b (`dock`, `float`, the handoff default and its setting, the stop behavior of choice 4, docked `size` and `share`). Phase 3 (screenshot helper, docs) not started.
**Requested by:** the operator (2026-11-22): agents should open panes, a browser above all, as floating panes, with a size and a position they can set, and be able to resize and move them afterwards. It should also make "the layout holds at this size" checks repeatable.
**Related:** `SPEC_FLOATING_PANE_ALWAYS_ON_TOP_2026_09_27.md`, `SPEC_FLOATING_PANE_REDOCK_TO_ORIGIN_2026_11_03.md`, `SPEC_FLOATING_PANE_SOURCE_WINDOW_2026_10_13.md`, `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md`.

## 1. Where things are today (read at origin/main `4bd34bc49d`)

- A floating pane is its own chromeless OS window holding one pane in a workspace of its own. srv cannot open windows. `Pane open` with `floating: true` (editor, media, files, the views) calls `open_pane_floating` (`crates/srv/src/server/app_api/pane.rs`): it creates the block, tears it into a new workspace with the `tear_off_block` saga, and publishes `openfloatingpane` (block id and workspace id only) to the source window. That window's frontend (`frontend/app/store/global.ts`) picks the bounds itself (about half the window, centered, 600 to 1200 wide) and calls the host `open_floating_pane_window`.
- The host already takes x, y, width and height, clamps them to the work area of the monitor (`clamp_rect_within`) and handles display scaling (Windows scales a DIP size by the destination monitor's DPI). The agent path never passes them. The host also has `get_window_rect` and `set_window_rect` by window label (used by the floater's edge resize).
- `Browser open` and `Terminal open` take only a split direction. `Agents open` and `Agents spawn` place a pane by the rules of #48341 (parked). Neither has a floating option.
- No agent action resizes, moves or docks a pane. `Pane layout` is read-only and returns no size, position or floating state.
- Ownership exists only for browser panes: `browser_owner` (an in-memory map written after the caller's signed identity is checked, mirrored in the block's `browser:owner_agent` meta for the badge, rebuilt after a restart from `browser_pane_records`).
- Redock is a saga (`RedockFloatingPane`) that puts the pane back where it was torn from when that place can still be found, else to the right of the target tab's last pane (`floater:origin` in the block's meta).
- Always on top is a per-floater tack the person sets in the floater's header (Windows phase shipped; macOS and Linux phase not).

## 2. Requirements

### 2.1 One placement option on every open an agent can make

`Browser open`, `Pane open` and `Widget open` take the same flat arguments (`Terminal open` is a shell drawer of the agent's own pane, not a pane, so it has nothing to float; the build found this reading the code). `Pane open` takes them for the views it opens by file (editor, media, files); a view it opens by deep link keeps the link guard's rules and is not floated by this build:

| Argument | Meaning |
|---|---|
| `floating` | true: open as a floating pane. Absent: today's behavior, except where section 4 choice 1 says otherwise. |
| `width`, `height` | Size in CSS pixels (a number), or a share of the work area (`"60%"`). Either alone is allowed; the other takes the default. |
| `spot` | A named place in the work area: `center`, `right`, `left`, `top`, `bottom`, `top-right`, `top-left`, `bottom-right`, `bottom-left`. |
| `x`, `y` | Exact position in CSS pixels from the top-left of the work area. Wins over `spot` when both are given (the refusal text says so if only one of x, y is given). |

- The work area is the one of the monitor the caller's own window is on (the person's window, as `Pane open` resolves the caller's tab today). An agent never needs to know the screen.
- A value outside the work area is clamped, never refused. Every floating open and every move or resize answers with what it actually got: `placed: { x, y, width, height, spot?, clamped: bool, display_scale }`, in the same units as the arguments.
- Window sizes below the pane minimum (about 320 x 200) are raised to it and reported as clamped.
- `Agents open` and `Agents spawn`: not changed by this spec. The `placement` argument that #48341 adds to them (`tab`, `split`) gets one more value, `floating`, only if the operator wants agent panes to float; it is left out of this build and the schema reserves the name.
- Several floating panes an agent opens without a position cascade: each is offset 32 CSS pixels down and right from the last one still open, wrapping when it would leave the work area.
- An agent may have at most 4 floating panes open at a time (to stop a pile-up over the person's work). The fifth is refused with "dock or close one first" and the list of the four.

### 2.2 Acting on panes the agent opened

New `Pane` actions, each taking `target_id` (a pane the caller opened) and answering with `placed` as above:

- `resize`: `width`, `height`.
- `move`: `spot` or `x`, `y`.
- `place`: size and position in one call (avoids a visible two-step).
- `dock`: put a floating pane back into the layout. Optional `split` (`right`, `left`, `down`, `up`) docks beside the caller's own pane; absent, it goes where the redock rule says (the tab it came from).
- `float`: take a pane the caller opened that is docked and float it, with the same placement arguments as an open. Cheap: it is the existing tear-off saga.

All five act on one pane at a time; none touches a pane the caller did not open (section 2.4).

### 2.3 `Pane layout` reports the result

Each pane in the `layout` answer gets `floating` (bool). A floating pane also gets `rect` ({ x, y, width, height } of its window, in the units of 2.1, plus `monitor` index) and `always_on_top`. A docked pane gets `size` ({ width, height } in CSS pixels, as drawn) and `share` (its fraction of the tab). The docked pixel size needs one measurement round trip to the window's frontend; if the window is hidden or starting, the pane reports `share` only and `size: null`, and says why.

### 2.4 Guard rails

- **Ownership.** An agent places, resizes, moves, floats and docks only panes it opened. The authority is a generalized opener record, written by srv after it checks the caller's signed identity, for every pane an agent opens through any of the tools above, with the same properties `browser_owner` has: a client that writes the meta mirror gains nothing; a duplicated pane has a new block id the record does not know; the person's Take over ends it for good; deleting the block drops it; after a restart it is rebuilt from a stored record. The existing browser map becomes one case of it, so the `Browser*` and `UI*` tools keep their rules unchanged.
- **Never the person's focus or caret.** A floating pane an agent opens appears without being activated: the window is shown inactive (no foreground change, no caret move). A `move`, `resize` or `place` never raises or activates it either. The person clicking it activates it as any window.
- **Rate limit and refusals.** The `Pane open` limit (ten a minute) covers floating opens and the new actions together. A refusal says what the caller can do instead (for a pane it did not open: "you can place only panes you opened; ask the person to move it, or open your own").
- **Container agents** get all of it: the calls go agent to srv to host window, with no host execution by the agent.
- **No modal.** Nothing here asks for a confirm. A person who dislikes a floater closes or docks it with the existing header controls.

### 2.5 Always on top

Per section 4 choice 3. Whatever is chosen, the person's tack in the header always wins over the agent's setting, and an agent can neither set nor clear the tack in this build (not requested).

### 2.6 What happens to a floating pane when its agent's turn ends or the agent stops

Per section 4 choice 4. In every variant: the person's own panes never move, and a pane the person has touched (typed in, resized by hand) is left where it is.

## 3. How it fits together (design, for the build)

- **The ack path.** srv cannot reach a window, so the floating open keeps the existing event (`openfloatingpane`) but it now carries the requested placement (`width`, `height`, `spot` or `x`/`y`, `activate: false`). The source window's frontend passes it to the host, which resolves the spot and the percentages against the work area and the DPI it already knows, clamps with `clamp_rect_within`, opens the window inactive and returns the final rect. The frontend acks srv with it (the same request, wait and ack shape `notify.reveal` uses, timeout 5 s). If the window is hidden or still starting, the call answers `queued`, as `Pane open` does now, and the placement is applied when it opens.
- **Resize, move, dock.** `resize` and `move` are srv asking the host to `set_window_rect` for the floater's window label. The label is known from the floater list the host keeps (label to pane id); srv keeps the label it was acked with and re-reads the list after a restart. `dock` and `float` call the existing `RedockFloatingPane` and `TearOffBlock` sagas with the caller's own tab as the target.
- **Browser panes.** `Browser open` with `floating` goes through `open_pane` with `floating: Some(true)`; ownership is recorded after the open exactly as now. The build verifies that the `Browser*` tools still find the pane after it moves workspaces (the block id does not change).
- **Units.** Agents always speak CSS pixels relative to the work area. Windows' physical-pixel coordinates are converted in the host, never exposed.

## 4. Choices for the operator (each with a recommendation)

**Decided 2026-11-22: all four as recommended.** (1) A browser an agent opens to hand something to the person floats by default, and a setting lets the person make every agent browser float (default off). (2) A floating browser is 1040 x 760 CSS pixels (at most 90% of the work area) at spot `right`, vertically centered; other views keep today's default. (3) Agent-opened floaters are not always on top. (4) A floater stays when the agent's turn ends and docks into the tab it came from when the agent stops or is closed. The options and reasons follow.

1. **Does an agent's browser float by default?** Options: (a) every agent browser open floats; (b) only a browser opened to hand something to the person (a sign-in, a consent step) floats, everything else stays docked; (c) a setting the person chooses, default off. A floating window sits on top of the person's work, so it should be a deliberate step. **Recommend (b), with (c) added** so the person can make it (a) once they have tried it. The `floating` argument is always honored whatever the default.
2. **Default size and spot of a floating browser (and of the other panes).** **Recommend** a browser at 1040 x 760 CSS pixels (fits a sign-in page and most desktop layouts; shrinks to 90% of the work area on a small display), spot `right` (flush right, vertically centered), so the person's left side stays visible. Other views: the current default (about half the window, centered), unless the agent gives a size.
3. **Always on top for agent-opened floaters?** Options: (a) not on top (ordinary windows; the person can tack one); (b) on top within AgentMux, like a tacked floater. **Recommend (a)**: an agent-opened window that cannot be covered by the person's own work is the thing most likely to annoy; the tack stays one click away.
4. **When the agent's turn ends, or the agent stops.** Options: (a) the floater stays until the person closes it; (b) on turn end it stays, and on the agent stopping or being closed it docks into the tab it came from; (c) it closes when the agent stops. **Recommend (b)**: a floater is never left over the person's work with nobody behind it, and nothing the person was reading is lost, since docking closes nothing.

## 5. What this does for resize tests, and what it does not

- **Covered.** In an agent's own task dev build, one `resize` call sets an exact size, repeatably, on all three platforms, with no real mouse: "the layout holds at 900 x 600" becomes a call plus a screenshot. A small helper in `scripts/ui-screenshots/` (next to `sizes.mjs`) takes a list of sizes, resizes a floating pane to each in turn and takes a screenshot of each, naming the files by size.
- **Not covered.** The drag itself: grabbing an edge or corner, snapping, the redock hover, the cursor shapes, and anything that depends on the pointer path. Those still need a real-mouse run on a testbed host. `set_window_rect` also sets the outer window; a floater is chromeless, so its inner size equals it, but a docked pane's size is a layout result and can only be checked by its measured `size`.

## 6. Build plan

- **Phase 1 (the options):** the placement arguments on `Browser open`, `Pane open` (file views) and `Widget open`, the event field, a work-area resolution in the window, the inactive show on Windows, the ack, `placed` in the answers, cascade and the cap, the opener record, tool schemas. A floating browser opened with `floating: true` is 1040 x 760 flush right.
- **Phase 2 (act and report), added to it:** the choice 1 default itself (a browser handed to the person floats, and the setting that floats every agent browser), because a handoff happens on a pane that is already open and so needs the `float` action; the inactive show on macOS and Linux if the platform look check shows the window takes the foreground.
- **Phase 2 (act and report):** `resize`, `move`, `place`, `dock`, `float`; `Pane layout` rects, sizes and floating state; the stop behavior of choice 4.
- **Phase 3:** the screenshot-at-sizes helper and its notes; agentmux-docs (MuxAPI pages, a "floating panes for agents" page) in the release that carries it.
- **Phase 2a, as built:** `Pane` `resize` (width and/or height; keeps the position), `move` (`spot`, or `x` and `y`; keeps the size) and `place` (any of them) act on a floating pane the caller opened, through `POST /api/v1/pane/place`. srv checks the opener record (`pane_openers`), then sends `placefloatingpane` to the floating window itself, which resolves the request against its own monitor's work area (`floater-placement.ts`), sets its rect through the host's `set_window_rect` (no activate) and answers with `pane.floatplaced`. The window's answer is the call's answer (`placed`, with `clamped`); a window that does not answer in 5 s gives `NO_ANSWER`. `Pane layout` marks each floating pane an agent opened with `floating: true` and `rect` (asked of its window, 2 s; `rect: null` with `rect_unavailable` when it cannot say). Floating opens and the new actions share the ten a minute of deep-link opens. Left to 2b: `always_on_top` in the rect (srv does not hold the tack), `floating` for panes the person tore off, and a docked pane's `size` and `share`.
- One PR per phase. Visible changes (a floater appearing, its default place) get the macOS and Linux look checks, and the Windows one on narko's behalf from a testbed.

## 7. Tests the build adds

- Placement resolution (spots, percentages, x/y wins, clamping, minimum size, small display, DPI) as pure functions with a table of cases per platform's work area.
- Opener record: set on every agent open, absent on the person's, not forgeable from the meta, dropped on delete and Take over, rebuilt after a restart, a duplicate unknown.
- Each action refused on a pane the caller did not open, with the stated text; accepted on one it did; the cap of 4; the rate limit shared.
- The open answers `placed` with `clamped` true when it had to clamp; `queued` when the window is not up.
- Cascade offsets and wrap.
- Choice 4 behavior for whichever option is chosen (agent stop docks to the tab it came from; the person-touched pane is left).
- `Pane layout`: `floating`, `rect`, `size`, `share` for docked and floating panes; `size: null` with a reason for a hidden window.
- Frontend: the `openfloatingpane` handler passes the placement and never activates.
- Host: `open_floating_pane_window` with `activate: false` leaves the foreground window unchanged on Windows, and the platform equivalents on macOS and Linux (a testbed run, not a unit test).

## 8. Open points the build must settle (checked in code, not run)

- **Inactive show on each platform.** Whether `open_floating_pane_window` can show without activating on Windows, macOS and Linux today (it may activate on create); if one platform cannot, the spec's "never takes focus" is met by returning focus to the previous window right after, and the look check says whether that flickers.
- **Ack across a hidden window.** The window that receives `openfloatingpane` is the caller's own; if it is minimized, the open should still work (the floater is its own window), which the build checks.
- **Where the opener record is stored** so it survives a restart: the existing browser pane records table, generalized, or a new small one. The build picks the smaller change.
