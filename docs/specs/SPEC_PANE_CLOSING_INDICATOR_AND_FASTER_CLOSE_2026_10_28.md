# A pane shows it is closing, and a close stops waiting on srv

**Status:** active. Approved by the operator; built in task #7983.
**Author:** Camper@narko, at the operator's request (2026-10-28).
**Scope:** `frontend/layout/lib/layoutMagnify.ts` (`closeNode`), `layoutStack.ts` (`closeBlockInStack`), `frontend/app/tab/tabcontent.tsx` (`beforeNodeDelete`, `onNodeDelete`), `frontend/app/tab/pane-close-guard.ts`, the pane header (`blockframe.tsx`, `PaneChrome.tsx`, `PaneTabStrip.tsx`).

The operator's words: *"if I close a pane, often even for typical ones, it takes a while to close. We need an indicator, both on expanded and minimized pane. A typical experience is I click X on a minimized pane, it may have many tabs, but after clicking X the pane remains there minimized for a couple seconds."* And, via AgentY: it happens even for an **empty editor pane**.

Words: a **pane** is one tile of the layout; a **pane tab** is one block stacked in it; a **minimized pane** is the header-only row a pane collapses to.

## 1. What close waits on today

Measured on `origin/main` 244e27c6a, in my own isolated `task dev` build (Windows, release srv), by clicking the pane's X from the page and timing until the pane's element leaves the DOM; every srv call during the close is logged (fetch and `rpcCall`).

| Case | Click → pane gone |
|---|---|
| empty editor pane | 36 ms |
| terminal pane | 31 ms |
| 8-tab terminal pane | 39 ms (16 probe calls, 1 to 12 ms each) |
| 8-tab terminal pane, minimized | 32 ms |
| same two, while four terminals flood output (CPU 90%) | 32 / 35 ms and 56 ms |
| **same, with 1.5 s added to one probe** | **1532 ms** (empty editor) and **1559 ms** (8 tabs, minimized) |

What the click path does (`closeNode` → `beforeNodeDelete`, `tabcontent.tsx`):

1. **Before the pane leaves the layout**, for **every** member of its stack, and not only agents: `BlockService.GetControllerStatus` and `agent.process-list` (the busy-agent check), plus the browser's `hibernationCheck` for a browser pane. Each is an srv round trip, awaited before the layout reducer runs. An 8-tab pane is 16 of them.
2. Only then the tree reducer removes the pane, and `ObjectService.ClosePane` (stop, delete) runs **after**, not waited for (83 ms for an empty editor).

So close is exactly as slow as the slowest of those probes. Idle, or with srv under a terminal flood, they answer in milliseconds and the pane is gone in about 40 ms; in the run with 1.5 s of added latency the pane stays 1.5 s. **I could not reproduce "a couple of seconds" on an idle or flooded dev build**, so the operator's delay comes from whatever slows those round trips on their machine (srv busy behind its state lock, the IPC stalls in #7895) or from something after removal that I cannot see from here (a heavy relayout). The design below removes close's dependence on srv for panes that have nothing to ask, and adds a trace so the next slow close says which phase it was.

## 2. The closing state

From the moment the person clicks X (or Close in the pane menu, the shortcut, or a tab's X) until the pane is gone, the pane shows that it is closing.

- **Entered synchronously**: `closeNode` and `closeBlockInStack` set the pane's closing flag before their first `await`, so the next frame already shows it. A second click, key or drag on a closing pane does nothing.
- **Where it lives**: signals beside the `LayoutModel` (`layoutClosing.ts`), keyed by node id for a pane and by block id for one tab, read through `nodeModel.isClosing()` / `closingTabs()` and `isBlockClosing()`. They are cleared when the close ends, gone or kept.
- **Expanded pane**: the header's end buttons are replaced by a spinner and **"Closing…"** (or **"Closing 5 tabs…"** for a stacked pane), the content is dimmed and inert, and the cursor is the progress cursor.

  Mockup: the same header with its buttons replaced by a spinner and "Closing…", the body dimmed.

- **Minimized pane**: the same label and spinner at the end of the row, the pane tabs dimmed and inert.

  Mockup: the minimized row with the pills dimmed and "Closing 5 tabs…" at its end.

  In a row too narrow for the words the label shortens to the spinner alone, and the row's tooltip says "Closing 5 tabs…".
- **One tab of several** (a pane tab's X): that pill dims and shows the spinner in place of its X; the rest of the pane stays usable.
- **Motion and theme**: colours come from the theme's text and muted tokens. The spinner follows the app's motion policy: with motion off it is the words alone.
- **When the close is kept** (the person answers No to the busy-agent or "Leave site?" prompt, the 30 s override for an agent, an unsaved-editor prompt): the pane returns to normal and says why in the same quiet notice the selection machine uses ("Not closed: Posa is mid-turn"). While such a prompt is open the prompt is the indicator, so the pane shows its normal look and re-enters the closing state on Yes.
- **Agent panes** keep closing in place with the shutdown log (SPEC_AGENT_SELF_QUIT §5.5); they get the same header label.
- **Closes that do not start in this window** (an agent's `ClosePane`, another window) have no frontend click to hang the state on; they stay as today (the pane disappears when srv's `delete` arrives). Out of scope.

## 3. A close that stops waiting on srv

1. **Ask only what can be answered.** `beforeNodeDelete` probes agent members for a turn in progress and processes (and browser members for the leave prompt), and nobody else. An editor, terminal, Swarm or any other pane has nothing to ask: **no round trip, the pane leaves in the same frame** (an empty editor goes from a wait on two calls to none).
2. **Remove first when nothing is asked.** With no agent or browser member the layout reducer runs at once and `ClosePane` finishes in the background, as it already does after the probes.
3. **Agent and browser members still ask**, in parallel as today, and the closing state is shown while they do. A slow answer is never read as "not busy": the pane stays in the closing state until it answers (an agent mid-turn must still get its prompt). A probe that errors still counts as not busy, as now.
4. **The close trace.** A ring of the last 50 closes, read from the console as `__closeTrace()` (like `__selectionTrace()`): each close's click, probes done, prompt shown and answered, reducer run, pane gone from the DOM and `ClosePane` returned, with the time since the click. A close slower than 500 ms from click to pane gone logs one console line naming its slowest phase. This is what will say whether the operator's delay is the probes, srv, or the relayout after removal; if it is the last, the spec's step 2 does not fix it and the build reports that instead of guessing.

## 4. Tests

- **Unit** (`closeNode`, `closeBlockInStack`, with the layout model stubbed): the closing flag is set before the first await; a second `closeNode` for the same node is a no-op; a refused or throwing close clears it and notices once; a prompt open means not closing, an accepted prompt means closing again.
- **Unit**: a stack of an editor and a terminal makes no srv call before the reducer; a stack with an agent calls the busy probes for the agent only; a slow probe leaves the pane closing and still prompts.
- **Component**: the header shows "Closing…" / "Closing N tabs…", the buttons are inert, the tab pill shows its spinner; with motion off no spinner.
- **Trace**: ring size, the slow-close line names the slowest phase.
- **Live** in my own task dev build: the §1 table again after the change (an empty editor should read about one frame), with the 1.5 s latency injected on the agent probe, and screenshots of both states from the real build rather than the mockups above.

## 5. Not in scope

Korp's IPC stall (#7895) and srv's own latency under load; agent-initiated and cross-window closes; making `ClosePane` itself faster (it already runs after the pane is gone).

## 6. Decisions I made (say if you want them the other way)

- The label shows at once rather than after a delay: a close that is fast never paints it, and one that is slow needs it from the first frame.
- A slow agent probe keeps the pane closing instead of skipping the prompt.

## 7. Build notes (task #7983)

- `closeNode` and `closeBlockInStack` set the state before their first await and clear it in a `finally`. `beforeNodeDelete` gets a second argument, `hooks.asking(answer)`, which a prompt wraps its wait in: the pane shows its normal look while the prompt is open and goes back to closing on yes.
- `beforeNodeDelete` asks agents only (`membersToProbe`); a browser still gets its leave-site check. A declined prompt leaves a quiet "Not closed" notice naming why.
- `__closeTrace()` lists the last 50 closes with their phases (`probes`, `prompt`, `reducer`, `gone`, `closePane`, or `kept`/`failed`); a close slower than 500 ms to leave logs one console line naming its slowest phase.
- An agent pane closed in place under its shutdown log (§2, last bullet) keeps the log as its indicator; it does not get the header label, since the pane leaves the closing state when the log takes over.
