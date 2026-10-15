# Spec: an agent pane nobody can see doesn't keep its rendered conversation

**Date:** 2026-10-13
**Status:** proposed
**Plan item:** #6 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.
**Evidence:** `docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md` §4.4.

## 1. The problem

**What the lab measured**
- Agent panes in a background window tab keep receiving output, and build and keep their full DOM: 23k–32k elements each, the same as visible panes.
- Showing the tab changed nothing: 788 → 795 MB.
- The operator's instance has 12 agent panes over 4 tabs, so most of its agent DOM is in tabs nobody is looking at.

#4756 (tool bodies unmount on collapse) and #4751 (the `<Key>` leak) cut each pane's DOM by 94–97%. A hidden pane still renders its live window, though: the last turns, the virtualizer's window, markdown, and any tool open while it runs. The cost per pane still grows with every tab the user adds.

**What exists today**
- `agent-dormancy.tsx` is rendering-only on purpose. Its doc comment explains why: "Gating the stream flush instead would be a real regression: it would freeze the store that off-pane indicators read from."
- It pauses three things: `MarkdownBlock`'s repaints of streaming text, the AskUserQuestion auto-timeout, and the failure auto-retry.
- It doesn't stop rows from mounting, `ToolBlock` bodies, or the virtualizer's windowing and measurement. Inactive tabs stay laid out by default (`window:keepinactivetabslaidout`), so a hidden tab still has a real viewport.
- A `MarkdownBlock` mounted while dormant renders its initial content in full.

## 2. What can go and what must stay

Everything that must keep working while a pane is hidden reads from the pane's **stores and stream pipeline**, never from its rendered rows:

| Signal | Source |
|---|---|
| Turn phase in Swarm, taskbar busy count | pane-state store (`registerActivity`, `agentActivity.ts:69-92`) |
| Needs-you | question/decision hooks over the document store → block meta `term:awaiting_user` |
| Waiting tone, OS notification | `waiting-for-you.ts` → pane event → `sound-service.ts`, `os-notify-bridge.ts` |
| Tool tones, turn-complete sound, tab/pill activity flash | pane-state reducer events (`tool-started`, `turn-ended`) |
| Unseen-turns dot, Tower | srv events |
| AskUserQuestion / permission panels | `pendingQuestions` / `pendingDecisions`, derived from the document store |

**So the rule:**
- **Keep:** the agent view component, its hooks, the stream subscription, the flush queue, both stores, the layout slot (row heights), the composer and its draft (the draft is module-level, keyed by block id), and the bottom panels.
- **Drop:** the rendered document, meaning `AgentDocumentView` and everything under it.

Unmounting the whole pane would lose the stream, the reducer and the question detection, so that's out.

## 3. The change

**1. Hibernate the rows of a hidden pane.**
- When an agent pane's visibility (`usePaneTabVisibility`, the same signal as `hidden` in `agent-view.tsx:155-156`) has been anything but `"active"` for `ROW_HIBERNATE_AFTER_MS` (30 s), stop rendering `AgentDocumentView` (`agent-view.tsx:1342`).
- In its place, render a placeholder of the same size, so the pane's layout and the bottom panels don't move.
- The 30 s grace keeps a quick look at another tab from costing a rebuild.

**2. Wake on show.**
- When visibility returns to `"active"`, render `AgentDocumentView` again from the store.
- The scroll position comes from `documentStateAtom` (already persisted); row heights come from the layout slot, which lives at view level and survives.
- The pane's existing reveal cover (`usePaneReveal`, `PANE_REVEAL_BOUND_MS = 1500`) hides the rebuild, as it hides a first open.
- Target: the first rows within 300 ms of the tab showing, for a pane at the live feed's caps (about 1 MB of finished turns). That is the same target `SPEC_AGENT_OPEN_LATENCY_2026_09_27.md` sets for opening a pane.

**3. Trim the store while hibernated.**
- Going hidden already schedules a roll-off pass and `UnloadToolResults` (`useLiveFeedRollOff.ts:182-190`).
- This makes that the floor: finished turns beyond the live feed's K, and tool output of rolled-off rows.
- The turn in flight and any in-progress question/decision nodes stay. They're what the off-pane signals read.

**4. Streaming while hibernated** keeps updating the store as today. Nothing renders, so nothing is parsed into markdown or highlighted until the pane wakes.

**Not in this change:**
- Releasing the store itself. It would need srv-side status events to replace the store-fed signals in §2; it's a later step if the store turns out large after #3 and the tool-output item (#7).
- Terminals and other views.
- The `keepinactivetabslaidout` setting: with hibernated rows there's little left in a hidden tab to lay out.

**A kill switch:** `agent:hibernatehiddenrows` (default true), read once at mount, like `agent:livefeed`. It's for one release while the wake cost is watched in the field.

## 4. Tests

- **The view:** a pane hidden for less than 30 s keeps its rows. Hidden for 30 s, its document is unmounted and the placeholder holds the pane's size. Shown again, the rows render from the store with the saved scroll position.
- **Off-pane signals while hibernated**, against the existing hooks with the document unmounted:
  - a `ToolStart` still plays the tool tone;
  - a turn end still flashes the tab;
  - an AskUserQuestion still sets `term:awaiting_user` and raises the waiting tone;
  - Swarm's phase follows the turn.
- **Draft:** a draft typed before hiding is still there after waking.
- **A pending question:** its panel is visible as soon as the pane is shown, and its auto-timeout restarts in full on reveal (today's rule).
- **The kill switch off** restores today's behaviour.

## 5. Verification in the lab

- **Memory:** the 12-agent, 4-tab run from the root-cause report, with tab 1 active. Expect the renderer's DOM to be about the active tab's 4 panes; JS heap and Blink heap scale with visible panes, not all 12.
- **Wake time:** switch to each hidden tab while its agents stream, recording time to first row and to quiet with the pane's open trace (`open-trace.ts`).
- **Typing:** the typing probe (20 keys/s) in a visible composer while hidden panes stream. No regression.
