# Why window-tab switches feel glitchy and pane-tab switches don't, and what VS Code does instead

**Date:** 2026-09-24
**Status:** analysis — recommendations in §6, none implemented yet.
**Author:** agentx
**Trigger:** Repo owner, after the half-window pane offset
(`docs/reports/REPORT_TAB_PANES_OFFSET_HALF_WINDOW_2026_09_24.md`): *"switching
tabs feels glitchy. on vscode it is much more solid … the pane tab switch is
very fast. the window tabs are the slow ones."*
**Related:** `ANALYSIS_CHROME_PARITY_TAB_SWITCH_LATENCY_2026_09_15.md` (#3239,
which introduced `content-visibility`), `SPEC_TAB_CONTENT_REVEAL_GATE.md` (#814),
`SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` (the unmet < 100 ms bar),
`SPEC_TAB_SWITCH_PER_PANE_PROGRESSIVE_REVEAL_2026_09_21.md` (draft),
`SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md` (#3519),
`SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md` (#3391).

Code citations are against `main` @ `8b69b291d`. VS Code citations are against
`microsoft/vscode` @ `ab52f2c` (fetched 2026-09-24).

---

## 1. Summary

Pane tabs and window tabs hide inactive content in two different ways, and
that one difference explains the gap.

- **Pane tabs** hide inactive members with `visibility: hidden`
  (`frontend/app/tab/pane-leaf-chrome.tsx:373`). Hidden members stay fully
  laid out, so a switch is one local signal change. There's no layout to catch
  up on, no gate, no transition and no round trip.
- **Window tabs** hide inactive tabs with `content-visibility: hidden`
  (`frontend/app/workspace/workspace.tsx`, the tab container's style). That
  skips layout of everything inside the tab while it's hidden. The agents in
  it keep streaming (they aren't dormant; only pane-stack members are), so
  every return has to lay out everything that changed in the meantime.
  The rest of the switch is machinery that has built up around that cost:
  - awaiting a backend round trip;
  - a reveal gate that waits for 80 ms without long tasks;
  - a 120 ms View Transition;
  - a forced layout read;
  - a focus call two frames later.

VS Code's editor switches feel solid because it does the opposite on each of
these points. It pushes sizes down instead of discovering them, switches in one
task with no gate or transition, and focuses in the same flow (§4).

The recommendation (§6) is to make window tabs work like pane tabs: keep hidden
tabs laid out, take the round trip off the critical path, then retire the gate
and transition for tabs that have already painted.

## 2. What a switch costs today (measured)

From the running 0.57.0 instance's host log, 2026-09-24, every window-tab
switch. `reveal` is when the gate lifted, measured from the RPC's log line.
`after` is the long tasks in the 3 s after the reveal, which is work the gate
had already declared settled.

| Time (UTC) | RPC | Reveal | Long tasks after reveal |
|---|---|---|---|
| 10:57:41 – 12:23:32 (14 switches) | 7–55 ms | 110–130 ms | 0 on 12 of them, one 61–63 ms task on 2 |
| 13:05:16 | 21 ms | 113 ms | 4, total 277 ms |
| 13:22:04 | 10 ms | 133 ms | 6, total 394 ms |
| **13:25:45** (the offset occurrence) | 15 ms | 151 ms | 6, total 477 ms |
| 13:26:04 | 18 ms | 133 ms | 4, total 313 ms |
| 13:26:12 | 17 ms | 127 ms | 0 |
| 14:31:42 | 37 ms | 86 ms | 2, total 134 ms |
| 14:31:59 | 12 ms | 128 ms | 0 |

Two separate problems show up:

1. **A fixed floor on every switch.** The gate never lifts sooner than about
   85 ms after it's armed (`SETTLE_MS = 80`, `frontend/app/store/tab-reveal.ts:77`),
   even when there's nothing to settle. The 120 ms cross-fade
   (`app.scss`, `workspace-tab-content`) runs on top. So a window-tab switch
   takes at least ~200 ms to finish visually, while a pane-tab switch finishes
   in one frame.
2. **Catch-up after reveal on busy tabs.** From 13:05 onward, with agents
   streaming in the hidden tab, switches were followed by 277–477 ms of long
   tasks after the gate had lifted. The gate can only wait for work it can
   see coming, so the tab appears and then stutters. That's the "glitchy"
   feel, and it's the unsettled window the #3519 refocus landed in.

**Caveat.** Agents stream all the time, so not every long task in that window
is caused by the switch. The contrast is still clear. Light tabs in the morning
had none. Busy tabs in the afternoon had 4–6 each, clustered in the first
1.7 s. The #3239 analysis was never benchmarked against the 500–600 ms May
baseline, so these are the first post-#3239 numbers.

## 3. The two mechanisms side by side

| | Pane tabs | Window tabs |
|---|---|---|
| Hiding | `visibility: hidden` + `pointer-events: none`, absolutely stacked (`pane-leaf-chrome.tsx:373-374`) | `content-visibility: hidden` + `pointer-events: none`, absolutely stacked (`workspace.tsx`) |
| Layout while hidden | Kept current | Skipped. It's owed on show |
| Before first paint | A local signal change. The backend write is a debounced fire-and-forget (`layoutPersistence.ts`) | **Awaits `WorkspaceService.SetActiveTab`** (`tab-actions.ts:129`) |
| Gate | None. Removed on purpose (#3136/#3157), because the gate itself caused the flash | Whole-tab `visibility: hidden` until 80 ms without long tasks, or 800 ms (`tab-reveal.ts:71-82`) |
| Transition | None | `document.startViewTransition`, 120 ms (`workspace.tsx:75-76`) |
| Forced layout | None | `getBoundingClientRect()` on the container right after the flip (`workspace.tsx:115`) |
| Focus | Terminal calls `giveFocus()` directly | `refocusNode()` two rAFs after the RPC (`tab-actions.ts:135-150`) |
| Hidden agents | Dormant (`isBlockDormant`) | **Not dormant.** Streaming keeps mutating the DOM |

The history explains how window tabs got here. Hiding went from `display: none`
(which discarded layout and caused the 0×0 → real-size ResizeObserver burst of
`REPORT_TAB_FLASH_SYSTEMIC_ANALYSIS_2026_08_31.md` §3.4) to
`content-visibility: hidden` (which keeps the container's size but still skips
its contents). Each layer after that (the gate, the fade, then the View
Transition, the forced read, the refocus) was added to hide or work around the
catch-up. None of them removes it. Pane tabs went the other way: #3136/#3157
removed their gate, and they switch instantly.

## 4. What VS Code does

VS Code has no exact equivalent of tabs holding tiled layouts; its grid stays
visible and switches happen inside each editor group. Its principles still
carry over. `B` = `https://github.com/microsoft/vscode/blob/ab52f2c82c4677a5983435d7f706380eaac43c09`.

1. **Push sizes down; don't discover them.** A window `resize` calls
   `layoutService.layout()`, which reads the size once and calls
   `workbenchGrid.layout(w, h)`. `SplitView` writes explicit pixel positions
   and calls `view.layout(size)` on each child (`B/src/vs/base/browser/ui/splitview/splitview.ts#L281-L311`).
   None of `grid/`, `splitview/`, `layout.ts` or `parts/editor/` uses a
   ResizeObserver. A pane gets its size **before** its content arrives
   (`editorPanes.ts` `doShowEditorPane`, L323-L360), so it's never visible at
   the wrong size.
2. **Show only what's active.** Each group reuses one pane per editor type and
   swaps the model into it. The outgoing pane is detached (`display: none` +
   `remove()`, `editorPanes.ts#L487`), so hidden panes don't take part in
   layout, observers or focus.
3. **Switch in one step, in one task.** Hide old, show new, `layout(size)`,
   set input, focus (`editorPanes.ts#L250-L267`). There's no rAF delay, no
   settle gate and no transition. The first frame after a switch is the final
   one.
4. **Save and restore state explicitly.** View state is saved on `clearInput`
   (`editorWithViewState.ts#L67-L70`) and restored straight after `setModel`.
   Derived decorations come synchronously from a cache (CodeLens,
   `codelensController.ts#L144-L146`, "cache model to reduce flicker", the fix
   for vscode#41968 "editor position is intermittently jumpy when switching
   tabs").
5. **Make containers unscrollable.** `layout.ts#L481`, commented "Prevent
   workbench from scrolling #55456", resets `mainContainer.scrollTop = 0` on
   every scroll. Monaco saves and restores parent scroll positions around its
   textarea focus (`textAreaEditContextInput.ts#L787-L791`: "we try to undo
   the browser's desperate reveal"). This is the class of bug #3671 closes
   with `overflow: clip`.
6. **Defer hidden work, apply it on show.** Config changes made while a pane
   is hidden are queued and applied on show (`textEditor.ts#L288`). Hidden
   terminals resize at idle, and pending resizes are flushed synchronously on
   `setVisible(true)` (`terminalInstance.ts#L1442-L1453`).

**Platform notes** (CSS Containment 2 §4.5; View Transitions 1):
- ResizeObservers never see skipped content change size. Their observations
  arrive a frame late, after the content is shown. Our catch-up is built
  from exactly those observers (about 8 per agent pane, 3–4 per terminal).
- Skipped content can't be focused, so a focus call that lands before the
  flip does nothing.
- During a View Transition the captured elements aren't painted or
  hit-testable. A transition stacked on a gate adds frames where input goes
  nowhere.

Chromium's own tab switch has a fallback surface (the last frame). Inside one
document we don't, so the first frame after a switch must already be complete.
Hiding the destination until it settles just shows a blank frame instead.

## 5. Root cause of the glitchiness

The layout of a hidden window tab is allowed to fall behind, then settled in a
burst when the user returns. Every layer since (gate, fade, transition,
forced read, delayed focus) tries to hide that burst. None of them can,
because it happens after the gate lifts, on ResizeObserver callbacks the gate
can't see coming. Pane tabs don't let layout fall behind, which is why they
don't need any of those layers.

## 6. Recommendations, ranked by how much glitchiness each removes

Each one should be measured with the existing `[perf] tab-switch`,
`[perf] tab-reveal` and `[perf] long-task` marks, using the §2 method: long
tasks in the 3 s after reveal, on a busy tab.

1. **Hide inactive window tabs with `visibility: hidden`, as pane tabs do.**
   Replace `content-visibility: hidden` in `workspace.tsx`, and keep
   `pointer-events: none` and the absolute stacking. Hidden tabs then stay
   laid out, so there's no catch-up on return. Paint is still skipped.
   - *Cost:* hidden tabs' DOM changes now take part in each frame's layout.
     That's the same as pane-stack members today, and it spreads the work
     over time instead of concentrating it on the switch.
   - *Mitigate with 2.*
   - Ship behind a setting so it can be A/B'd live against today's numbers.
   - This is the in-repo proven model. VS Code gets the same effect differently
     (detach plus explicit sizes), but that's a much larger refactor (7).
2. **Make agents in hidden window tabs dormant**, as pane-stack members are
   (`isBlockDormant`, `pane-leaf-chrome.tsx:362-364`). Keep buffering the
   stream in the store, but pause markdown rendering and row mounts while the
   tab is hidden, then apply on show. This is VS Code principle 6. It keeps
   1's background cost down.
3. **Take the round trip off the critical path.** Set `displayTabId` locally
   as soon as the user clicks, then send `SetActiveTab` and reconcile if the
   backend disagrees. #2993 already does this for the tab pill; pane tabs
   already do it for content.
4. **Retire the reveal gate and View Transition for tabs that have already
   painted.** Once 1 has landed there's nothing left for them to hide, and
   they cost at least ~200 ms per switch (§2). Keep the gate only for a tab's
   first-ever reveal. #3300 already limits `createTab` to that shape.
5. **Focus in the same task as the show, with `preventScroll`.** Once layout
   is current, there's no transient geometry to wait out. Replace #3519's
   two-rAF `refocusNode()` (`tab-actions.ts:135-150`) with a direct call right
   after `displayTabId` flips. #3671 already makes the agent pane's
   `giveFocus()` pass `preventScroll`, and makes `.tile-layout` unscrollable.
6. **Set a bar and check it in.** The May spec's < 100 ms of long tasks per
   hot switch is still unmet. Measure it the §2 way on a tab with 3–4
   streaming agents, record before and after each step, and keep the method
   as a repeatable script.
7. **Later: push tile sizes down instead of discovering them.** The tile tree
   already computes each tile's rectangle (`layoutGeometry.ts`). Handing it
   to panes as `layout(w, h)` would remove the ResizeObserver cascades
   altogether (VS Code principle 1). It's worth doing only if 1–5 leave a
   measurable gap.

**Not recommended** (already tried or ruled out):
- Per-row `content-visibility: auto` in agent panes. Tried in May: 700–1090 ms,
  worse than the baseline, and reverted.
- More or longer gates. A gate can't see work that starts after it lifts. The
  per-pane progressive reveal draft
  (`SPEC_TAB_SWITCH_PER_PANE_PROGRESSIVE_REVEAL_2026_09_21.md`) refines gating
  but doesn't remove the catch-up. 1 makes it unnecessary for tabs that have
  already painted.

**Suggested order:** 1 + 2 together as one experiment behind a setting, with 6's
measurements. If they hold, 3, then 4, then 5. Each is small on its own. 1 and
4 are the ones users will feel.
