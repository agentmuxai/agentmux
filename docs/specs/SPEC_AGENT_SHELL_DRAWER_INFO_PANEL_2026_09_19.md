# SPEC: Agent pane shell drawer — replace the Session/History bar with a process & shell info panel

**Date:** 2026-09-19
**Status:** proposed
**Related:**
`docs/specs/SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md` (where the drawer renders),
`docs/specs/SPEC_AGENT_PANE_SHELL_EXIT_COLLAPSES_DRAWER_2026_09_15.md` (drawer close on `exit`),
`docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md` (the agent shares this shell),
`docs/specs/SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md` (pane-close confirmation, which uses the same process list),
`docs/specs/SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md` (the History tab),
`docs/specs/SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md` (the strip's other panel, whose open/close and floating mechanics the stats popover mirrors),
`docs/specs/SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31.md` (removed the strip stats that `sessionTotals` used to feed),
PR #3430 (process list now counts only agent-started processes; composer `⚙ N` badge removed).

---

## 1. Problem

The drawer the composer's **Shell** button opens (`.agent-composer-details`, `agent-view.tsx`) has two parts:

1. `AgentControlBar` on top: session-lifecycle UI.
2. `ResizableDetailsDrawer` → `AgentShellSubblock`: the xterm shell.

`AgentControlBar` is conversation material that ended up in the shell's space:

| Row | Shown when | Content |
|---|---|---|
| interrupted banner | `session:was_interrupted` | "Session was interrupted by a restart…" + Dismiss |
| resume-failed banner | `session:resume_failed` | "Couldn't resume the previous conversation…" + Dismiss |
| large-session banner | `session:line_count ≥ 500 000`, not archived | line count + Archive |
| archived banner | `session:archived_at` | Archived + Restore / Export |
| **Session** row | lineCount > 0, not archived/large | Archive · Export |
| **History** row | always (Claude and non-Claude) | View full history |

None of this is about the shell.

- The two banners that matter most (interrupted, resume-failed) are **invisible unless the user happens to open the Shell drawer**. A conversation-level warning is hidden behind a terminal toggle.
- "View full history" is already reachable from the pane body's context menu (`agent-model.ts` `getBodyContextMenuItems` → "Agent History") and from `AgentDocumentView`'s `onOpenHistory`.
- Archive/Export are reachable **only** here.

Meanwhile, nothing anywhere in the pane tells the user what is running on their machine on this agent's behalf:

- the dev server the agent started with `npm run dev &`;
- a watcher it left behind;
- the state of the drawer shell itself.

Until #3430 the composer's `⚙ N` badge tried to do this. It counted the agent's own plumbing (claude.exe, conhost, agentmux-mcp) and showed ≥3 for every idle agent. It has been removed; this spec gives that information a proper home.

**User direction (2026-09-19):** "remove the process count from the composer. instead, put the best stats in the drawer, replace the area session/history .. that shouldnt go there, have that area for process/shell type stuff useful info."

## 2. Goals / non-goals

**Goals**
- The top of the drawer answers two questions at a glance:
  - *What is this shell?*
  - *What has the agent left running?*
- One compact line by default. The terminal keeps its space; details expand on demand.
- Every process shown can be stopped from the drawer, without ever killing the agent CLI itself.
- Session/History UI moves somewhere at least as discoverable. The interrupted/resume-failed banners become **more** visible, not less.

**Non-goals**
- A general task manager. Only processes in this pane's tracked jobs are shown.
- Tracking on Linux/macOS. `new_tracker` is the stub there (confidence `none`), so the panel degrades (§4.4) until real trackers land.
- Changing the terminal itself (zoom, resize, scrollback, PtyShell lease).

## 3. Where the Session/History UI goes

| Element | New home | Why |
|---|---|---|
| interrupted / resume-failed banners | A notice row at the **top of the composer region**, above `AgentComposerStrip`, always visible while the flag is set. Same Dismiss behavior (clears the meta). | They describe what the next message will do, so they belong next to the composer. |
| large-session banner (≥500k lines) + Archive | Same notice row. | It is an actionable warning about the conversation. |
| archived banner + Restore / Export | Same notice row while archived. | Same reasoning. |
| Session → Archive · Export | The session-stats popover the strip's context reading opens (§3.1). Claude only, same gating as today (lineCount > 0, not archived). | User direction (2026-09-19). The numbers and the actions on them belong together, and the reading is already on screen every turn. |
| History → View full history | Dropped from the drawer. Already in the context menu and in `AgentDocumentView`. | Duplicate entry point. |

**Implementation.** Split `AgentControlBar.tsx` into `AgentSessionNotices` (the four banners, rendered in the composer region) and `AgentSessionStats` (the context reading turned trigger, plus its popover — §3.1). `AgentControlBar` itself goes away. The `.agent-session-row*` styles go with the rows; `.agent-session-btn` moves out of `.agent-view` nesting into a top-level `_session.scss`, since the popover renders through a `Portal` and would otherwise lose its chrome; the dead `.agent-process-badge` block (`_control-bar.scss:346-378`, unused since the control-bar badge was removed) is deleted in the same change.

### 3.1 The session-stats popover

**Revised 2026-09-19 (user):** "turn the stats label into a button (100k/1.0m) .. and the button opens that pane." The session UI does **not** become a fourth section of the Mode/Model/Effort dropup, as the first draft of this section proposed. The strip's context reading becomes the trigger, and it opens its own panel. Each button then opens what its own label is about: the runtime trigger opens runtime settings, the stats reading opens stats. It also makes the reading itself discoverable — it was inert text whose only affordance was a tooltip.

```
   [ 104k / 200k ]  ← the strip's ctx reading, now a button
   ┌ Session ──────────────────────┐
   │ Context  104k / 200k (52%)    │
   │ Cost     $2.41 · 37 turns · 18m│
   │ Tokens   1.2M in (89% cached) ·│
   │          46k out              │
   │ History  12k lines · 2h ago   │
   ├───────────────────────────────┤
   │ [Archive]  [Export]           │
   └───────────────────────────────┘
```

**Structure.** `AgentSessionStats` owns both the trigger and the panel, and renders inside the strip's existing `ctx` slot, so Compact keeps its adjacency and edge-priority ordering. The panel is a `role="dialog"` in a `Portal`, positioned with the same `computeMenuPosition` / `autoUpdate` anchor pair `AgentRuntimeDropup` uses.

**Consequence for the strip's layout rules:** the `ctx` slot's `interactive` flag becomes unconditionally true (it now contains a button even when Compact is absent). `auth` is then the only passive slot left, so the "passive content sits inward of interactive content" guard is exercised there instead.

**Styling trap** (found in review, worth keeping): the trigger is a `<button>`, so it needs its UA chrome neutralized — but a blanket `font: inherit` / `color: inherit` in a later-loaded stylesheet beats `.agent-composer-strip-ctx` at equal specificity and silently erases the mono face and the `--mid`/`--high`/`--critical` band colors, which are the compaction-pressure signal the reading exists to give. Inherit only what that class does not set, and give the class an explicit base color, since a button otherwise falls back to the UA's `buttontext`.

**Stats shown** (user direction: "include stats"). Every value already exists in the pane; none needs a new backend field:

| Line | Source | Notes |
|---|---|---|
| cost · turns · duration | `paneModel.state.sessionTotals` (`SessionStats.cost_usd`, `num_turns`, `duration_ms`) | Cumulative over the pane's lifetime, summed at each TurnEnd. **Currently rendered nowhere** — `sessionTotals` lost its only consumer when `SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31.md` removed the strip's center stats. This section gives it a home again. |
| input / output tokens, cache share | `sessionTotals.input_tokens`, `output_tokens`, and the cache split (`fresh_input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`) | Cache share = `cache_read / input`. It is what explains a cheap turn vs an expensive one (REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18 §2.2). Omitted for providers that report no split. |
| context fill | `paneModel.state.lastContextTokens` + the provider's context window | Same numbers as the strip's ctx text, spelled out with the percentage. |
| session size · last activity | `session:line_count`, `session:last_activity_ms`, `session:start_ts_ms` meta | The line count also drives the ≥500k warning, which stays a banner (§3). |
| archived | `session:archived_at` | When archived, the section shows "Archived <when>" and swaps Archive for **Restore**. |

Cost/turns are Claude-only in practice (`cost_usd` comes from `result.cost_usd`); a missing value renders as `—` rather than `$0`, so codex/gemini panes do not claim a false zero.

**Buttons.** `[Archive]` / `[Export]` / `[Restore]` call the same `SessionArchiveCommand` / `SessionExportCommand` / `SessionRestoreCommand` as the old drawer rows, with the same in-flight disable. Restore replaces Archive once the session is archived.

**Panel behavior.** Escape, an outside click, or focus leaving the panel closes it; clicking one of its own action buttons must not. Its listeners — and the floating `autoUpdate` — live only while it is open: tying `autoUpdate` to the component lifetime instead leaves scroll/resize observers repositioning a detached element for the rest of the pane's life (found in review; `AgentRuntimeDropup` has the same latent bug, not fixed here).

**Growth.** The panel floats upward from a trigger that sits near the pane bottom, so on a short pane it must scroll rather than clip — `assertMenuInPaintableArea` is the existing guard.

## 4. The info panel

Replaces `AgentControlBar` inside `.agent-composer-details`, above `ResizableDetailsDrawer`. New component: `AgentShellInfoPanel`.

### 4.1 Collapsed (default): one summary line

```
● pwsh  pid 18432 · 12m · C:\repo\app     ⚙ 2 running · 318 MB    ⌄
```

**Left: the drawer shell.**
- **Status dot:**
  - green = `shellprocstatus: "running"`;
  - grey = `init`;
  - red = `done` with a non-zero `shellprocexitcode`, plus the text `exited 1`.
- **Shell name:** image basename of the shell process.
- **PID.**
- **Uptime:** from `spawn_ts_ms`.
- **cwd:** the sub-block's `cmd:cwd`, middle-ellipsized, full path in the tooltip.
- **"agent is using this shell" lock:** the existing `term:agentlockuntil` badge moves here, out of the terminal overlay.

**Right: agent-started processes.**
- Count and total RSS of what the agent has running for this pane (§4.3).
- Hidden entirely when the count is 0, so an idle agent shows only the shell half.

**Chevron:** toggles §4.2. The state persists per block as `term:drawerinfoexpanded` meta, the same persistence path as `term:shellheight`.

**Height:** one row, matching the composer strip's 11px type. The resize handle stays at the drawer's top edge, above this line.

### 4.2 Expanded: process table

One table, grouped into two sections.

**Section 1: "Started by the agent".** Source: `AgentProcessListCommand({block_id: agentBlockId})`, filtered by #3430 to shell-launched processes.

| Name | PID | Mem | Age | Ports | |
|---|---|---|---|---|---|
| node.exe `vite` | 22140 | 212 MB | 8m | :5173 | ✕ |
| node.exe | 22188 | 96 MB | 8m | | ✕ |

- **Rows are trees, not flat lists.** Indent by `parent_pid` within the section, so `bash → node → node` reads as one launched thing. Shells that are only a pass-through (one child, no ports) collapse into their child's row.
- **✕ (stop):** `AgentKillProcessCommand({block_id, pid})` on the row's subtree, children first. Confirm only if the row has listening ports ("Stop vite on :5173?").
- **Section footer: "Stop all (N)"** uses a **new** RPC, `agent.kill-started` (§5.2). It must **not** call `AgentKillTreeCommand`: that terminates the whole job, including the agent CLI and its MCP servers.

**Section 2: "Started from this shell".** Source: `AgentProcessListCommand({block_id: shellSubBlockId})`.
- The drawer shell is tracked under its own sub-block id (`shell/lifecycle.rs` `track_spawned(&self.block_id, pid)`).
- Its root is a shell, so its children count under #3430's rule.
- Same row format. Hidden when empty.

**Empty state:** "Nothing running." The table never shows the agent CLI, conhost or MCP servers (#3430).

**Updates:** live via the existing `agent:process-added` / `agent:process-exited` events scoped `block:<id>`, for both ids. Generalize `hooks/useProcessCount.ts` into `useTrackedProcesses(blockId) → Accessor<AgentProcessInfo[]>`, with snapshot then deltas, same subscribe-before-fetch ordering. Mem/age values refresh from a snapshot re-fetch every 5s **only while expanded**; the collapsed line uses the event-maintained list and the last snapshot's RSS.

### 4.3 What "best stats" means: data per field

| Field | Source today | Gap |
|---|---|---|
| shell status / exit code / spawn time | `controllerstatus` event for the sub-block, already subscribed in `AgentShellSubblock.tsx` | Lift the subscription so the panel and the terminal share it |
| shell PID | `ShellController.inner.child_pid` | **Not exposed.** Add `shellprocpid` to `BlockControllerRuntimeStatus` |
| shell name | none | Derive from the PID's image via the sub-block's process list root, or add `shellprocname` alongside `shellprocpid` |
| shell CPU/mem | `useBlockStats(subBlockId)` (`blockstats` event, fed by pidregistry) | Optional in the collapsed line; show in the tooltip first |
| cwd | `cmd:cwd` meta (creation cwd) | Live cwd needs OSC 7 / prompt integration; out of scope, label it "started in" |
| process name, PID, RSS | `AgentProcessInfo` | none |
| process parent (tree indent) | `TrackedProcess.parent_pid` (#3430) | **Not on the RPC.** Add `parent_pid` to `AgentProcessInfo` |
| process age | `started_at_ms` | **Always 0 on Windows** ("deferred" in `windows.rs`). Fill via `GetProcessTimes` |
| process args (`vite`, `next dev`) | image path only | Needs the PEB command line (`NtQueryInformationProcess` class 60, `ProcessCommandLineInformation`). Phase 3; show the image name until then |
| listening ports | none | Phase 3: `GetExtendedTcpTable(TCP_TABLE_OWNER_PID_LISTENER)` filtered to member PIDs; one call per poll, shared like the #3430 parent snapshot |
| tracking confidence | `AgentProcessListResult.confidence` | none (§4.4) |

### 4.4 Platforms without a tracker

When `confidence === "none"`, the right half of the collapsed line reads `process tracking unavailable on this platform` in muted text, and the table is not offered. The shell half still works, since it comes from `controllerstatus` and does not depend on the tracker. `best_effort` (future macOS) adds a `≈` before the count, with a tooltip explaining that escaped descendants may be missing.

## 5. Backend changes

### 5.1 RPC shape

`AgentProcessInfo` gains the following fields. All are additive and optional on the TS side, so older frontends ignore them.

```rust
pub parent_pid: Option<u32>,
pub listening_ports: Vec<u16>,   // Phase 3; empty until then
pub args: Option<String>,        // Phase 3; None until then
```

`started_at_ms` becomes real on Windows via `GetProcessTimes`, converting FILETIME to Unix ms.

`BlockControllerRuntimeStatus` gains `shellprocpid: Option<u32>` (shell controllers only).

### 5.2 `agent.kill-started`

`AgentKillStartedCommand({block_id}) → {killed: u32}`. It terminates exactly the processes `list_block(block_id)` returns (the #3430 filter), deepest first, via `TrackerHandle::kill_pid`. It never touches roots, conhost or MCP servers. Unlike `kill_tree`, the job and the agent stay alive.

The per-row ✕ uses the same subtree logic, so it needs a `pid` parameter: `AgentKillStartedCommand({block_id, pid?: u32})` kills that PID's agent-started subtree, or all of them when `pid` is omitted. This supersedes the frontend's direct use of `AgentKillProcessCommand`, which only kills a single PID and would orphan grandchildren.

### 5.3 Cost

Per poll tick (2s):
- one Toolhelp snapshot, shared across blocks (#3430 P2);
- one `GetProcessTimes` per member (cheap; only on first sight of a PID, cached in the registry entry);
- Phase 3: one `GetExtendedTcpTable` call shared across blocks.

No per-process work for the collapsed line beyond what the poller already does.

## 6. Phases

**Phase 1: move out, move in** (frontend plus one backend field).
1. `AgentSessionNotices` in the composer region; `AgentSessionStats` — the context reading as a button, opening a popover with the stats and Archive/Export/Restore (§3.1); delete the drawer's Session/History rows and `AgentControlBar` (§3). Shipped in #3435.
2. `AgentShellInfoPanel` collapsed line: shell half (status, PID, uptime, cwd, agent lock) and agent-started count + RSS. Add `shellprocpid` to the runtime status.
3. `useTrackedProcesses` replaces `useProcessCount`.
4. Tests:
   - panel renders shell status from a `controllerstatus` fixture;
   - count half hidden at 0;
   - `confidence: "none"` degrade;
   - notices render outside the drawer (drawer closed, banner still visible);
   - Session section gated like today's rows (hidden at lineCount 0; Restore instead of Archive when archived);
   - the ctx reading is a button that opens the panel, and Escape closes it;
   - clicking Archive/Export does not close the panel;
   - missing `cost_usd` renders `—`, not `$0`.

**Phase 2: table + stop.**
1. `parent_pid` and real `started_at_ms` on the RPC.
2. Expanded table with both sections, tree indent, and pass-through shell collapse.
3. `agent.kill-started` (whole set and per-subtree); ✕ and "Stop all" wired to it.
4. Tests:
   - Rust: kill-started leaves roots and MCP alive (real Windows job, like #3430's test);
   - frontend: tree grouping, collapse rule, confirm-on-ports.

**Phase 3: richer identity.**
1. Listening ports (`GetExtendedTcpTable`), shown as `:5173` chips; clicking one opens `http://localhost:5173` in a web pane.
2. Command-line args via the PEB read, so rows say `vite` / `next dev` instead of `node.exe`.
3. Optional: shell CPU/mem in the collapsed line.

## 7. Open questions

- ~~**Q1:** Archive/Export: body context menu or pane header menu?~~ **Resolved (2026-09-19, user):** neither — the strip's context reading becomes a button that opens its own session popover, carrying the stats with it (§3.1).
- **Q2:** Should "Started from this shell" exist at all, or should the drawer only show the agent's processes? Processes the user runs are visible in the terminal itself. The value is in background jobs (`&`) that scrolled away.
- ~~**Q3:** Should closing the drawer offer to stop agent-started processes?~~ **Resolved (2026-09-19, user):** no. "closing drawer does nothing to processes it didnt start, its just a temporary shell drawer." Closing the drawer is a view toggle and never touches any process — not the ones the agent started, and not the ones started from the drawer shell, which keep running behind the collapsed drawer exactly as they do today. Only pane close (the #3422 confirmation) and the explicit stop controls in §4.2 end anything.
- **Q4:** The pane-close confirmation lists "N processes running" but not what they are. Once the table exists, should that dialog reuse the row format?
