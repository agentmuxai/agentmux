# Status: Agent Pane History and Memory — Bake Tracking

**Status:** living — all six PRs merged 2026-10-01; baking, not yet in a release; one live check pending (§5)
**Date:** 2026-10-01
**Verified against:** `86eb025e3` (main, after #4126)

---

## 1. Why this document exists

Six PRs landed on 2026-10-01 that change what the agent pane keeps in memory
and when conversation moves to History. They interact (the roll-off pass now
also runs result unloading), so they are tracked here as one unit while they
bake: what shipped, what to watch for, what diagnostics exist, the known
limitations, and what is still open.

## 2. What shipped

| PR | Change | Merge commit |
|---|---|---|
| #4115 | Report: why roll-off cut history too aggressively | — |
| #4121 | The live feed is bounded by size (15 MB) and rows (20,000), not by turns | `9b9e28e39` |
| #4123 | No enter animation on the user's own message (the send flash) | `4cb8a1c36` |
| #4124 | Spec: unloading collapsed tool results | `05fd3570a` |
| #4125 | U1: a finished tool's live log is freed once its result lands | `dc9808f09` |
| #4126 | U2: collapsed tool results ≥ 8 KB are unloaded; opening a row reads the result back from the transcript | `86eb025e3` |

Docs:

- `docs/reports/REPORT_LIVE_FEED_ROLL_OFF_TOO_AGGRESSIVE_2026_09_30.md` — root cause, owner direction (§6), flash cause (§7).
- `docs/specs/SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01.md` — U1/U2 design.

### 2.1 Behaviour now

**Live feed (#4121).** Finished turns roll off the front of the pane into
History only when the pane holds more than 5 MB of finished content (15 MB as shipped in #4121; lowered on 2026-10-02, see the change log) or more
than 20,000 rows. There is no turn limit by default; `agent:livefeedturns`
is now an optional cap. A pass runs on send, turn end, the pane being hidden,
history load, and whenever the document first goes over its limits (or the
reader returns to the bottom while it is over them). Rows on screen, pinned
rows, and the turn in flight are never removed. Nodes are measured up to
32 MB each for this budget (`nodeBytesFull`), not the 2 MB cap the streaming
tail uses.

**Restore and paging.** A pane opens with the last 10,000 transcript lines
(`RESTORE_WINDOW_LINES`; the backend caps one read at 10,000). Scrolling up
pages in 2,000 more lines at a time, but only while nothing has rolled off
yet; after that, earlier turns are in History.

**Send flash (#4123).** The user's own message no longer fades in. Measured
layout shift on send went from 0.09 to 0.00.

**U1 (#4125).** When a tool finishes with a result, its streamed log chunks
are dropped; the row renders the result. Kept: an accepted background
launch's log, and a long call's log while its Activity Dock row shows it
(the dock needs stdout/stderr in stream order, which the result can't
rebuild). Late chunks for a freed log are dropped silently.

**U2 (#4126).** In the same idle pass as roll-off — and also when the live
feed is off — finished, collapsed tool results of 8 KB or more, before the
turn in flight, are replaced by a stub (pill, size, token count). Opening
the row reads the one transcript line back (`blockfile:read_range`, limit 1)
and re-parses it. Kept in memory: pinned or open rows, content-first tools,
rows still in the Activity Dock, results from a block-local (`b:`) stream.
The same pass frees the logs U1 kept once the dock has let the row go.

## 3. What to watch while it bakes

| Symptom | Likely area | What to capture |
|---|---|---|
| "Open History" appears after only a few messages | Roll-off (#4121) | Pane's agent, rough session length, whether a new session started |
| Conversation jumps or the scroll position moves during work | Roll-off pass while reading | Was the reader at the bottom? Did a turn just end? |
| A tool row says "This result is no longer available in the pane" | U2 reload failed | Tool name, how old the turn is, whether the agent restarted or compacted since |
| "Loading result…" stays up | U2 reload hung | Same as above, plus DevTools console |
| A dock row goes blank or its output reorders after the call ends | U1/U2 dock interplay | Command, how long it ran |
| Flash on send is back | #4123 selector | Screen recording |
| Pane gets sluggish or renderer memory climbs in a long session | Budget not holding | Renderer memory (Task Manager), session length, heavy tool use? |

## 4. Diagnostics

What exists today:

- `[live-feed] <block>: N older turn(s) kept (not in the transcript)` — `console.debug`, when a pass had to keep turns it couldn't hand to History.
- The reducer emits `turns-rolled-off`, `tool-results-unloaded` (count, bytes) and `tool-result-loaded` events, but **nothing logs them**.
- A failed U2 reload is **silent**: `readToolResult` returns `null` for every failure (no source, read error, stream or generation mismatch, line missing, tool not found), and the row shows the "no longer available" message.

Gap: from a user's report alone there is no way to tell how often roll-off or
unloading runs, how much it frees, or why a reload failed.

**Proposed D1 (not started):** small, log-only.

- One `console.info` per roll-off pass that changed something: turns and rows removed, bytes before and after.
- One `console.info` per unload pass: results unloaded and bytes, logs freed.
- A `console.warn` on a failed reload with the reason (`no-source`, `read-error`, `stream-moved`, `gen-moved`, `line-missing`, `not-found`), and its stream, gen and line.

## 5. Pending verification

- **U2 live check.** Not yet exercised in a running app. In a `task dev` window: open an agent, have it read 3–4 files over 8 KB, send one more message; collapsed rows should show their pill, and opening one should show "Loading result…" and then the full result. Owner action: open the agent and run the reads.
- #4123 was measured live over CDP before merge (layout shift 0.09 → 0.00). #4121 ran in a portable build, but the 15 MB roll-off point was not measured live. U1 is covered by tests only.

## 6. Known limitations and open items

Limitations (by design for now):

1. Only results from a global (`g:`) stream unload. A pane that only ever has its block-local `b:` file keeps all its results.
2. An archived block reads from its `b:` file; a `g:` stub there can't reload and shows "no longer available".
3. An accepted background launch whose task notification never arrives keeps its log.
4. A replayed chunk for a dock-kept log that was later freed is buffered again (rare: replay only).
5. The pane does not fill up to the 5 MB budget on open; it restores 10,000 lines.
6. Once anything has rolled off, scroll-up paging stops; earlier turns are in History.

Open items:

| ID | Item | Needs |
|---|---|---|
| F2 | Fresh-session clamp hides earlier history; show a divider instead | **Decided 2026-10-01: divider.** In progress |
| F3 | Fill the pane to the 5 MB budget on open (beyond 10,000 lines) | Owner decision; backend read cap |
| F4 | Backend `is_claude_turn_start` counts jekts as turns (matters only with a turn cap set) | Small fix |
| D1 | Diagnostics in §4 | Go-ahead |
| V1 | U2 live check (§5) | Owner to open the agent |

## 7. Change log

- 2026-10-01 — created after #4126 merged.
- 2026-10-02 — budget lowered from 15 MB to 5 MB, and the "Loading older messages..." banner removed. With 15 MB, scrolling up in a long-running agent (Manoz: about 68 MB of displayed history) paged in 2,000-line pages for hundreds of loads before anything rolled off, each page flashing the banner and pushing the rows down; the pane slowed down. The banner dated from #338 and had been unreachable since #3700 turned paging off with the live feed on; #4121 made it reachable again.
