# A window's tab content froze while its pill moved on: an error in a transcript row left reactive effects dead

**Status:** implemented in the PR that adds this document: the row guard (§4.1), the tab-display watchdog (§4.2) and the hidden Swarm pause (§4.3). The throwing `<Show>` itself is not yet identified (§5).
**Date:** 2026-10-25
**Author:** Camper@narko
**Trigger:** the operator's busiest window stopped showing the tab they selected: the tab bar and window title moved, the content stayed on an older tab, every later click behaved the same, and tearing the tab off into a new window was the only way back. Task #3491.
**Evidence:** the window's console lines in the host log, read-only, and a capture of the window; the v0.59.41 frontend bundle for decoding stack frames.

## 1. What was seen

- The pills, the title and `SetActiveTab` all moved on each click, and every `tab-reveal` line named a different target (all `source=ungated`). The content region kept showing one older tab's live panes (not a frozen snapshot: its clocks and streams ran).
- Eleven `Stale read from <Show>.` errors, logged as uncaught, in the 4 minutes before it started; the last cluster at 11:29:44, three of them inside a store write, and the window was stuck by 11:30:22.
- A burst of 768 "egress lane full, consumer stalled" drops on one socket at the same time, then none.
- 144 ResizeObserver loop errors, four forced reveals after 1.5 s, and Swarm panes rebuilding 150 to 384 times a minute: a window whose page could not keep up.

## 2. The egress drops are a symptom

`eventbus.rs` drops an event when a lane is full. 768 drops on one socket cannot be the priority lane (capacity 8192); it fits the background lane (256), which carries sysinfo and per-block stats. What was dropped was telemetry, not workspace or tab updates, and `mos.ts` applies any update with a newer version, so a lost update doesn't wedge later ones. The page was too busy at that moment; that is all the drops say.

## 3. The mechanism

`Stale read from <Show>.` is Solid's own guard, thrown as a string, when the accessor a `<Show>` hands its child is read after the condition went false. In the bundle, the stack reads: Solid's error handler (found no handler, so it rethrows) from `runUpdates`' catch (which sets the queued effects and the queued memos to null) from the loop that runs queued memos, from the store write, from `setNodes`, from the agent document store's dispatch.

So a memo re-ran inside the flush a transcript write started, threw, and nothing above it handled the error. Solid then **discards every effect queued in that flush, in any pane of the window**. Those effects are already marked stale, and Solid only queues clean nodes when a signal changes, so they never run again. Which bindings were queued is luck; in this window it was whatever switches the displayed tab. The pill and title are other computations, so they kept moving. A torn-off tab is a fresh mount, which is why that recovered it.

`DocumentRow.test.tsx` shows it: an unrelated effect queued with a throwing memo ends at `[0]` and never runs again; under a handler it runs on.

## 4. What changed

1. **A row guard.** Every transcript row (`DocumentRow`) is built under `catchError`, so a computation that throws inside a row is handled there and the flush goes on. The handler logs `[agent-row] render error in a <type> row (<id>): <message>`, once per message per 10 s, so the next occurrence names the row.
2. **A tab-display watchdog** (`display-watchdog.ts`, installed in `workspace.tsx`). Every second it compares the tab that should be displayed (committed tab and switch intent, as `resolveDisplayedTabId` says) with the tab whose container the page is showing (inline `z-index` and `pointer-events` on the tab containers). Disagreement with no switch, gate or creation in flight for 2 s: log it and set the display signal directly; if the page still doesn't follow 2 s later, log it and show a banner in that window ("This window stopped switching tabs." with a Reload button). The window is never reloaded on its own: a reload would lose a composer draft or a drag under way. Only a different tab being shown counts; no tab shown is a transient state of a tab closing, opening or being revealed, and an earlier version that acted on it fired during ordinary switching. The banner goes away if the page follows the active tab again.
3. **Hidden Swarm panes don't rebuild.** A Swarm pane whose own visibility isn't `active` (another window tab, or a pane tab behind its siblings) keeps its last list and rebuilds once when shown. Every agent event used to rebuild every Swarm pane in the window.

## 5. Not known, not done

- **Which `<Show>` throws.** The frames are Solid internals and a minified store dispatch. The row guard turns the next occurrence into a log line naming the row type and id.
- **Other consumers of the transcript store** (the activity dock, the bottom panels) aren't under the row guard. The watchdog covers the tab display for any cause; other effects stranded by an unguarded throw elsewhere are not covered.
- **`retired` rebuilds, about 240 a minute in that window:** a per-pane write about once a second; not investigated here.
- The first-visit pop-in of agent transcripts in never-shown tabs is a separate finding (task #3286).
