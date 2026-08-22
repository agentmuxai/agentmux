# Status: Cross-Channel Opening an Old Agent Locks the Whole App for Minutes, Not Just That Pane

**Status: root cause traced with strong circumstantial evidence, not
proven end-to-end, not yet fixed.**
Live-reproduced today: opening the persistent agent "AgentY" from a
freshly-touched local build instance (`0.55.19`) — a genuine
cross-channel open, AgentY had never been opened in that instance before
— froze the entire AgentMux window ("totally locked") for roughly a
minute by the operator's own estimate; log evidence below shows the
underlying incident actually spans **3 minutes 36 seconds** end to end.

## 1. User-visible symptom

Operator opened "AgentY" (a long-lived, heavily-used persistent agent —
its transcript is ~1.5 million lines) in a `0.55.19` local `task package`
build that had never had this agent open before. The whole app became
unresponsive for what felt like about a minute. Ask: "we need old agents
to open instantly."

## 2. Timeline, reconstructed from `agentmuxsrv-v0.55.19.log.2026-08-22`

All times UTC, 2026-08-22.

| Time | Event |
|---|---|
| `07:55:20.284` | `ControllerResync` — new local block `34d8ae1b-...` created for AgentY (forcerestart=true, i.e. a fresh pane open) |
| `07:55:20.296`–`07:55:25.9xx` | Subagent-watcher cold-backfill scan: 257 subagent transcript files found, capped to 200, replayed (~5.6s just for this step) |
| `07:55:25.920` | First `blockfile:line_count` for AgentY's block |
| `07:55:26.004` | First `blockfile:read_range` (offset≈1,527,161) — **only 85ms after line_count: fast** |
| `07:55:26.421` | `ResolveCli` |
| **gap: 73.2s** | — |
| `07:56:39.608` | `blockfile:line_count` |
| **gap: 23.1s** | — |
| `07:57:02.740` | `blockfile:read_range` (offset≈1,526,961) |
| **gap: 53.6s** | — |
| `07:57:56.311` | **`TearOffBlock via saga`** — the pane gets dragged out to a new window |
| `07:57:56.414`–`58.055` | Pane re-resyncs in the new window (fresh `ControllerResync`, new tab) |
| `07:58:05.227` | `blockfile:read_range` |
| **gap: 21.4s** | — |
| `07:58:26.645` | `blockfile:line_count` |
| **gap: 6.0s** | — |
| `07:58:32.652` | `blockfile:read_range` |
| **gap: 24.0s** | — |
| `07:58:56.644` | `blockfile:line_count` (last observed event) |

Total span: **07:55:20.284 → 07:58:56.644 = 3m 36s**. The `TearOffBlock`
at 07:57:56 — 2m 36s into the incident — is very likely the operator
dragging the frozen pane into a new window in an attempt to unstick it,
not a normal user action; it's consistent with, not contradicting, "about
a minute" as the point at which the operator judged it hung and tried
something.

## 3. What's NOT the cause (checked and ruled out)

- **AgentY's own `output.idx` was never rebuilt.** Grepped the entire
  session log for `output.idx rebuilt` events matching AgentY's true
  definition id (`dedc33bf-b69c-4236-9b34-20bda3ef2738`, confirmed via
  direct query against `data/db/objects.db`'s `db_agent_definitions`
  table) — **zero matches**, anywhere in the log, not just this window.
  This means the `output.idx` freshness-check fix shipped in PR #2701
  (`agentmux-srv/src/server/app_api/mod.rs`'s `global_zone_line_count`)
  is working correctly here: AgentY's own cached index was fresh and
  reused, not needlessly rebuilt from scratch. **AgentY's own transcript
  size is not, by itself, the direct cause of these specific gaps.**
- **Not a frontend/renderer hang.** The host log (frontend process) logged
  **10,015 lines** in the same `07:55:15–07:59:00` window with only one
  gap over 2 seconds (3.77s, right at pane-open). A genuinely hung JS main
  thread would stop producing `[fe]`/`[wrr]` log output too — it didn't.
  The renderer was continuously busy, not blocked.

## 4. What the evidence points to instead

**Two other agents' `output.idx` rebuilds landed in the exact same
window, in the same `srv` process:**

| Time | Agent | Lines | Bytes covered |
|---|---|---|---|
| `07:55:19.104` | AgentX (`8e5f7b6d-...`) | 1,937,272 | 891,664,822 (~850 MB) |
| `07:55:26.695` | Smike (`9a8194e2-...`) | 398,816 | 173,397,162 (~165 MB) |
| `07:56:41.164` | Smike | 399,126 | 173,544,029 |
| `07:57:56.982` | Smike | 400,592 | 173,908,469 |

Each of these is a full synchronous re-scan of the named agent's entire
`output` file (`rebuild_output_idx`,
`agentmux-srv/src/backend/blockcontroller/shell/indexing.rs:28-98` — see
PR #2701, "fix(agent-pane): reuse fresh output.idx cache instead of
full-history rescan on every pane open," for the freshness-check
mechanism this section builds on). **Neither AgentX nor Smike is the
agent the operator opened** — they're unrelated panes that happened to
have genuinely-stale indexes (real content growth, not the bug PR #2701
fixed) at the same moment.

**`rebuild_output_idx` runs synchronously, inline, on whatever Tokio
worker thread picks up the RPC — it is never offloaded via
`tokio::task::spawn_blocking`.** This was already noted as a known,
separate tradeoff in the prior retro ("the cost still scales with the
total transcript size... a real, but separate and more defensible, design
tradeoff; the missing freshness check... is the clearly redundant part" —
i.e. PR #2701 fixed the *unnecessary*-rebuild case, not the *blocking*
nature of a *necessary* one). A large necessary rebuild for one agent can
occupy a worker thread for a long time; if the Tokio runtime's worker
pool is small enough relative to concurrent load, unrelated requests
(including AgentY's own, already-cache-hit-fast ones) queue behind it.
This is a plausible, evidence-consistent explanation for why AgentY's
requests — which individually didn't need to do any expensive work —
still show 6-73 second gaps between log lines in the same window three
much larger rebuilds were running.

**Not proven as the sole or complete mechanism.** This report does not
have direct proof (e.g. Tokio runtime metrics, or a thread-pool-occupancy
trace) that these specific rebuilds *caused* these specific gaps as
opposed to correlating with them. It's the most evidence-consistent
explanation found, not a confirmed causal chain — worth stating precisely
rather than overclaiming.

## 5. A secondary, possibly-related signal: native window-event flood

The frontend host log shows an unusually high, sustained rate of native
Win32 `EVENT_OBJECT_LOCATIONCHANGE` events (`0x800b`) during the same
window — **7,149 `[wrr]` callback log lines** in ~225 seconds (≈32/sec
sustained; the callback's own doc comment,
`agentmux-cef/src/wrr/win_event.rs:749-753`, states normal user activity
is "~5-20/sec"). Each callback does synchronous `GetClassName`/
`GetWindowRect`/`IsIconic` Win32 calls plus IPC to the launcher process
(`agentmux-cef/src/wrr/win_event.rs:887-920`). `LOCATIONCHANGE` fires on
native window move/resize — a plausible (not confirmed) trigger is
layout instability in a pane rendering a very large document, which is
exactly the subject of today's separate scroll-height-oscillation
investigation (`docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_LIVE_INSTANCE_DATA_2026_08_22.md`,
issue #2718) on this same `0.55.18`/`0.55.19` host, same day. **Flagging
the correlation, not claiming the link** — no direct evidence ties this
specific incident's `LOCATIONCHANGE` volume to a specific pane's reflow;
worth checking together given the coincidence in timing and host.

## 6. Impact

Any pane-open that triggers (a) a genuinely-necessary `output.idx`
rebuild for a large agent — including one *other* than the agent being
opened, sharing the same `srv` process — risks freezing the whole
instance for however long that scan takes (minutes, for transcripts in
the hundreds-of-MB to ~1GB range seen live today), not just the opened
agent's own pane. This is a materially worse failure mode than "one
agent's cold-open reopen is slow" — it's "the whole app can lock up
because of an unrelated agent's housekeeping."

## 7. Recommended fix directions (not implemented here)

1. **Offload `rebuild_output_idx` to `tokio::task::spawn_blocking`** (or
   an equivalent blocking-thread-pool mechanism) so a large scan for one
   agent's `output.idx` cannot starve the async runtime's worker threads
   and delay unrelated requests. This is the most directly actionable
   fix given §4's evidence, and is architecturally distinct from PR
   #2701's freshness-check fix (which only avoids *unnecessary* rebuilds
   — a genuinely necessary one, for any agent, still blocks today).
2. **Investigate incremental/append-only indexing** for `output.idx` so a
   *necessary* rebuild after a small append doesn't have to re-scan the
   entire (potentially ~1GB) file from byte zero — flagged as a known,
   separate tradeoff in the prior retro, still unaddressed.
3. **Correlate with the scroll-oscillation `[wrr]`/`LOCATIONCHANGE`
   signal (§5)** — if large-document layout instability is genuinely
   driving native window-event volume, fixing the render-side instability
   (already tracked in issue #2718) may reduce this incident class too.
4. **Surface a "still loading — an unrelated agent's data is being
   indexed" signal to the user** instead of a silent full-app freeze, at
   minimum as a stopgap while #1 is implemented — matching the same
   "don't fail silently" principle from the prior stale-session-id status
   doc.

## 8. Reproduction data

- Affected agent (opened): AgentY, definition id
  `dedc33bf-b69c-4236-9b34-20bda3ef2738`, new local block
  `34d8ae1b-a2cb-443a-a2b9-fc6121593530`
- Instance: `0.55.19`, local build channel `local-main-b28b7a-b966d418`
- Concurrently-rebuilding agents: AgentX (`8e5f7b6d-1d3d-422c-b43b-f21cc049653d`,
  ~850MB/1.94M-line rebuild), Smike (`9a8194e2-337e-4f32-a272-53b8b1be94c3`,
  ~165-166MB/~400K-line rebuild, ×3 in-window)
- Log sources:
  `C:\Users\asafe\.agentmux\logs\agentmuxsrv-v0.55.19.log.2026-08-22`
  (srv, top-level shared path) and
  `C:\Users\asafe\.agentmux\channels\local-main-b28b7a-b966d418\versions\0.55.19\logs\agentmux-host-v0.55.19.log.2026-08-22`
  (host/frontend)
- Window analyzed: `07:55:15Z`–`07:59:00Z`

## 9. Sources

- `agentmux-srv/src/backend/blockcontroller/shell/indexing.rs:28-98`
  (`rebuild_output_idx` — synchronous, no `spawn_blocking`)
- `agentmux-srv/src/server/app_api/mod.rs` (`global_zone_line_count`,
  fixed for the unnecessary-rebuild case by PR #2701)
- `agentmux-cef/src/wrr/win_event.rs:719-920` (`win_event_callback`,
  `EVENT_OBJECT_LOCATIONCHANGE` handling)
- `docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_LIVE_INSTANCE_DATA_2026_08_22.md`
  (§5's possibly-related scroll-instability signal, same host/day)
- `data/db/objects.db`'s `db_agent_definitions` table (used to resolve
  definition ids to agent names for this report)
