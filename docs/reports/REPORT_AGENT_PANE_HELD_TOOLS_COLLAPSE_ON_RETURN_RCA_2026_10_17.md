# RCA: an agent pane's open tool previews are collapsed when you come back to it

**Date:** 2026-10-17
**Status:** analysis. Root cause identified from source and history. Fixed in the PR that adds this report: §6.1 (with Loap-3's variant, §8), §6.2, §6.3 and §6.5. §6.4 (snapshot) was dropped on Loap-3's advice.
**Author:** Naki #2 (naki-naki-2)
**Trigger (repo owner, 2026-10-17):** "the agent pane has auto-collapsing previews. they are supposed to collapse after the tool call scrolls off the top of the pane once new messages come in. But … if I switch tabs or leave in some way, when I come back, the tool calls are all collapsed, so there is a coupling between scrolling off the top and leaving the pane. If we leave and come back we do NOT want them collapsed, they should appear as they did."
**Code:** `main` @ `49e1b06e2` (2026-10-17).

---

## 1. Summary

- **The intended rule** (#1491, 2026-06-16): a tool row that finishes while you can see it is *held open* (`documentState.heldOpenNodes`). It collapses only once it has scrolled off the top of the pane, as new output arrives below it.
- **Why they collapse when you come back:** three separate paths release the hold because the pane was hidden, not because you watched the row scroll away. One of them was added on purpose eight days ago.
  1. **#4847 (2026-10-14) releases every hold once a pane has been hidden for 30 s** (`useLiveFeedRollOff.ts:198-220`, `HIDDEN_HOLD_RELEASE_MS = 30_000`). This is the main cause: leave a pane for more than 30 seconds, by switching its pane tab or its window tab, and every held preview is collapsed when you return, even if nothing moved. It was a deliberate memory measure (`SPEC_AGENT_PANE_TOOL_OUTPUT_RETENTION_2026_10_13.md` §2 item 3, "a hidden pane has no watcher"), and it is what turned this into a regression.
  2. **The scroll-off scan still runs in a hidden pane** that keeps its layout. The guard against that (#2538, 2026-08-11) only skips a pane whose rect is 0×0, which was the `display:none` era. Since agent pane tabs stay mounted under `visibility: hidden` (#3187/#3391, 2026-09; `pane-leaf-chrome.tsx:414`), and window tabs can too (#3686, "keep inactive window tabs laid out"), a hidden pane has its real size. If the agent keeps working while you're away, it stays pinned to the bottom, and the held rows scroll off its top and are released. This fires within seconds, not after 30.
  3. **Coming back can collapse them by itself.** A hidden pane defers its expensive rendering (`agent-dormancy.tsx`). On return the deferred rows render at full height, the pane re-pins to the bottom, and that pin pass runs the same scan (`AgentDocumentVirtualList.tsx:1029-1038`). It releases the rows you had on screen, at the moment you return.
- **Related (restart and reload):** the pane snapshot doesn't save `heldOpenNodes` (`useSnapshotPersistence.ts:150`), so after a reload or restart every held row comes back collapsed. The other restore work (#4943 snapshot, #4964 scroll position) brings back where you were, but not what was open.
- **Fix (§6):** a hold is released only by scrolling you could see. No mass release on hiding. No scan while hidden. On return, re-baseline, so rows displaced by catch-up stay open. Rows that finish while the pane is hidden aren't held, which keeps the held set bounded for the memory goal of #4847. Optionally save the held set in the snapshot.

## 2. How the hold works today

| Piece | Where | Does |
|---|---|---|
| Hold | `ToolBlock.tsx:194-217`, `AgentDocumentView.tsx:160-176` | A tool that finishes while its row is mounted joins `heldOpenNodes`, so it stays expanded |
| Release scan | `AgentDocumentVirtualList.tsx:1194-1221` (`releaseScrolledOffHolds`) | For each held id: if its element is gone, or its bottom is above the scroller's top, release it |
| Scan triggers | `:1029-1038` (viewport ResizeObserver pin), the content RO pin pass (`:1626-1628`, `prepare`), `:1284` and `:1307` (scroll batches while following), `:997` (no-RO fallback) | Every pin while following, and user scrolls while following |
| Hidden guard | `:1200-1208` (#2538) | Skip the scan when the scroller's rect is 0×0 |
| Hidden release | `useLiveFeedRollOff.ts:198-220` (#4847) | 30 s after the pane becomes hidden, clear `heldOpenNodes` entirely |
| State lifetime | `agent-view.tsx:208` (`createAgentAtoms()` per view) | Lives as long as the view; agent tabs are keep-alive, so the view survives a tab switch |
| Snapshot | `useSnapshotPersistence.ts:150` | Saves collapsed/pinned/filter/scroll, **not** `heldOpenNodes` |

`hidden` in the agent view is `usePaneTabVisibility(blockId) !== "active"` (`agent-view.tsx:155-156`). It is true for a dormant pane-tab member and for a hidden window tab, and it already reaches `useLiveFeedRollOff`, but not the virtual list's scan.

## 3. History

| Date | PR | Change | Effect on this bug |
|---|---|---|---|
| 2026-06-03 | #1257 | Re-apply sticky scroll when a pane returns from hidden | The re-pin on return (path 3's trigger) |
| 2026-06-16 | #1491 | Tool blocks stay expanded until scrolled off the top | The feature |
| 2026-08-11 | #2538 | "stop mass-collapsing on hidden pane": skip the scan at 0×0 | **The earlier fix.** Correct while hidden panes were `display:none` |
| 2026-09-11, 09-18 | #3187, #3391 | Keep terminal and agent tabs mounted across a pane-tab switch, hidden by `visibility` | Hidden agent panes now have a real size, so #2538's guard no longer applies (path 2) |
| 2026-09-15 | #3239 | Inactive window tabs: `display:none` → `content-visibility:hidden` | Descendants aren't laid out; the guard probably still applies here (not verified) |
| 2026-09-22 | (dormancy) | `agent-dormancy.tsx`: a hidden pane defers markdown rendering | Rows grow on return (path 3) |
| 2026-09-24 | #3686 | Optionally keep inactive window tabs laid out (`visibility:hidden`) | Path 2 for window tabs in that mode |
| 2026-09-25 | #3725 | Per-tab keep-alive (agent stays keep-alive) | Confirms agent views survive a switch; state isn't lost by unmounting |
| 2026-10-08 | #4473 | Jekts collapse like tool calls | Jekt rows get the same hold and the same bug |
| 2026-10-14 | #4847 | Bound the tool output a pane keeps: **release holds after 30 s hidden** | **The regression** (path 1) |
| 2026-10-15 | #4943, #4964, #4974 | Pane snapshot saves overlay changes; a reload restores the scroll position | Restore brings back position, not held rows (§1, related) |

So it was fixed (#2538), weakened when hidden panes changed from `display:none` to `visibility:hidden` (#3391), and broken outright by #4847's deliberate 30-second release.

## 4. The three paths, in detail

### 4.1 Path 1: the 30-second release (#4847)

```ts
// useLiveFeedRollOff.ts:198-220
createEffect(on(hidden, (hidden) => {
    if (!hidden) return;
    scheduleRollOff();
    const t = setTimeout(() => {
        ...
        setDocState((prev) => prev.heldOpenNodes.size === 0 ? prev : { ...prev, heldOpenNodes: new Set<string>() });
        scheduleRollOff();
    }, HIDDEN_HOLD_RELEASE_MS);   // 30_000
    onCleanup(() => clearTimeout(t));
}, { defer: true }));
```

- Its reason (spec §2.3): a held row's result is exempt from unloading (`UnloadToolResults keepIds` includes `heldOpenNodes`, `useLiveFeedRollOff.ts:73-76`). In a background pane, every tool that finishes joins the held set (the row is mounted under keep-alive), so the held set, and the memory it pins, grows until the turn rolls off.
- Its comment also says the scan can't run in a hidden pane "(their rect is 0×0)". That's no longer true for `visibility:hidden` panes (path 2).
- A unit test enshrines it: `useLiveFeedRollOff.test.ts:151` ("releases held-open rows once the pane has been hidden for HIDDEN_HOLD_RELEASE_MS").

### 4.2 Path 2: the scan in a hidden, laid-out pane

- An inactive keep-alive slot is `visibility: hidden; opacity: 0` with its real size (`pane-leaf-chrome.tsx:372-430`, by design, so terminals don't need re-fitting).
- The scroller's ResizeObserver and the content RO keep firing as the agent streams. `pinNow` is `h > 0 && stickToBottom()`, true, so it pins and calls `releaseScrolledOffHolds()` (`:1029-1038`).
- The guard at `:1208` checks for 0×0, which isn't the case here, so the scan releases every held row the pin pushed above the top.

### 4.3 Path 3: the catch-up on return

- While hidden, `AgentDormancyProvider` defers MarkdownBlock's paint (rendering only, data flows).
- On return, those rows render at full size, the content grows, and the pane (still following) re-pins. The pin pass runs the scan, and rows that were on screen when you left, now pushed up by the newly rendered content, are released as you look.

## 5. What "appear as they did" requires

- A hold is released only by scrolling the person could see: the pane is active, and the row was on screen in this visit before it left the top.
- Leaving a pane changes nothing about which rows are open.
- Coming back shows the open rows that were open. New rows that arrived while away follow the normal default; they weren't on screen, so they aren't held.
- A reload or restart keeps them too (optional, §6.4).

## 6. Recommended fix

### 6.1 Remove the 30-second mass release; meet #4847's memory goal another way

- Delete the `heldOpenNodes` reset in `useLiveFeedRollOff.ts:198-220`, and update its test to assert that holds survive hiding.
- **Don't hold what nobody saw finish.** In `ToolBlock.tsx`/`AgentDocumentView.tsx`'s hold, add a row to `heldOpenNodes` only when the pane is active. A tool that finishes in a hidden pane isn't held, so its result can unload like any collapsed row. This removes the growth #4847 was fixing, at its source.
- **Let held results unload while hidden, and read them back on return** (optional, if the held set is still too heavy). While hidden, leave `heldOpenNodes` out of `UnloadToolResults.keepIds`. The row stays *open*; only its text leaves memory. Gate `ToolBlock.tsx:284`'s reload-when-open effect on the pane being active, or it reloads at once and undoes the unload. On return, open rows read their result back (one indexed line read each; the retention spec measured p95 under 50 ms).

### 6.2 No scan while the pane is hidden

- Pass the agent view's `hidden` accessor into `AgentDocumentVirtualList` (it already reaches `useLiveFeedRollOff`), and return early from `releaseScrolledOffHolds` when hidden. Keep the 0×0 check as a second guard.
- This covers dormant pane tabs and hidden window tabs in both modes. It doesn't cover a minimized OS window, which `hidden` doesn't track. The 0×0 guard covers that if the rect is zero; check it during the fix.

### 6.3 Re-baseline on return

- Keep a per-visit "seen" set: a held row joins it when the scan finds it intersecting the viewport while the pane is active. The set is cleared when the pane hides.
- Release a hold only if the row is in the seen set and is now above the top, or the person scrolled it there themselves.
- Effect: rows pushed up by catch-up rendering or by output that arrived while you were away aren't released on return. Once you're back and watching, the next time they scroll off they collapse as designed.
- The "no element → release" safety net (`:1215-1218`) becomes "no element and seen", so a row that was never on screen in this visit isn't released by virtualization.

### 6.4 Optional: save held rows in the snapshot

- Add `heldOpenNodes` (or the held rows within the saved scroll window) to the pane snapshot, and restore it in `onSnapshotOverlay` (`agent-view.tsx:425`). A reload or restart then shows the same open rows, matching #4964's restored scroll position.
- Add it to the overlay-dirty tracking (`useSnapshotPersistence.ts:150-158`), so a release marks the snapshot dirty.

### 6.5 Tests

- `useLiveFeedRollOff.test.ts`: a pane hidden for 30 s, or for 10 minutes, keeps its holds. Replace the current test.
- Hold: a tool finishing while the pane is hidden isn't added to `heldOpenNodes`; one finishing while active is.
- `AgentDocumentVirtualList.collapse.test.tsx`:
  - with `hidden` true and a real-size scroller, a pin that pushes a held row above the top doesn't release it;
  - after returning, a pin caused by rows growing doesn't release held rows that weren't seen in this visit;
  - a held row seen on screen and then scrolled off while active is released.
- Snapshot round-trip (if §6.4): held rows survive a reload.
- Manual, in an isolated `task dev`: open a pane, let a few tools finish on screen, then for each of a pane-tab switch, a window-tab switch, minimizing the window and a reload: wait over 30 s with the agent working, come back, and check the same rows are open.

## 7. Notes for the fix

- #4847 was a real memory fix (the renderer-memory plan, item 7). §6.1 keeps its effect without the collapse by not holding rows nobody saw finish, plus optional unloading. Measure the 12-agent memory run from the retention spec again after the change.
- Jekt rows (#4473) share the hold, so the fix covers them too.
- Coordinate with whoever owns the follow controller (Camper's follow state machine, `e580e53`, 2026-10-14, and the one-way flow work, #4438): §6.3 changes the scan's release rule, and the controller's pin passes call it.

## 8. What was built (2026-10-17)

Coordinated with Loap-3 (#4847's author), who agreed with §6.1 and suggested the unloading variant to keep #4847's bound in bytes, not just rows.

- **No mass release.** A pane hidden for `HIDDEN_HELD_UNLOAD_MS` (30 s; it was `HIDDEN_HOLD_RELEASE_MS`) keeps its holds. Instead, the next pass stops exempting held rows from `UnloadToolResults`, so their text leaves memory while the rows stay open (`useLiveFeedRollOff.ts`).
- **Read back on show.** ToolBlock's load-when-open effect waits for the pane to be shown (`ToolBlock.tsx`), so an open row reads its text back on return and not while hidden.
- **No hold while hidden.** `holdNodeOpen` does nothing while the pane is dormant (`AgentDocumentView.tsx`). That covers a tool that started on screen and finished after the pane was hidden.
- **No scan while hidden; re-baseline on return.** The scan moved out of `AgentDocumentVirtualList.tsx` into `virtualization/held-release.ts` (`useScrolledOffRelease`), and it returns at once while the pane is dormant. Rows held when it is shown again are *carried*: they are released only after they have been on screen in this visit, counted once the show has settled (`RETURN_SETTLE_MS`, 400 ms, and the show cover lifted). The rule is a pure function, `virtualization/held-release.ts`.
- **Not done:** saving `heldOpenNodes` in the snapshot (§6.4). Loap-3: it changes as you scroll, which is why `useSnapshotPersistence.ts:150` leaves it out. Within a session the agent view is kept alive, so the held set already survives leaving.
- **Tests:** `held-release.test.ts`; `AgentDocumentVirtualList.collapse.test.tsx` (hidden at full size, carried rows, settling); `AgentDocumentView.hold.test.tsx`; ToolBlock's reload-on-show; `useLiveFeedRollOff.test.ts`, where #4847's release test is replaced by "holds survive, held results unload after 30 s hidden", plus "a hidden pane's unheld finished tools still unload".

**After Loap-3's review of #5059:**

- **No jump while held text reads back.** An open row whose result was unloaded drew a one-line "Loading result…". So after a return from more than 30 s away, a held 40-line preview shrank and then grew back, moving the rows below it. The placeholder now keeps the body's height from the moment its text left (`ToolBlock.tsx`, `unloadedBodyPx`), so nothing below it moves while it reads back.
- **Carried rows don't keep their text forever.** A carried row that hasn't been on screen after the pane has been shown for `CARRY_MAX_MS` (60 s), and is still off screen, is released. That is what would have happened had the reader been watching, and it lets the row's text unload.
- `HIDDEN_HOLD_RELEASE_MS` is renamed `HIDDEN_HELD_UNLOAD_MS`: it no longer releases anything.
