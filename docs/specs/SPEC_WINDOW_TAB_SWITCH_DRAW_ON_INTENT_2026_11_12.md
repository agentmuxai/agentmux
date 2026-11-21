# SPEC: A faster window-tab switch: draw a tab's agent panes while the pointer is on its pill, so every pane appears at once at its latest content

**Status:** active (approved, being built). Camper (panes and tabs) agreed the design on 2026-11-12; the operator approved it on 2026-11-20 as recommended (relayed by AgentY). Spec task #47492, build #47493.
**Author:** Loap-3 (performance). **Lane:** panes and tabs are Camper's; this reuses his prewarm lease (`ANALYSIS_WINDOW_TAB_PREWARM_2026_10_25.md`) and the pill-intent signal of browser hibernation (`SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md` §6).
**Verified on:** `origin/main` at `e1a0e2044` (2026-11-12), 0.60.25. Re-checked 2026-11-20 against `origin/main` `87af6bbdf3`: the files this spec builds on (`agent-prewarm.ts`, `agent-prewarm-driver.ts`, `usePrewarm.ts`, `tab.tsx`, `agent-dormancy.tsx`) are unchanged since; the agent view's scroll code it only reads has had three changes (#7153 keeps the topmost row in place, #7914 limits a flush to the live end of a long transcript, #7261 is comments), so the build re-reads `settle-on-show.ts` and the virtual list before it starts and the section 2 numbers stay the baseline.
**Follows:** PR #7039 (a pane in a hidden tab loads its older history before the first show) and `REPORT_TAB_SWITCH_CATCH_UP_2026_11_11.md`, whose §7 this spec corrects (section 6).

The operator's words, relayed by AgentY: window-tab switches make "multiple active agentpanes ... appear to scroll", long, slow and glitchy; each pane should appear at once, a working agent where it would be had the tab stayed selected; and the switch should be faster. On 2026-11-12: "did we get the tighter tab reveal work in?"

## 1. Summary

Nearly every window-tab switch is already a one-frame swap. What takes the time is what happens after it: the agent panes of the tab you arrive at have been **asleep** (dormant: they keep their data current but draw no markdown and no tool bodies, `agent-dormancy.tsx`), so they draw everything that came in while hidden in the frames after the swap. The transcript is covered meanwhile, for 146 ms at the median and 253 ms at the 90th percentile (97 shows in the operator's logs), longer when several panes wake together.

The proposal: **start drawing a tab's agent panes when the pointer rests on its pill, before the click.** The switch happens when the button is released (`onClick`, `tab.tsx`), so the gap is the time from the pointer's arrival to the release: the aim, plus the press itself. The draw moves out of the switch into that gap. If the gap is long enough the panes are already at their latest content when the tab appears and there is nothing to cover; if not, whatever is not drawn yet is drawn after the swap exactly as today. Nothing gets slower.

This reuses two things that exist: the pill intent signal (`hoverTabPill` / `pressTabPill` / `leaveTabPill`, today only for waking sleeping browser panes) and the one-pane-at-a-time draw lease (`store/agent-prewarm.ts`, today only for never-shown tabs, only when idle, off by default). Cost is paid only for tabs the person points at, not for every tab in the window.

## 2. What the person sees today

Recorded, not measured fresh (host logs 0.60.21 to 0.60.25 on the operator's machine, 2026-11-07 to 2026-11-12):

| Case | What happens | Recorded |
|---|---|---|
| **Warm swap** (119 of 137 reveals were `source=ungated`; 118 of the 122 switches with a timing) | The pill and the content swap in one frame, before the backend answers (`resolveDisplayedTabId`). Includes the first visit to a tab the window loaded with: those count as shown once their layout settled (`markLoadedTabsShownWhenSettled`), but their agent panes have drawn nothing yet. | `[perf] tab-switch` is logged only above about 100 ms: 118 lines, median 218 ms, 90th percentile 351 ms. That mark ends two frames after the backend's answer, so it is the round trip, **not** when the tab appears. |
| **Panes whose content changed while hidden** (the common case with agents) | Transcript covered until its size holds still for 2 frames (cap 350 ms), while the panes draw the new content; then it lands at the bottom (`useSettleOnShow`, `OneWayFlow.settleOnShow`). | `[pane-cover] show-cover cleared`: n = 97, min 53, **median 146, 90th percentile 253**, max 384 ms. No `lifted-early` line yet in the logs read (that line is new in 0.60.25). |
| **Working agents** (output arriving at the switch) | As above, then the agent's own output follows at the bottom. Not separable from a catch-up in the logs: in none of 27 sampled shows were two panes streaming. | Lab case B/C of the report: identical before and after #7039, within noise. |
| **Cold tab** (never shown, not loaded with the window: made later, or the setting off) | Waits for the backend and the 80 ms quiet gate. | 18 gated reveals in five days, of which 4 are switches with a timing: median 320 ms. Rare. |

What the lab added (case A of the report, a first visit to a tab the page loaded with, three finished panes): first frame with the new tab at 60 to 190 ms after the click, transcripts uncovered at 235 to 321 ms, and two long tasks of about 110 and 160 ms right after the click, which is the panes drawing. The lab clicked after a page load, the same path as a first visit to a loaded tab.

## 3. What the person sees after, and the targets

- **A pane appears once, at its latest content, with no frame in which it visibly scrolls to its place.** For an idle agent: no frame whose rows share nothing with the last painted one, and none more than half a screen short of the bottom while following (already 0 after #7039 for a first visit; this keeps it, including for a re-visit). For a working agent: only its own new output moves.
- **Time from the click until every pane of the tab is uncovered**, for a show where the pointer rested on the pill at least 300 ms: no cover at all in most cases, median at most 50 ms and 90th percentile at most 120 ms (today 146 and 253). With no rest (a keyboard switch, a programmatic switch, a click with no hover): unchanged from today, never worse.
- **First frame with the new tab** at most 100 ms after the switch (the click) at the 90th percentile (today 60 to 190 ms in the lab), because the draw is no longer in the switch's own task.
- **Main thread:** the draw ahead runs in the hover, one pane at a time, and a pane's draw must not hold the main thread longer than the pane's draw does today (about 200 ms per pane, report §1.3 and the prewarm analysis §4). Typing in the displayed tab must not stall: no lease starts within 500 ms of a keystroke.

These are targets from numbers already recorded; whether a hover is long enough is not known (section 5).

## 4. Design

1. **One intent signal for the window** (agreed with Camper, 2026-11-12). A new module `store/tab-intent.ts` exports `tabIntentId` (a signal) with set and clear. `hoverTabPill`, `pressTabPill` and `leaveTabPill` (`view/browser/hibernation.ts`) keep their names and call it, and `prewakeTabId` becomes `export const prewakeTabId = tabIntentId`, so `droppable-tab.tsx` and the two readers in `use-pane-sleep.ts` are untouched and the agent lane does not import from `view/browser`. `droppable-tab.tsx` calls `hoverTabPill` only for an inactive tab, and `pressTabPill` on `pointerdown` (build: check both are wired as read).
2. **Draw lease on intent.** The panes of the intent tab take the existing lease (`store/agent-prewarm.ts`) one at a time: the pane with focus first, then layout order, skipping panes behind another pane tab (`isBlockDormant`, as `usePrewarm` does). The lease is a single holder (`setLeaseHolder`), so "all panes together" would mean changing it to a set; not done for a lead of a few hundred ms. The lease itself already lets any hidden pane draw from its current store (`useRenderDormant`: dormant = hidden and not leased); the limit is the scheduler, so the build changes three things in it:
   - **Its own candidate rule.** `pick()` skips a block once `warmed` (permanent) or when its tab was seen. Intent needs: the tab is not displayed, and the pane is stale, through a new `stale: () => boolean` on `PrewarmCandidate` (never drawn, or its document changed since it last drew; the cover's `changes` identity in `useSettleOnShow` is the candidate). An intent draw does not add to `warmed` and is not counted as the idle prewarm's. Whether a lease flushes rows a hidden pane held back (`useLiveFeedRollOff`'s hidden-pane grace) is checked in the build before it is promised.
   - **Its own entry and conditions.** `idle()` needs 2 s without `pointermove`, and a hover is a pointer move, so the intent path does not use it. It requires: window not hidden, no `tabSwitching()`, no `windowResizing()`, memory pressure normal, no keystroke in the last 500 ms. `intent(tabId)` runs a pass now; the 3 s timer is not its entry point.
   - **Sharing the lease.** An intent draw takes the lease from the idle prewarm (both can be on) and gives it back at once on leave or switch; lease cap 4 s as for prewarm; a pane that closes gives it back.
   - A pane drawn ahead must not make `tabWasShown` / `markLoadedTabsShownWhenSettled` count the tab as visited.
3. **A press starts the same draw at once** (`pressTabPill`, on `pointerdown`) for any pane not yet drawn. The switch itself runs on the release, so even a click with no hover has the press-to-release time as a lead (not measured; the `[tab-intent]` log records it).
4. **The cover.** A pane drawn ahead is current at the show, so `useSettleOnShow` finds nothing that changed and does not cover it (it still lands at the bottom, `settleOnShow`). A pane the hover did not reach is covered as today.
5. **Leaving without a switch.** The lease ends when the pointer leaves, the pane goes dormant again, and what it drew stays (as for prewarm). A tab already drawn ahead and unchanged is not drawn again.
6. **Setting.** `window:drawtabonhover`, default on, to turn it off if it costs more than it gives. The existing `window:prewarmagenttabs` stays as it is, off.
7. **Logs** (these are the measurement, section 5): `[tab-intent] hover=Nms press=Nms drawn=A/B` at the switch (pointer arrival to switch, press to switch, panes drawn of the tab's panes), and `[pane-cover] show-cover skipped (drawn ahead)` beside the existing `shown` / `cleared` lines.

## 5. What it costs, and what is not known

- **Memory.** Only tabs the person points at, and only panes that changed or never drew. What a drawn pane holds is what it holds after a visit today. Recorded for a pane drawn ahead: +330 DOM nodes and about +5 MB heap for two panes with short histories (prewarm analysis §4); about +5% of the page's heap with three 3,000-line transcripts loaded (report §4.1). No new standing cost for tabs nobody points at.
- **Work for nothing.** A hover that does not become a click draws a pane that is not seen yet. Bounded by the lease cap and by "not drawn again while unchanged". A pointer moving across the strip draws the first pane of each tab it dwells on (100 ms dwell); this must be measured, not assumed small.
- **Unknown: how long the gap is.** Nothing recorded says how long the pointer rests on a pill before the press. The browser spike measured a 600 ms hover as enough for three of seven pages. A pane draws in about 200 ms and three panes in sequence need 0.6 s or more. The gain is at most the gap, and for a keyboard switch (Ctrl+Tab, Alt+number) it is zero.
- **Unknown: the cost of a draw on a long-running agent** (the layouts of the prewarm analysis were short).

## 6. Not doing, and a correction

- **Not the cold-tab optimistic show** (report §5 option 4, §7). It addresses 4 timed switches in five days. `markLoadedTabsShownWhenSettled` makes every tab a window loads with warm, and `createTab` marks a tab it built warm, so the cold path is the exception. §7 of the report treated the cold tab as the common first visit; it is not. The tab-switch path (`setActiveTab`, `SetActiveTab`) is not touched by this spec.
- **Not a frame-counted cover cap** (dropped in the report §6) and **not a snapshot** of the tab (the 09-30 analysis §1: a snapshot is slower to put on screen than the live tab).
- **Not keyboard prediction** (drawing the next tab in order before a Ctrl+Tab). Possible later, from the `[tab-intent]` log; it costs a draw per tab that may not be wanted.

## 7. How it is measured

1. **From the operator's own use, no lab.** The two new lines in section 4 plus the existing `[pane-cover] show-cover cleared after=` and `[perf] tab-switch` give, after a few days: the lead distribution (how long pointers rest on a pill, and how long a press lasts, before the switch), the share of shows with the panes drawn ahead, and the cover time with and without a lead. Baseline for comparison is the table in section 2 (n = 97).
2. **A lab, on a testbed host (Area54, charlie, gamerlove or starpower), never narko; the host's custodian is told first and no operator OK is needed there** (one answer to one question: does a drawn-ahead pane show without a cover and without a long task after the press?). The existing `scripts/perf/tab-switch-lab.mjs` cases A and C, with the hover handler called 0, 300 and 600 ms before the switch, two runs each, interleaved with the variant off; a dev build of `origin/main` on its own home, the same three 3,000-line transcripts, about 10 minutes, the dev window visible, no real mouse or keyboard. Not run yet: it needs the build's switch (the hover handler) to exist, so it runs on the build's branch, as its before/after.
3. **Pass** when, for a 300 ms lead: median cover at most 50 ms, no frame with `jumpFrames` or `bigGapFrames`, first frame at most 100 ms, no long task over 50 ms in the 400 ms after the switch, heap after launch within 5% of today's. **Fail** (do not ship on) when the lead-less case is slower than today, or a hover that is not followed by a switch leaves the heap growing (draw the same tab 20 times, hover only).

## 8. What I need

- **Camper** (panes and tabs lane): agreed section 4 on 2026-11-12 with the answers now folded into it (signal module, scheduler changes, one pane at a time, no break of the reveal gate or his glide pass, which read `tabSwitching()` only). He owns and will review `store/agent-prewarm.ts`, `workspace/agent-prewarm-driver.ts` and `hooks/usePrewarm.ts`; the build writes the changes in them. No objection to D1.
- **The operator, through AgentY:** approval of the design (D1: build draw-on-intent, default on behind a setting) and nothing for the lab in section 7.2 (D2): it runs on a testbed host with its custodian told, which needs no operator OK, and it is not needed to build.
