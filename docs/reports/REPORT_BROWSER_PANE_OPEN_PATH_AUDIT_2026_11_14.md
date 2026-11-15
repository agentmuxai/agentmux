# Audit: the path from a click on the Browser widget to a page that starts loading

**Date:** 2026-11-14. Code read at `origin/main` (`360fa3c04`). Timings are from host logs of installed 0.60.23 to 0.60.26 and of a task dev build on Windows 11.
**Task:** #47590. **Author:** Camper@narko.
**Status:** analysis. The findings marked "fixed" are fixed by the pull request that carries this report; the rest are ranked follow-ups, not yet built.
**Readers:** the operator, AgentX, AgentY, whoever takes a follow-up.

The operator's words (2026-11-14): "do you have telemetry tracking opening a browser pane? simply opening one appears to be way too long. we need to audit the path there, make sure it is clean, DRY and optimized."

## 1. The answers in short

- **Telemetry.** The host half of an open is timed by one `[pane-open]` line per open and per navigation (queue, create, engine, load, total; a warning at 2 s). The window's half, from the click to the create request, had no timing. This pull request adds it: the pane's `createPane` log line now ends with `[pane-open:fe] create_block=… mount=… total=…`. Section 3 lists every signal.
- **Why it was slow.** In a source-tree (dev) build each open and each navigation froze the window for 0.5 to 7 s: the host started `git` on its UI thread at every page event. That is fixed (#7259): 2221 ms to 19 ms from the create request to the load starting, on the same machine. An installed build never had it and takes 12 to 42 ms for that stretch, in either pane kind (33 opens on 0.60.23 to 0.60.25).
- **What is left of the open.** Three things the page waits for, in order: the backend creating the block (about 40 ms in one logged open of a dev build), the first Browser pane of a session loading its code, and one host call. No timer sits in that path.
- **Clean and DRY.** The path does the same thing more than once in several places. None of the repeats delays the page request; they cost backend calls, re-renders and log volume, and two of them were real bugs (sections 4.1 and 4.2).

## 2. The path

1. **Click.** `openWidget` (`frontend/app/window/action-widgets-config.ts`) calls `createBlock`.
2. **Block.** `createBlock` (`frontend/app/store/block-layout-actions.ts`) waits for the backend's `CreateBlock`: a state lock, one SQLite write, the reply with the new block and its tab. **The page waits for this.**
3. **Store.** The reply's objects go into the object store (`frontend/app/store/mos.ts`).
4. **Layout.** The new node is inserted with focus; the layout change is sent to the backend without waiting.
5. **View.** `lazyPaneTab.create` (`frontend/app/block/lazy-pane-tab.tsx`) builds the pane. The first pane of a type in a session shows a stand-in until `import()` of its code resolves. **The page waits for this, once per session.**
6. **Model.** `BrowserViewModel` registers the pane, subscribes to the host's events and saves the address to the block.
7. **Mount.** `usePaneRectSync` (`frontend/app/view/browser/use-pane-rect-sync.ts`) reads the pane's rectangle and sends `browser_pane_create`, straight from `onMount`: no animation frame, observer or timer in between.
8. **Host.** `browser_pane_create` (`crates/cef/src/ipc.rs`) registers the pane and posts its creation to the UI thread. The call returns before the browser exists.
9. **UI thread.** `CreateBrowserPaneTask` (`crates/cef/src/browser_pane/creation.rs`) makes the pane's window and asks CEF for a browser. CEF calls back: `on_before_browse`, `on_after_created`, `on_loading_state_change`, `on_load_end`.
10. **Back in the window.** Each callback sends a nav-state event; the model updates the address bar, history and loading state, and saves the address.

## 3. What is measured

| Signal | Where | What it says |
|---|---|---|
| `[pane-open] block=… open via=… queue= create= engine= load= total= slowest=` | host log (`crates/cef/src/browser_pane/open_timing.rs`, #7257) | One line per open and per navigation, from the create request to the load's end. Warns at 2 s. |
| `[pane-open:fe] create_block= mount= total=` | host log, on the pane's `createPane` line (this pull request) | The window's part: the wait for the block, then the code, view and first layout, up to the create request. Only for a pane this window just opened. |
| `[ui-stall] the UI thread took no message for N ms` | host log (#7259, Windows) | The host's UI thread was blocked. `AGENTMUX_UI_STALL_STACKS=1` adds the stack. See `docs/debug/RUNBOOK_UI_THREAD_STALL_WINDOWS.md`. |
| `[pane-trace] seq= block= event=` | host log | The order of creation events for one pane. |
| `[perf] ipc <name> <N>ms` | host log | A window-to-host call that was slow. |

Window log lines reach the host log in batches of about 100 ms, so their timestamps are not exact; the durations inside `[pane-open:fe]` are measured in the window and are.

**Still not measured:** the first pane's code load on its own (it is inside `mount`), and the time from the host's reply to the first painted frame of the page.

## 4. Findings, ranked

Ranked by what a person feels first, then by wasted work.

### 4.1 Fixed in this pull request

| # | Finding | Effect | Change |
|---|---|---|---|
| 1 | **The first pane of a type waits for its code.** Nothing loaded a lazy pane type's code ahead of its first use. | The first Browser pane of a session opens behind a stand-in for as long as the load takes. On the page's path. | `PaneTabManifest.preload`; the widget bar calls it when the pointer enters a widget, so the click finds the code loaded. It covers every lazy pane type. A failed preload is forgotten and the pane loads again. |
| 2 | **An empty address from the host cleared the tab's address.** The first nav-state event of a new pane can carry `url: ""` (no main frame yet). The model took it as the confirmed address. | The address bar and title blanked for a moment, the "enter an address" state could show while the pane was being created, and the empty address was saved to the block, where a restore would read it. | `onNavState` ignores an empty address: no confirmed-address event and no save. History and loading state are still taken. |
| 3 | **Every object that arrived as an update was fetched again, and the late fetch won.** `updateMuxObject` created a store entry with a fetch for any object it had not heard of. The fetch's result replaced what was there with no version check. | One extra backend call per new pane, tab and layout, a second write of the block (which re-ran everything that reads it), and a window in which a newer update could be replaced by the older fetch. | An unknown object's entry is created without a fetch. A fetch result that is not newer than what is there is dropped. |
| 4 | **The header icon was rebuilt on every write to the block.** The icon memo read the whole block. A browser pane writes its block several times as it opens. | The icon element was rebuilt and re-inserted, and a diagnostic line logged, per write and per header. | The memo depends on the favicon, the icon and the icon colour only. |
| 5 | **No timing for the window's half of an open.** | The question "which half is slow" could not be answered from a log. | `[pane-open:fe]` (section 3). |

### 4.2 Not fixed here, ranked

| # | Finding | Effect | Why not in this pull request |
|---|---|---|---|
| 6 | **Six senders of a focus request on one open.** The frame's focus-in reclaim, the layout insert, the layout selection effect one frame later, the block's mount effect, the stand-in's claim (before the pane exists) and the claim after create each send `input_select`. Nothing de-duplicates them, and the first one names the app, not the pane. | Up to six host calls, each a reducer step on the UI thread. The last one wins, so the order decides who has the keyboard. | It is the input path that the overlay click fix (#7263) just changed. It needs one owner for "focus the new pane" and a check with a real click, as its own change. |
| 7 | **Several saves of the block per open.** The address is saved at navigate, again at each nav-state event, and the load time once more. Each is a SQLite transaction on the backend and a broadcast back. | Six to eight writes and broadcasts per open (estimated from the code). Each broadcast is dropped by the store's version check, so the cost is on the backend. | Wants a compare-before-write on both sides; small, but it touches every pane type's `setMeta`. |
| 8 | **A settings write and a full-config broadcast per open.** The recently-used list lives in `settings.json`; opening a widget that is not first in it rewrites the file and broadcasts the whole config to every window. | A disk write and a broadcast per open. Not waited for. | A storage decision: the list should not live in settings. |
| 9 | **The block round trip gates everything.** Nothing else starts until the backend has created the block. | About 40 ms in one logged open of a dev build. Not yet measured in an installed build; `create_block=` will show it. | Removing it means the window picks the block's id so the view and the block are created in parallel. A design change. |
| 10 | **The pane's rectangle is sent again right after create.** The create call carries the rectangle, but the sync does not record it as sent. | One extra host batch per open. | In an overlay pane that second send may be what shows the pane. Not safe to remove without checking that path live. |
| 11 | **Per-pane repeats on the host's UI thread.** The window icon is set on every pane window; the pane's label is resolved by scanning all browsers in four places; password-manager preferences are re-read per pane; a trace label is computed with tracing off. | Small each, per pane. | Low value against the risk of touching creation. |
| 12 | **A load-time sample can be recorded for a load that was never seen starting.** | A wrong estimate for a site's load time. Read from the code, not seen in a log. | Arises mainly when the window is stalled. |
| 13 | **Steady-state work per open pane.** A 200 ms rectangle poll and a 250 ms sign-in-offer clock run for every pane; each host event runs every pane's listeners; about 20 diagnostic lines per open. | Not on the open path. Grows with the number of panes. | Outside this task. |

### 4.3 The same thing written more than once

- The pane label parse (`browser-pane-<block>-<n>` to a block id) is written out in several places in `crates/cef/src`, two of them identical lines in `input_owner/native_click.rs` and `input_owner/views.rs`. One helper should own it.
- "Focus the pane that was just opened" has six owners (finding 6).
- The pane's visibility accessor is built three times per pane (`pane-tab-host.tsx`, `use-pane-rect-sync.ts`, `use-pane-sleep.ts`).
- The address of a pane is saved from three places (finding 7).

## 5. Not determined

- Which host callback sends the first, empty nav-state. By reading it is `on_loading_state_change` before `on_before_browse` has run. The window no longer depends on the answer.
- The size of the Browser pane's code chunk, and so how long the first pane waited. `[pane-open:fe] mount=` on a session's first open will show it.
- Whether the window's calls to the host (create, focus, rectangles, log batches) queue behind each other on the same connections.
- Whether the per-pane DevTools session the host attaches for side buttons slows a pane's first load.

## 6. What to do next

1. Read `[pane-open]` and `[pane-open:fe]` from the operator's own opens in the next release, and rank the rest of this list by those numbers.
2. One owner for a new pane's focus (finding 6), with a real-click check.
3. Compare before saving the block (finding 7) and move the recently-used list out of settings (finding 8).
4. One helper for the pane label parse.
