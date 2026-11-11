# Report: the catch-up scroll on a window-tab switch (#47436)

**Date:** 2026-11-11
**Status:** analysis. §1 to §3 are read from the operator's own logs (0.60.23 and 0.60.24, 2026-11-11). §4 is the lab, run on 2026-11-11 with the operator's OK (relayed by AgentY), and §6 says what was built on its result.
**Scope:** the agent pane's transcript on a window-tab switch (`frontend/app/view/agent/`, `workspace/`).
**Related:** `SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md` (the October mechanism: `scroll/settle-on-show.ts`, `OneWayFlow.settleOnShow`), `ANALYSIS_WINDOW_TAB_PREWARM_2026_10_25.md`.

The operator's words: "multiple active agent panes that appear to scroll when switching the window tab". The October mechanism covers one pane that got output while hidden, for at most 350 ms, and keeps layout changes at the bottom for 400 ms after the first painted frame. The logs show three things it does not reach.

## 1. Measured (operator's logs, 36 pane shows in 0.60.23/0.60.24)

**1.1 A pane shown for the first time since launch finishes loading its older history 0.6 to 1.4 s after the show.** 8 of the 36 shows, each followed by an `[agent-open-rest]` line for the same pane:

| show | pane | seconds after the show | lines |
|---|---|---|---|
| 04:25:19 | QTest | 0.90 | 3,308 |
| 04:54:35 | Agent2 | 1.35 | 1,378 |
| 04:56:17 | Klon | 0.87 | 910 |
| 04:56:27 | Deke | 0.64 | 68 |
| 07:04:18 | Clamk | 0.97 | 3,675 |
| 16:57:38 | Camper | 1.04 | 2,369 |
| 17:00:18 | Agent2 | 1.08 | 2,188 |
| 17:00:18 | Clamk | 1.08 | 4,122 (1.6 MB, 2 pages) |

Why: `fillRest` (`hooks/useHistoryPagination.ts`) holds while `fillPaused()` is true (`agent-view-transcript.ts:60`: the pane is dormant, or not following) and while the person gave input in the last second (`FILL_QUIET_MS`). A tab nobody has opened since launch is dormant the whole time (prewarm, `window:prewarmagenttabs`, is off by default and no `[prewarm]` line is in these logs), so its fill starts about one second after the click that shows it. That is after the cover (at most 350 ms) and after the settle window (400 ms from the first painted frame). The older pages are prepended above a pane that is already shown.

**1.2 Shows of several panes at once reach the cover's 350 ms cap; a single pane never does.**
- Single-pane shows (21): cover 66 to 268 ms.
- Several panes at once (7 shows): 151/151, 108/108, 150/196, 193/193, 194/226, **346/384**, **254/365/365** ms. The last two lifted at the cap, not because the layout had been still for two frames.
- The cap is wall-clock from the first frame (`settle-on-show.ts:85`). A 150 ms main-thread block (two long tasks of 67 and 152 ms at 17:00:18) spends it in one step.

**1.3 The main thread is busy for 100 to 740 ms in the two seconds after a show.** Long frames over 50 ms in the 2 s after each show: 0 to 6 per show, 83 to 737 ms in total (worst 259 ms). The 17:00:18 show: 5 frames, 586 ms, the worst 228 ms (scripts 117, render 148, style+layout 79). By invoker over all shows: promise continuations of a dynamic `import()` (about 1.1 s in total over the day), frames with no script, the view-transition callback, `Response.json` parsing (the fill's pages), the document click handler.

**1.4 The switch itself.** `[perf] tab-switch` over 59 switches in 0.60.21 to 0.60.24: typically 150 to 370 ms, of which the `SetActiveTab` round trip is most (about 60 to 75%; 95 to 640 ms in all but one switch). The 17:00:18 switch: 319.6 ms, `SetActiveTab` 242 ms, reveal gate 82 ms. A warm tab is swapped in optimistically; a tab not yet shown since launch waits for the round trip and the gate.

**1.5 Which shows were "several working panes".** The `[wave-turn]` watchdog says each pane's phase every minute; the last line before each of the 27 show moments gives the phase at the show. In none of them were two panes `Streaming`. The nearest: 07:10:33 (Deephar streaming with Clamk done), 04:54:44 and 16:50:07 (QTest streaming with AgentY done). The 17:00:18 show had Agent2 and Clamk `Done` and Deephar `Idle` (3 s before). So the operator's case with several working panes at once is not in these logs; the lab must run it (case C, §4), and the multi-pane covers at the cap (§1.2) were of finished panes with large backlogs.

## 2. Why the October mechanism does not cover this (from §1)

1. It looks at the layout the pane has in the 350 ms after the show. The older history (§1.1) arrives later and is not "content it got while hidden".
2. Its cap is time, so the busiest shows (several panes, a large backlog, a blocked main thread) are the ones that lift early (§1.2).
3. It works on the pane's own size. It does not know that other panes are drawing in the same frames (§1.3).

## 3. Candidates (not measured)

- **C1** The prepend of the fill (§1.1) moves what is on screen, or remounts the rows of the head, after the settle window: "a long scroll, like it is catching up to history".
- **C2** A cover lifted at its cap shows the rest of the layout in stages, which the pinned pane shows as text jumping up.
- **C3** Panes shown together draw in the same frames; each pane's commit delays the others.
- **C4** A working agent's output after the show eases in per pane (the follower), which reads as scrolling when four panes do it at once.
- **C5** Rows dropped or windowed while hidden (#46547) return at estimated heights and re-measure in stages. Not seen in the logs; would show in the lab.

## 4. The lab (needs the operator's yes through AgentY)

- **Question:** which of C1 to C5 moves what the person sees after a window-tab switch, frame by frame, and by how much. It decides what to change; the October rule is "ship a perf fix only on a measured gain".
- **Runs:** one task dev build of `origin/main` (my own checkout and home, never the operator's instance), a second window tab with three agent panes (fake Claude; transcripts of 2 to 4k lines, 1 to 1.6 MB like the operator's), two of them streaming. An in-page probe samples each pane's `scrollTop`, `scrollHeight`, `clientHeight`, five visible row tops and the cover flag on every frame for 3 s after the switch. The switch is a call in the page, not a click.
  - A: first show since launch (as at 17:00:18), three finished panes.
  - B: show of a tab hidden for 5 minutes while one agent kept working.
  - C: as B with all three agents streaming (the operator's clarification; not in the logs, §1.5).
  - Each twice. The same again after the change.
- **Host and build:** narko, `origin/main`.
- **Time:** about 10 minutes of runs, after a build I would do anyway.
- **Screen and input:** the dev window is visible on the desktop for those minutes; no real mouse or keyboard; one screen recording of the pane before and after.

### 4.1 What the lab showed

Run in a `task dev` build of `origin/main` (808212be1's parent, 9e13028be) on its own home, with `scripts/perf/tab-switch-lab.mjs`: before and after interleaved in one session on the same transcripts (three panes of 3,024 lines each, about 1 MB, in the second tab), two runs per case, the page reloaded before each. A dev build is slower than the packaged one in absolute terms; only before against after is compared. The switch is the tab's own click handler called in the page; the probe samples every animation frame for 3.5 s. A first pass had a defect (one cold run started with the tab already displayed, and no raw frames were kept), so the same cases were run again; the numbers below are the second run (`tab-lab-2`).

**Case A, first show since the page loaded (three finished panes):**

| per pane, both runs agree | before | after |
|---|---|---|
| `[agent-open-rest]` after the click | 408 to 659 ms (lines 619 to 717) | none: the fill finished while the tab was hidden |
| painted frames while following more than half a screen short of the bottom | 1 (2,689 to 2,737 px short) | 0 |
| frames whose rows share none with the last painted frame (the view replaced) | 2 | 0 |
| content height steps after the cover lifted | 1 of 2,688 to 2,736 px | 0 |
| cover (from the first painted frame) | 235 and 321 ms | 313 and 244 ms |
| main-thread long tasks in the 3.5 s | 316 and 1,158 ms | 479 and 326 ms |

The late fill is the frame the person sees as the pane catching up to history: the view is replaced by older content 2.7k px above the bottom, then put back, 100 to 150 ms after the cover lifts. The recordings (`switch-before.mp4`, `switch-after.mp4`) show it: after the reveal the first pane changes its picture three more times in the before recording and not at all in the after one.

**Heap, in case A before the second tab is shown (page's JS heap after a collection, DOM nodes):** before 76 and 73 MB, 7,825 nodes; after 77 and 78 MB, 8,735 nodes: about 4 MB (5%) and 900 nodes (11%) more with three 3,000-line transcripts loaded ahead.

**Cases B and C (a pane working at the switch, or all three):** the same in both variants within the noise of two runs. Rows move (up to 3,000 px of motion in 3.5 s) and a frame short of the bottom by up to 2,100 px shows once in one run of each variant, 0.8 to 1.4 s after the click: a working agent's own output arriving in a large step (a tool result of several hundred lines), which a pane that had stayed selected shows the same way. Nothing in these two cases separates a catch-up from the agent's own output, so no claim is made for them. In case B the working agent had mostly finished before the switch.

**The cover (option 2):** in case A it lifted at 235 to 321 ms in all four runs (two per variant), below the 350 ms cap, so the cap did not act. In the first pass of case C, two covers lifted at a frame-counted cap after 10 frames (394 ms from the first frame, 526 ms from the show, against 296 ms in the before run), so the longer cap is not shown to help and can lengthen the blank. It was not kept (§6).

**The switch's speed:** the first frame with the new tab shows 60 to 190 ms after the click and the transcript is uncovered at 235 to 321 ms, in both variants. This change does not make the switch faster. A larger share of those 300 ms is the main thread: two long tasks of about 110 and 160 ms right after the click (mounting and committing what three panes got), then more while the cover is up.

## 5. Options (to choose after the lab)

Budget proposed: from the switch to the settled paint at most 250 ms for a tab already shown once, with up to four panes; after the first painted frame, no frame in which a row on screen moves, for an idle agent; for a working agent, only its own new output moves.

1. **Take the fill out of the show.** (Recommended first; fixes C1 if the lab shows it.) Either run the fill for a hidden pane in idle slices as data only (no drawing), so the show finds the history already in the document, or hold it until the pane has been visible and still for a few seconds and the person has not scrolled. A prepend is made position-neutral in the same frame (the layout store already adds the inserted height to `scrollTop`, #4962).
2. **Cover by progress, not by wall clock.** (Fixes C2/C3.) The cap counts painted frames (and has a ceiling of about 1.5 s); panes shown together are covered and released one frame apart, the focused one first. Not a blank: the cover is only of the transcript, the chrome and composer stay, and a pane that had nothing new shows at once as today.
3. **Keep the old frame until the new one is ready** (the tab content swaps when the destination has drawn). Largest change, and it makes a switch feel slower; only if 1 and 2 are not enough.
4. **Cold tab: skip the round-trip wait** by showing a not-yet-shown tab optimistically too, behind the reveal gate. Addresses §1.4, not the scroll.

## 6. What is built (PR for #47436)

- **Option 1, kept** (`hooks/hidden-fill.ts`, `agent-view-transcript.ts`, `useHistoryPagination.ts`): a pane in a hidden tab loads its fill in idle moments, one hidden pane's page at a time, and not while the page is under memory pressure; not for a hidden window, and not for a pane scrolled up. The first show then finds the history in the document. Measured in §4.1: the replaced-view frame and the 2.7k px short-of-bottom frame are gone from the cold show; the cost is about 5% of the page's heap with three 3,000-line transcripts loaded ahead.
- **Option 2, not kept.** A cap counted in frames did not change a cold show (the cover lifted by itself in every run) and lengthened a cover in the one case it acted. The cap is as it was (350 ms). What stays is the log: a cover that lifts at the cap writes `[pane-cover] ... show-cover lifted-early by=cap frames=N after=Nms`, so the operator's next log says whether covers hit the cap on a real multi-pane switch (the 17:00:18 show did, twice).
- **Not done, and why:** a faster switch (options 3 and 4) is not shown by this lab to come from either change. The lab as run can say where the time goes (the two long tasks after the click); the next measurement is one cold switch with `window:prewarmagenttabs` on, which the OK did not cover.

## 7. Option 4, the cold tab: what the round trip waits for, and what could go wrong (written for Camper, who owns tabs; nothing in the tab-switch path is changed)

**What the wait is.** `setActiveTab` (`store/tab-actions.ts`) awaits `WorkspaceService.SetActiveTab` before a tab that was never shown is revealed; a tab already shown is swapped in from the switch intent in the same frame (`resolveDisplayedTabId`, `workspace/window-tab-visibility.ts`). On the srv side the call does almost nothing: `handle_set_active_tab` (`crates/srv/src/reducer/tab.rs`) checks the workspace and the tab, sets `active_tab_id`, bumps the version and emits `ActiveTabChanged`. So the 95 to 640 ms of §1.4 is the transport and the renderer's own queue, not backend work. At 17:00:18 the call's 242 ms contains two long tasks of 67 and 152 ms starting at the click: the renderer was busy mounting the switch while it waited to read the answer.

**What showing before the answer would buy.** Less than it looks: the main thread is busy for the same 200 to 300 ms either way, and the reveal gate (`holdRevealGate`, 82 ms at 17:00:18) must still hide the destination's first paint, which is what a never-shown tab needs (its panes mount on the first visit). The saving is the part of the wait that is real latency; the logs cannot split it from the contention. It needs the lab's trace (a Chromium trace of one cold switch) before a design.

**What could go wrong.**
- The backend refuses (the tab was closed meanwhile): `cancelIntent` shows the committed tab again, so a cold tab would flash in and out. The warm path accepts that; for a cold tab the flash includes a first mount.
- Two switches in a row: the sequence guard (`tabSwitchSeq`) handles the measure, but a cold destination mounted for an intent that is then replaced is mounted for nothing.
- Everything that reads the committed `activetabid` (MuxAPI `Pane` activate, the tab strip's real state, `focusManager.refocusNode()` after two rAFs) would disagree with the shown tab for the length of the wait.
- A cold tab's `tabWasShown` flips on the first reveal; the prewarm scheduler (off by default) and `useWindowTabDisplayed` read it.

Nothing is changed here until Camper has seen this and the trace says the wait is latency, not contention.
