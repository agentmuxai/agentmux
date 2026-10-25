# Report: the agent pane's auto-collapse, leaving the pane, and what arrives while you are away

**Date:** 2026-10-21
**Status:** analysis; §5.2 (E3-E5) is built (§9), and E7, E9 and E12 are built (§10). E10 and E17 are not. Read from source on `main` @ `f54e04a31`; nothing here was reproduced in a running build (§8).
**Author:** QTest (qtest)
**Trigger (repo owner, 2026-10-21):** the auto-collapse "needs to be more intelligent". Last time it was fixed so that leaving the pane does not collapse the previews. "I think we need an additional check so if there is new input while away, it also doesn't collapse. We need to explore all edge cases."
**Builds on:** `docs/reports/REPORT_AGENT_PANE_HELD_TOOLS_COLLAPSE_ON_RETURN_RCA_2026_10_17.md` (Naki #2, fixed in #5059). Read its §1–§3 first; this report does not repeat them.

---

## 1. Summary

- **The rule, as built:** a tool row (or a non-sensitive jekt) that finishes (arrives) *while the pane is shown* is held open (`documentState.heldOpenNodes`). It collapses only after the reader has seen it scroll off the top.
- **What #5059 fixed:** leaving the pane no longer releases holds. Rows held when you left are *carried* on return and are released only after they have been seen again in this visit.
- **What it did not cover, and did on purpose:** a row that **finishes or arrives while the pane is hidden is never held** (`AgentDocumentView.tsx:165-166`, `holdNodeOpen` returns while dormant). It comes up closed. That choice was made to keep a hidden pane's held set bounded for memory (#4847).
- **So the gap is this:** you leave; the agent keeps working or a jekt comes in; you return; the rows that finished or arrived in between are collapsed. A tool that was *running and open* when you left, and finished while you were away, is the most visible case. This is the "new input while away" the owner describes, on the reading that "input" means new rows (tool results, jekts, new turns).
- **Recommended fix (§5):** remember only the *ids* of rows that finished or arrived while the pane was hidden (cheap, capped). When the pane is shown again and its layout has settled, hold those that are **on screen**; anything above the viewport collapses, as it would have if you had watched it scroll off. No growth in held text while you are away, and it matches "a row collapses only because you saw it leave".
- **Other edge cases found (§4):** a fixed 400 ms "settled" window that can mark a row seen too early on a heavy pane (E7); a covered or locked-screen window that may not count as hidden (E9; a minimized one does, since #5172); holds lost on reload, restart and upgrade (E12); roll-off to History after the next send (E10); a held row's text failing to read back (E11).

## 2. How the pane decides a row is open

| Mechanism | Where | What it does |
|---|---|---|
| Open/closed rule | `virtualization/disclosure.ts` `rowDisclosure` | Tool: open if pinned, running/awaiting approval, or held (not if dismissed). Jekt: open if pinned or held; a `sensitive` jekt is always open. Content-first tools (WebSearch) open until the user collapses |
| Hold on finish or arrival | `ToolBlock.tsx:194-217` (`holdIfFinishedOnArrival`, active→inactive), `AgentDocumentView.tsx:165-175` `holdNodeOpen` | Adds the id to `heldOpenNodes`. **No-op while the pane is dormant** |
| Live-arrival test | `virtualization/live-arrival.ts` | A row stamped within `LIVE_ARRIVAL_WINDOW_MS` (5 s) counts as arrived live (a fast tool drawn already finished) |
| Release scan | `virtualization/held-release.ts` `useScrolledOffRelease` → `heldRowsToRelease` | Releases a held row whose box is above the scroller's top (or not rendered). Runs from pin passes and user scrolls **only while following the bottom** (`AgentDocumentVirtualList.tsx:998, 1039, 1114, 1261, 1285`). Skipped when dormant or when the scroller is 0×0 |
| Return handling | `held-release.ts` `carried`, `RETURN_SETTLE_MS = 400`, `CARRY_MAX_MS = 60 000` | On show, the held set is *carried*: released only once seen on screen after the settle, then scrolled off; or after 60 s shown and still off screen |
| Held text unload | `hooks/useLiveFeedRollOff.ts` (`HIDDEN_HELD_UNLOAD_MS = 30 000`, `UnloadToolResults keepIds`) | After 30 s hidden, held rows' *text* may leave memory; the row stays open and reads its text back on show (`ToolBlock.tsx:288-293`) |
| Roll-off to History | `useLiveFeedRollOff.ts` `runRollOff`, `RollOff` | Triggered by a new last `user_message` (the next send), the pane hiding, turn end, byte/row budget. Older turns leave the live pane for History (rows there are closed). Default `LIVE_FEED_DEFAULT_TURNS` is unbounded, so in practice the byte/row budget decides |
| User state | `pinnedNodes`, `collapsedNodes` | Survive hiding (the view is kept alive). Pins beat everything |
| Not saved | `useSnapshotPersistence.ts:166` | `heldOpenNodes` is not in the pane snapshot |

`dormant` (`agent-dormancy.tsx`) is "mounted but not visible": a hidden pane-stack member or a window tab not displayed. It is **not** "visible but not focused", and not obviously "window minimized".

## 3. What "new input while away" can mean

The owner's phrase has three plausible readings. The fix in §5 covers all three, but they differ in what the user sees, so it is worth confirming which one the owner has in mind.

1. **New rows from the agent while away:** tool calls that finish after you left, including one that was running when you left.
2. **An incoming jekt, swarm broadcast or message while away:** a `jekt_message` row (collapsed one-liner unless `sensitive`) and the turn it starts.
3. **A new user turn while away** (a send from elsewhere): triggers a roll-off pass (`useLiveFeedRollOff.ts:198-210`) and a new turn's worth of rows.

## 4. Edge cases

Verdicts come from reading the code and its tests, not from running them. **Collapses** means the user would see the row closed that they would expect open.

| # | Scenario | Verdict today | Why / where |
|---|---|---|---|
| E1 | Row open (held), you leave, nothing else happens, you return | Stays open | #5059; carried until seen |
| E2 | Row open (held), you leave, the agent keeps working, you return | Stays open (carried); row may sit above the viewport | Re-pin pushes it up; not released until seen, or 60 s shown |
| E3 | Tool **running and open** when you leave; it finishes while you are away | **Collapses** | `holdNodeOpen` no-op while dormant; row is closed by default. No transition was "seen" |
| E4 | Tool starts and finishes entirely while you are away | **Collapses** | Same. Arguably right (you never saw it), but it is what the owner sees on return |
| E5 | Non-sensitive jekt arrives while you are away | **Collapses** (one-line row) | Jekt hold goes through the same `holdNodeOpen`. `sensitive` jekts stay open |
| E6 | Row arrives while you are in the pane but scrolled up (not following) | Held; stays open | Hold is added (pane active); scan only runs while following. When you return to the bottom, it scrolls off and collapses as designed |
| E7 | Heavy pane: catch-up render on return takes longer than 400 ms | **Possible early collapse** | `settled` is a fixed `RETURN_SETTLE_MS`; a carried row on screen at 400 ms mid-catch-up is marked seen, then pushed off by the pin and released. `settle-on-show.ts` says the cover lifts "under" 400 ms; nothing proves layout is done |
| E8 | Window tab switch, both modes (`display:none`-like and laid-out `visibility:hidden`) | Handled by dormancy | RCA §3. The `content-visibility` case was "probably" fine and never verified |
| E9 | Window minimized, covered, or the OS screen locked | Probably handled; not verified | #5172 (2026-10-19) made the dormancy that `holdNodeOpen` and the release scan read (`agent-view.tsx`, `useRenderDormant`) also true when the page is hidden (`document.hidden`). So the scan is skipped and no hold is made, as for a hidden tab. Two loose ends: the 30 s held-text unload timer reads the pane's `hidden` only, so a minimized window never unloads held text; and `document.hidden` may not flip for a window that is merely covered or on a locked screen |
| E10 | New send (user message) while the pane holds more than its budget | Older turns roll off to History; their open rows appear closed there | `scheduleRollOff` on a new last `user_message`; held rows are not in `keepIds` (only pins are). Not a collapse, a move; the user may read it as one |
| E11 | Held row's text was unloaded after 30 s hidden and the read-back fails | Row stays open with a "failed" note, retried on re-open | `ToolBlock.tsx:290-297, 666`. Height is preserved (`unloadedBodyPx`), so no jump |
| E12 | Reload, restart, **upgrade** | **All holds lost** | `heldOpenNodes` not saved. Within a session the view is kept alive, so this only bites on restart; an upgrade restarts everything |
| E13 | Two windows or panes of the same agent | Independent holds | `createAgentAtoms()` per view (`agent-view.tsx:208`). Seeing a row in one pane does not release it in the other. Not a bug; note it |
| E14 | Row pinned by the user | Never released | "pin wins" in `heldRowsToRelease` |
| E15 | A dismissed (denied/canceled) tool | Never held open | `rowDisclosure` |
| E16 | Pane re-shown, user scrolls up at once | No release while not following | Scan gated on `stickToBottom`. `CARRY_MAX_MS` only counts while the scan runs |
| E17 | Carried row never seen, pane shown for 60 s | Released silently while off screen | By design (`CARRY_MAX_MS`). When the user scrolls up later it is closed. Consistent with "had you been watching", but it can contradict "appears as it did" |
| E18 | Resume of a restored/background pane that mounts hidden | No holds, rows closed | A loaded transcript has no stamps; fine |
| E19 | Focus is in another *visible* pane (split layout) | Behaves as watched: rows collapse as they scroll off | The pane is not dormant. Right by the rule, but the reader may not be looking at it |
| E20 | Very long absence: hundreds of tools | All finished-while-away rows closed | Bounded and cheap; the opposite problem if we hold them all (§5.3) |

**The load-bearing gaps are E3–E5 (the new check the owner asks for), E7 (a way the existing fix can still fail), E9's loose ends, and E12 (everything lost on upgrade).**

## 5. Recommendation

### 5.1 The principle

A row collapses only because the reader **saw it leave the screen**. Applied to arrivals while away: a row that finished or arrived while you were away has not been seen leaving, so on return it stays open if you would have seen it on screen, and closed only if it would already have scrolled off above the viewport.

### 5.2 The check

- While the pane is dormant, `holdNodeOpen` (and the jekt arrival path) records the **id only** into a capped set, `awayArrivals` (FIFO, cap about 200 ids; no text, no row held). Nothing is held, so #4847's bound is unchanged: held text does not grow while you are away.
- On show, after the layout has settled, every id in `awayArrivals` whose row is **on screen** is held (`holdNodeOpen`), the set is cleared. Rows above the viewport are not held: they collapse exactly as they would have had you watched.
- Rows held before you left keep today's carried behaviour (E1, E2), so the two sets are complementary: *held-before* are carried until seen, *arrived-while-away* are held if on screen on return.
- A running tool that is still running on return is open anyway (`status.active`).
- Sensitive jekts stay open as today. Pins and user collapses always win.

This needs no new state in the snapshot and no per-row timestamps (a tool's stamp is its call arrival, not its finish time, so it cannot tell "finished while away").

### 5.3 Alternatives

| Option | Effect | Why not |
|---|---|---|
| **A. Hold every row that finishes while away** (remove the dormant guard) | No collapse for anything that arrived | Reintroduces #4847's growth: a 12-agent overnight run holds hundreds of open rows per pane. Would need a cap, and a cap-by-count is arbitrary |
| **B. Ids while away, hold the on-screen ones on return** (recommended) | Matches what a watcher would have seen | Needs a reliable "layout settled" signal (E7) |
| **C. Hold the last N finished rows** | Simple | Arbitrary N; may hold rows far above the viewport, or too few |
| **D. Change nothing, tell the user** | None | The owner has asked for the behaviour |

### 5.4 Hardening that belongs with it

- **E7:** replace the fixed 400 ms with "the show cover has lifted **and** `scrollHeight` has been unchanged for two animation frames" (with the current 400 ms as the upper bound). Both the carried-seen test and the new on-screen hold use it.
- **E9:** #5172 already covers a minimized window for the scan and the hold. Check (a) a covered or locked-screen window, and (b) whether the held-text unload timer should use the same signal, so a minimized agent window stops pinning held text.
- **E12:** decide whether holds should survive an upgrade or restart. The pane snapshot already saves the scroll position; saving the *held ids* (capped, ids only) would make an upgrade restore the open rows. Loap-3 argued against it in #5059 because the set changes on every scroll; saving on hide and on quit only would avoid that churn. Needs the owner's call.
- **E10:** pass `heldOpenNodes` as `keepIds` to the roll-off when the budget allows, or move an open row's open state into History's view of that turn. Lower priority.
- **E17:** keep, but show nothing; or raise `CARRY_MAX_MS`. Needs the owner's call.

### 5.5 Tests

- `AgentDocumentView.hold.test.tsx`: a tool that finishes while dormant is recorded in `awayArrivals`, not held; the set is capped.
- `held-release.test.ts` (pure): given `awayArrivals`, rects and a viewport, the on-screen ones are returned to hold, the ones above are not.
- `AgentDocumentVirtualList.collapse.test.tsx`: leave, finish a running tool, return, settle → the row is open; leave, finish 30 tools, return → only the on-screen rows are open; a row held before leaving and one that finished away both survive a pin.
- Jekt arrival while away: a non-sensitive jekt on screen on return is open; a sensitive one always is.
- E7: a catch-up that lands after 400 ms does not mark a carried row seen.
- E9: with `document.hidden` true, the scan does not run and nothing is held (#5172 tests dormancy; add one through the release scan); and the unload timer follows it, if that is changed.

## 6. Questions for the owner

1. Which "input" did you see collapse: tool calls that finished while you were away, a jekt/broadcast that came in, or a new turn? (§3; §5.2 covers all three.)
2. Should a row that arrived while you were away and is **above** the viewport on return be open too? Option B says no (it would have scrolled off); the alternative is "open until seen", capped.
3. Should previews survive an **upgrade or restart** (E12)?
4. Is the 60 s `CARRY_MAX_MS` (E17) the right cut-off?

## 7. Related work and coordination

- #5059 and the RCA are Naki #2's. This change sits in `AgentDocumentView.tsx`, `ToolBlock.tsx`, `virtualization/held-release.ts` and `hooks/useLiveFeedRollOff.ts`, so Naki #2 should be asked before it starts. Loap-3 reviewed #5059 and owns #4847.
- Camper's follow state machine and the one-way flow (`scroll/one-way-flow.ts`) drive the same pin passes (the RCA §7 notes this).
- No open PR touches these files (checked 2026-10-21).

## 8. What was not verified

- Nothing was run. E3–E5 and E7 follow from the code and the RCA's own §8 ("no hold while hidden"); they should be reproduced in an isolated `task dev` build before the fix: open a pane, let a tool run, switch pane tab, let it finish (and 30 more), return; repeat for a window-tab switch, a minimized window, and a jekt.
- E8's `content-visibility` window-tab case and E9 are unverified by the RCA and by this report.
- Which of the three readings in §3 matches what the owner saw.

## 9. What was built (2026-10-22)

Option B (§5.2), agreed with Naki #2 and Loap-3 before the change. Both asked for: ids keyed by the row's node id; the set cleared on the show and on remount; nothing left behind for a row that is removed or replaced; and the held set bounded by the viewport, not by the id cap.

- `virtualization/held-release.ts`: `AwayArrivals` (ids only, newest kept past `AWAY_ARRIVALS_MAX` = 200, taken and cleared as one) and the pure `awayRowsToHold` (a row is held only if it is rendered and intersects the viewport, and isn't already held or pinned).
- `AgentDocumentView.tsx`: `holdNodeOpen` while dormant now records the id (tools and non-sensitive jekts both come through it) instead of dropping it. Nothing is held and no text is kept while hidden, so #4847's bound is unchanged.
- `useScrolledOffRelease` judges the ids once the show has settled (`RETURN_SETTLE_MS` and the show cover lifted, at most `AWAY_APPLY_MAX_MS` = 3 s), then holds the ones on screen through the existing `onHoldNodeOpen`. A row held this way reads its text back through the existing load-when-open path and becomes unloadable again when released. If the pane is hidden again first, the ids wait for the next show; they are dropped on unmount.
- Rows above the viewport, rows no longer rendered (removed, replaced, virtualized away) and a pane reporting no size are not held.
- Tests: `held-release.test.ts` (the cap with 300 rows; 300 tools finishing while away hold only the 40 in view), `AgentDocumentVirtualList.collapse.test.tsx` (held when on screen after the settle; not when above; a gone row ignored; ids kept if hidden again).
- `scripts/check-file-sizes.baseline`: `AgentDocumentVirtualList.tsx` 1806 → 1810 (the prop and the two hook options; the logic is in `held-release.ts`).
- **Not built:** the rest of §5.4 (E7 settle signal, E9 loose ends, E12 holds across an upgrade, E10, E17), which need owner decisions or their own change. **Not reproduced** in a running build (§8): the unit and component tests cover the logic, not the feel of a row opening at the moment you return.

## 10. What was built next (2026-10-25)

- **E7, the settle signal.** `virtualization/show-settle.ts`: after a show, "settled" is the cover lifted and the scroller's `scrollHeight` unchanged for two samples (50 ms apart), at least 400 ms after the show, and at most 3 s whatever the layout does. It replaces the fixed 400 ms (`RETURN_SETTLE_MS`) and the separate 3 s poll (`AWAY_APPLY_MAX_MS`); both the carried-seen test and the away rows' hold use it. A hide before it settles cancels the wait.
- **E9, the unload timer.** Already done by #5172: the pane passes `renderDormant` (tab hidden or window hidden) as `hidden` to `useLiveFeedRollOff`, so a minimized window's held text unloads after `HIDDEN_HELD_UNLOAD_MS`. No change. A window that is only covered, or a locked screen, is still unverified.
- **E12, holds across a restart or an upgrade.** The snapshot now carries `documentState.heldOpenNodeIds`: the newest 40 held ids (`MAX_SAVED_HELD`), ids only, additive to schema 2 (an older build ignores the field). They are written when the pane goes hidden and on `pagehide`, not as the reader scrolls (the churn #5059 argued against). On restore they are held again and also marked `restoredHeldNodes`, so `useScrolledOffRelease` treats them as carried: open until seen on screen in this visit and then scrolled off, or released after `CARRY_MAX_MS`.
- **Not built:** E10 (roll-off after the next send) and E17 (the 60 s cut-off, kept). Still not reproduced in a running build.
- Tests: `show-settle.test.ts`; the collapse test (slow return waits, cap judges, restored rows carried, baseline unchanged); `useSnapshotPersistence.test.ts` (written on hide and on `pagehide`, capped, nothing when none are held); `useHistoryPagination.firstscreen.test.ts` (restored ids, newest kept, non-strings dropped).
