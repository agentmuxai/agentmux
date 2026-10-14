# Spec: an agent pane holds a bounded amount of tool output text

**Date:** 2026-10-13
**Status:** active. Phase 1 (§2 items 1–4 and 6–8) is implemented with this spec; phase 2 (item 5, inputs) has its own PR.
**Plan item:** #7 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.
**Builds on:**
- `SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01.md`: U1 and U2 shipped in #4125/#4126. That spec's status line still says "proposed — nothing here is built", which this PR corrects.
- `SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md` §6.9 (the live feed).

## 1. The problem

**Measured:** after the 12-agent lab session, with tool bodies already unmounting on collapse (#4756), the main window's JS heap was 218 MB, about 97 MB of it strings. Most was tool output (Bash output, Read file contents, Grep results) held in the panes' document stores.

The live feed caps finished turns at 1 MB per pane (`LIVE_FEED_MAX_FINISHED_BYTES`), and collapsed results of 8 KB or more are already unloaded (`tool-result-unload.ts`). The code shows why text still builds up.

### 1.1 What isn't unloaded

`planUnload` (`tool-result-unload.ts:54-78`) skips a result when any of these holds:

1. **It's in the newest turn,** even after that turn finished (`i < lastUser`). Roll-off also always keeps the last turn (`live-feed.ts:224-236`).
2. **Its turn never ends.** Turns split only at `user_message` (`live-feed.ts:83-94`). An agent driven by jekts or agent messages after one user message is one turn that grows without limit.
3. **It's held open.** Every tool that finishes while its row is mounted joins `heldOpenNodes` (`ToolBlock.tsx:194-217`).
   - The release scan skips hidden panes, because their rect is 0×0 (`AgentDocumentVirtualList.tsx:1200-1226`).
   - So panes in background tabs keep these until their turn rolls off.
4. **It's under 8 KB.** Many Read and Grep results are 3–8 KB.
5. **Its source is a `b:` stream** (only `g:` unload).
6. **It's a tool's input, not its result.** Edit's `old_string`/`new_string` and Write's content are never unloaded.

Passes run only on events: history ready, turn end, next send, going hidden, and over budget, which fires once (`useLiveFeedRollOff.ts:114-190`). A long turn gets no pass at all.

### 1.2 Other copies

- **The global dispatch log** (`command-source.ts:57,73-87`, `RING_CAPACITY = 500`) keeps the full command of every agent-document dispatch (`agent-document-store.ts:204-211`). That includes:
  - `StreamFlush` new and updated nodes;
  - `HistoryRestored` nodes (a whole restore, about 1 MB per pane);
  - `ResultLoaded` results;
  - `ToolChunkAppend` chunks.

  Unloaded or rolled-off text stays reachable until 500 later dispatches. When agents go idle, the last 500 stay pinned. Nothing in production reads the log.
- **Shiki's token cache** (`preview-text/highlight.ts:27-50`): 200 entries of up to 200 KB of code each, keyed by the full code text.
- **`HighlightedCode`'s cache** (`components/HighlightedCode.tsx:35`): no bound at all. It serves Files previews and PowerShell commands, not tool output, but it's the same mistake.
- **The History tab** (`AgentHistoryView.tsx`): a full copy (raw lines + parsed nodes) while it's open, growing with paging.

## 2. The rule

**A pane's tool text is bounded at all times, not just between turns:**

1. **Unload inside the turn.**
   - Drop the `i < lastUser` condition. A finished tool whose row isn't pinned, isn't open, and isn't in the Activity Dock can be unloaded in any turn, including the one in flight.
   - Its row stays; only its text goes, as for older turns.
2. **Count jekts and agent messages as turn boundaries,** for unloading and roll-off: `jekt_message` and `agent_message` start a turn, as srv already counts them. One long-running agent then rolls off like any other.
3. **Release holds in hidden panes.**
   - When a pane has been hidden for 30 s, clear its `heldOpenNodes`.
   - The hold exists so a row the user is watching doesn't collapse under them; a hidden pane has no watcher.
   - This lines up with the row hibernation spec's grace (`SPEC_AGENT_PANE_HIDDEN_ROW_HIBERNATION_2026_10_13.md`).
4. **Unload smaller results:** `UNLOAD_MIN_BYTES` from 8 KB to 2 KB. The stub (pill and counts) is under 200 bytes, so anything above 2 KB is a real saving. Reloading is one indexed line read.
5. **Unload large inputs too (phase 2).**
   - An Edit's `old_string`/`new_string` and a Write's `content` above 2 KB are replaced by the same kind of stub and reloaded with the result.
   - The header keeps what it shows: path and line range.
   - Phase 2, its own PR: reloading needs the transcript line of the tool's `tool_use` record, which nothing records today (`resultSource` is the `tool_result` line).
6. **Run passes on a budget, not only on events.** While a pane's live text is over its budget, a pass runs at most every 10 s (idle callback). The `feedOverBudget` trigger fires once today.
7. **Stop the dispatch log keeping payloads.** `recordDispatch` keeps only the command's type, the slice, and the sizes (node count, bytes). It's diagnostic metadata, and nothing reads the payloads.
8. **Bound the code-highlighting caches by bytes, not entries:**
   - Shiki's token cache: 8 MB total, LRU.
   - `HighlightedCode`: an LRU of 200 entries and 8 MB.

**Budget per pane:** the live feed's 1 MB of finished turns, plus at most 512 KB of loaded tool text in the turn in flight. Text for rows that are pinned or open is outside the budget, because the user asked for it.

## 3. What a user sees

- **Nothing,** for rows they don't open.
- **Opening an unloaded row** reloads it from the transcript, as U2 already does: one indexed line read, a few milliseconds plus parsing one line.
- **The failure case is unchanged:** "This result is no longer available in the pane. Open History to see it." It now logs why (stream/gen mismatch, missing line, timeout), which today it doesn't.

## 4. Tests

- **`tool-result-unload.test.ts`:**
  - a finished, unheld tool in the turn in flight is unloaded;
  - a pinned one isn't, and one in the Activity Dock isn't;
  - a 3 KB result is unloaded;
  - an Edit's large `old_string` is stubbed and reloaded.
- **`live-feed.test.ts`:** a jekt starts a turn for roll-off.
- **`useLiveFeedRollOff.test.ts`:**
  - a pass runs every 10 s while over budget;
  - a pane hidden 30 s releases its holds (the existing test stubs `hidden` as false; add the hidden case).
- **`command-source.test.ts`:** a recorded dispatch holds no node payloads.
- **Cache tests:** the highlight caches evict by bytes.

## 5. Verification in the lab

- **Memory:** the 12-agent run from the root-cause report, with #4751 and #4756 in. Expect the main window's strings under 20 MB, against 97 MB, and the JS heap to fall by about that much.
- **Reloads:** open 20 unloaded rows across panes and record reload time and failures. Expect zero failures and p95 under 50 ms.
- **The jekt case:** one agent driven only by jekts for 100 turns. Its pane stays within the budget.

### 5.1 Results (2026-10-13)

Isolated production portables built from the same `main` (with #4751 and #4756), with and without this change; 12 scripted agents over 4 tabs, 40 rounds of one turn per agent; measured after a forced GC.

| Main window renderer | `main` | With this change |
|---|---|---|
| Private working set | 962 MB | 842 MB (−120 MB) |
| JS heap used | 247 MB | 162 MB (−35%) |
| Strings in the heap snapshot | 98.6 MB | 27.9 MB (−72%) |
| Heap snapshot, all objects | 355 MB | 242 MB |
| DOM nodes | 107,000 | 66,000 |

Strings came down by 71 MB but not under the 20 MB this section expected; what remains is not yet broken down.

Reloads: 30 finished rows opened across the visible panes after 5 rounds, 16 of them unloaded and read back: no failures, p50 17 ms, p95 39 ms, max 39 ms.

The jekt case has not been run: the lab drives agents by sending them messages, not jekts.
