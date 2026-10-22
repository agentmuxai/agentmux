# Report: which renderer process runs which window, tab and pane

**Date:** 2026-10-19
**Status:** analysis. Read from the code on `main` @ `f0b2df57c` and from the process table of one running instance (0.59.32, Windows). Nothing changed.
**Author:** korp
**Trigger:** Operator, 2026-10-19: *"to be clear .. each pane has its own renderer, and the agentmux window does too? in agentmux-docs do we have a solid explanation of how the panes/tabs/windows interact with the renderers?"*
**Related:** `docs/reports/REPORT_MEMORY_FOOTPRINT_500MB_BUDGET_2026_10_12.md` §3 (the first inventory), `docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md` (reload and the governor), `docs/specs/SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md` §5 (which pane tabs stay mounted), `docs/reports/REPORT_WINDOW_TAB_SWITCH_PANES_APPEAR_AT_DIFFERENT_TIMES_RCA_2026_10_19.md` (what the split means for tab switches).

---

## 1. Short answer

**Not every pane has its own renderer. Every window does, and every browser pane does. Every other pane shares its window's renderer.**

- A **window** (the main window, a second window, a torn-off floating pane) is one Chromium browser that loads the AgentMux frontend. It gets one renderer process. **All of that window's tabs, and every pane in them except browser panes, run in that one process:** agents, terminals, editors, files, Swarm, Tower, sysinfo, settings, widgets.
- A **browser pane** is a separate Chromium browser that the host creates and lays over the window. Its web page runs in **its own renderer process**, and sometimes in more than one (§3.3). The window's renderer only holds the pane's frame: the bar with the address and buttons, and an empty placeholder the page is positioned over.
- **A floating pane is a window of its own**, with its own renderer running its own copy of the frontend (`floating-pane-workspace.tsx`), not the renderer of the window it came from. Tearing a pane off unmounts it in the source window and rebuilds it in the floater from srv; docking it back does the reverse. A browser pane is closed and recreated on either move (`crates/cef/src/browser_panes/mod.rs`, `AlreadyLiveElsewhere`): it keeps its cookie jar, so it stays signed in, but the page reloads.
- **Hidden pre-warmed windows** (two for new windows and tear-offs, one for floating panes) each have a renderer too, loaded and waiting.

So "this window is using a lot of memory" almost always means the window's own renderer, which carries every non-browser pane of every tab in it, hidden ones included.

## 2. The processes of one instance

| Process | How many | What runs in it |
|---|---|---|
| launcher (`agentmux-launcher`) | 1 | lifecycle, restarts, window reality reconciliation |
| srv (`agentmux-srv`) | 1 | all durable state, agents' CLIs as its children, transcripts |
| host (`agentmux-cef`, Chromium's browser process) | 1 | windows, browser panes, IPC, the GPU and utility children below |
| GPU process | 1 | compositing and WebGL for every renderer (terminal WebGL included) |
| utility processes | a few | network service, storage service, audio service |
| **window renderer** | 1 per window, 1 per floating pane window | the SolidJS frontend: every tab and every non-browser pane of that window |
| **pool renderers** | 3 (`POOL_TARGET_SIZE = 2`, `PANE_POOL_TARGET_SIZE = 1`), more kept briefly under normal pressure | a hidden window that has loaded the frontend and waits to become the next new window or tear-off |
| **browser pane renderer** | 1 or more per browser pane | the web page shown in that pane |

The agent CLIs (Claude Code, Codex, …) are not renderers and not Chromium processes: they are srv's children and have their own budget (memory budget report §7).

## 3. What is in each kind of renderer

### 3.1 A window's renderer

- **Every window tab of the window is mounted at once.** An inactive tab is hidden, not unloaded: with `window:keepinactivetabslaidout` (the default) it stays laid out under `visibility: hidden` (`frontend/app/workspace/workspace.tsx`, `window-tab-visibility.ts`). Its DOM, its stores and its terminals' buffers stay in memory.
- **Inside a pane, the pane tabs you aren't looking at** stay mounted if their type is `keepAlive` (agent, terminal, browser, editor, files) and are unmounted otherwise (Help, sysinfo, Swarm, …), per `SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md` §5 and each view's manifest.
- **A hidden agent pane** stops drawing new output (`agent-dormancy.tsx`) but keeps receiving and storing its stream, so the memory it holds keeps growing with what the agent does.
- **A minimized pane** is a display mode (`frontend/layout/lib/layoutMinimize.ts`): it is drawn as a header chip and stays mounted.
- **A minimized window** counts as dormant for agent panes (#5172): they stop drawing, and keep their memory.
- **Widgets** run in a sandboxed `<iframe>` with an opaque origin inside the window (`sandboxed-widget-host.tsx`). Chromium may give such a frame a process of its own; not measured here.

Blink's heap does not shrink after things are freed (recycle spec §1: 89–92% of a long-running renderer's heap was empty). A window renderer therefore stays near its high-water mark until the page is reloaded.

### 3.2 A pool renderer

A pool window is a real window kept hidden, with the frontend loaded, so a new window or a tear-off appears without a cold start (`crates/cef/src/commands/window_pool.rs`, `pane_pool.rs`). Promoting it turns its renderer into that window's renderer; the pool then makes another.

### 3.3 A browser pane's renderer

- The host creates each browser pane as its own `CefBrowserView` (`crates/cef/src/browser_pane/creation_views.rs`). Chromium places the page in a renderer by its usual rules: no AgentMux switch changes the process model (`crates/cef/src/app/mod.rs` sets none of `process-per-site`, `site-per-process`, `renderer-process-limit`). In practice that means:
  - one renderer for the page;
  - **another for each cross-site `<iframe>`** on it (site isolation);
  - a navigation to another site moves the pane to a new renderer and lets the old one exit;
  - Chromium can keep a **spare renderer** ready for the next navigation;
  - past its process limit Chromium may let same-site pages share a renderer, so "one per pane" is the normal case, not a guarantee.
- **The cookie jar doesn't change the count.** Personal, a named profile and Incognito (`crates/cef/src/browser_pane/identity.rs`) are request contexts: they decide which cookies the page sees, not how many processes it uses.
- **Hiding a browser pane doesn't free its renderer.** In a hidden window tab, an inactive pane tab or a minimized pane, the page is collapsed to 0×0 (`frontend/app/view/browser/use-pane-rect-sync.ts`) and keeps its whole renderer: DOM, scripts, images.

### 3.4 Since 2026-10-13, windows share a cookie jar but not a renderer

`secondary_window_request_context()` (`crates/cef/src/commands/mod.rs`) now returns the shared default context; before that, each secondary window had an in-memory context of its own. Either way each window is its own browser, so Chromium gives it its own renderer. The recycle governor skips a renderer that serves more than one window (recycle spec §4.1), because Chromium can do that under its process limit.

## 4. What frees memory, and what doesn't

| Action | Window renderer | Browser pane renderer |
|---|---|---|
| Switch to another window tab | nothing freed | nothing freed (page collapsed) |
| Switch to another pane tab | a `remount` type is unmounted; `keepAlive` types keep everything | nothing freed (page collapsed) |
| Minimize a pane or the window | nothing freed | nothing freed |
| Close a pane | its share, but the heap mostly keeps the pages | the whole process exits |
| **Reload Window (Free Memory)** (#5086) | reloaded: the heap starts over, panes restore from srv | the page re-attaches and keeps its state (#4944) |
| **Recycle governor** (#5092) | the same reload, automatically, only for a window over 600 MB and twice its post-load size that hasn't been in front for 10 minutes (2 under commit pressure), never one with recent input, unsaved editor changes or an agent waiting for an answer; off with `window:autorecycle: false` | not touched |

The governor never recycles the window you are working in. A window that is always in front only shrinks by Reload Window or a restart.

## 5. Measured on one instance

The operator's instance, 0.59.32, one window with four tabs, 2026-10-19, private working set from Windows (the same metric as Tower):

| Renderer | Started | Private |
|---|---|---|
| The window's renderer | with the instance, a day earlier | **≈ 3.8 GB** (2.4 GB resident) |
| Four renderers started with the instance (pool windows, likely) | same second | 31, 69, 262, 264 MB |
| Browser-pane renderers (9 browser panes open across the 4 tabs, some pages with cross-site frames) | as each pane was created or navigated | 16 to 197 MB each, **≈ 1.15 GB** together |

- The two GitHub panes in Tab 1 were about **111 MB** and about **185 MB**. These were matched by start time against the host log's `browser pane created` lines, which is exact for one and narrows the other to two processes started in the same second.
- The window's renderer, which carries the four tabs' agents, terminals, Swarm, Tower and charts, is **over three times all the browser panes together** and over seven times the whole app's 500 MB budget. It is the window in use, so the governor (§4) leaves it alone.
- **Exact attribution exists.** Each renderer reports its PID to the host when a browser is created in it (`crates/cef/src/renderer_map.rs`), and Tower uses that map to label renderers by window or pane (`crates/srv/src/backend/tower_agentmux.rs`). Tower's view is the reliable way to read this; the start-time matching above is a fallback for a script that can't call Tower's RPC (agents can't: it refuses agent connections).

## 6. What the docs say today

`agentmux-docs` covers this in single sentences, and gets one part wrong:

- `internals/architecture.md` (the process table) and `glossary.md` (`renderer`, `browser pane`) say each window and each browser pane gets a renderer. Correct, but:
  - the glossary says a renderer "runs the SolidJS frontend JS for one browser context". **A browser pane's renderer doesn't run the frontend**: it runs the web page.
  - neither says that all tabs and all other panes of a window share its renderer, which is the fact that explains where memory goes;
  - neither covers pool renderers' count, cross-site frames, spare renderers, what hiding or minimizing does, or what frees memory.
- `pane-types.md` calls the window's page "the main React webview" (the frontend is SolidJS).
- **Reload Window (Free Memory)** and the recycle governor (`window:autorecycle`) aren't documented anywhere in `agentmux-docs`.

**Proposed:** a new Internals → Architecture page, *Windows, tabs, panes and renderers*, holding §1 to §4 of this report for readers, linked from the architecture overview and the glossary, plus the two corrections above. Drafted alongside this report in `agentmux-docs` (`src/content/docs/internals/renderers.md`).

## 7. Open questions

1. How much of the 3.8 GB is hidden tabs. The root-cause report measured hidden agent panes at the same size as visible ones (§4.4 there); a heap snapshot of this window would split it by tab. This decides how much hibernation (parked, P4.4 of the performance plan) would return.
2. Whether Chromium is isolating the sandboxed widget iframes into their own processes in this build.
3. Whether a spare renderer is being kept; the 16 MB browser-side renderer started last fits one, but it wasn't confirmed.

Agent: korp@narko
