# Status: Cross-Channel Opening an Old Agent Locks the Whole App for Minutes, Not Just That Pane

**Status: incident timeline reconstructed, several plausible mechanisms
ruled out, the actual cause of the multi-second-to-70-second gaps NOT
established. Not fixed. Revised after PR review caught the original
version's causal attribution (§4) overclaiming what the log evidence
actually shows — see the correction there.**
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
- **Not a frontend/renderer hang.** Isolated to just the `[fe]`-tagged log
  lines specifically (the ones actually emitted by frontend JavaScript —
  `[wrr]` lines are emitted by the native Rust `win_event_callback`,
  independently of the JS main thread, so an earlier version of this
  report combining both was measuring the wrong thing; see the §5
  correction). **2,529 `[fe]` lines** in the same `07:55:15–07:59:00`
  window, with no gap over 3.77s anywhere (that one gap sits right at
  pane-open, not inside the later multi-second stalls). A genuinely hung
  JS main thread would stop producing `[fe]` output too — it didn't. The
  renderer was continuously busy, not blocked. This conclusion held up
  under the corrected, `[fe]`-only measurement.

## 4. What the evidence does NOT establish — corrected after review

**The original version of this section attributed the gaps to two other
agents' `output.idx` rebuilds landing in the same window. That
attribution does not hold up and has been withdrawn — flagged correctly
in PR review (Codex, P1).** The actual cause of the 6–73 second gaps is
**not established** by this investigation.

`output.idx rebuilt` (`indexing.rs:88-90`) logs only *after* the full
scan-and-write completes — it carries no start time or duration, only a
completion timestamp. Checking each of the three "concurrent" rebuilds
against that constraint:

| Rebuild completes | Agent | vs. AgentY's incident |
|---|---|---|
| `07:55:19.104` | AgentX, ~850MB | **Finished *before* AgentY's pane was even opened** (07:55:20.284) — cannot have caused any of AgentY's gaps, all of which start later. |
| `07:55:26.695` | Smike, ~165MB | Completes 274ms after the first 73.2s gap begins (07:55:26.421) — consistent with having *finished* right as the gap started, not with running through it. |
| `07:56:41.164` | Smike, ~165MB | Completes inside the second 23.1s gap window — the only one of the three where overlap is even plausible, and only if the scan started within that same window (unknown). |
| `07:57:56.982` | Smike, ~165MB | Completes *after* the third gap's end (`TearOffBlock` at 07:57:56.311) — not inside it. |

None of the three cleanly falls inside the gap it was previously claimed
to explain. Without a start time or duration for any of these scans, "the
rebuilds were running during AgentY's gaps" is not something this log
data can show — it's a hypothesis this report incorrectly presented as a
finding. **What remains true:** these rebuilds are real, large, and ran on
the shared `srv` process at genuinely nearby times to the incident; the
architectural gap (`rebuild_output_idx` has no `tokio::task::spawn_blocking`
offload, §7.1) is real and independently worth fixing regardless of
whether it explains this specific incident. But this report cannot claim
it *does* explain this incident.

**What would actually establish or refute this:** start-time/duration
instrumentation on `rebuild_output_idx` (or Tokio runtime
worker-occupancy metrics) captured during a live reproduction, correlated
directly against the RPC that's stalled — not after-the-fact completion
timestamps alone.

## 5. A secondary, possibly-related signal: native window-event flood — corrected scope

The frontend host log shows an unusually high, sustained rate of native
Win32 `EVENT_OBJECT_LOCATIONCHANGE` events (`0x800b`) during the same
window — **7,149 `[wrr]` callback log lines** in ~225 seconds (≈32/sec
sustained; the callback's own doc comment,
`agentmux-cef/src/wrr/win_event.rs:749-753`, states normal user activity
is "~5-20/sec").

**Correction on what each callback actually does (PR review, Codex P2):**
the original wording here claimed each callback does `GetClassName`/
`GetWindowRect`/`IsIconic` plus launcher IPC — overstating the common
path. Reading the actual handler
(`agentmux-cef/src/wrr/win_event.rs:887-991`): every callback does the
cheap `read_class_name` + `read_window_rect` reads. `IsIconic` and the
heavier pool-close logic (state lookup, dispatch, quit-gate
re-evaluation, launcher IPC for a window *close*) only run on the rare
"window moved off-screen into the recycle pool" branch
(`rect.left < OFFSCREEN_POOL_THRESHOLD_X`, lines 913-967) — not on a
normal on-screen `LOCATIONCHANGE`. The common on-screen case does reach
one IPC call (`launcher_ipc::report_hwnd_position_changed`, line 972),
but it's **debounced to ~20Hz per window**
(`position_debounce::should_emit`, line 971), not fired once per logged
callback. So the true IPC volume driven by this flood is well under the
7,149 raw callback count — closer to `min(7149, ~20/sec × window-count ×
duration)` per distinct hwnd, not one-for-one with the log line count.
The 7,149 figure measures native callback *invocations* (each doing at
minimum two cheap Win32 reads), not 7,149 IPC round-trips.

`LOCATIONCHANGE` still fires on native window move/resize — a plausible
(not confirmed) trigger is layout instability in a pane rendering a very
large document, which is the subject of today's separate
scroll-height-oscillation investigation
(`docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_LIVE_INSTANCE_DATA_2026_08_22.md`,
issue #2718) on this same `0.55.18`/`0.55.19` host, same day. **Flagging
the correlation, not claiming the link, and not claiming this flood alone
is expensive enough to explain minutes of unresponsiveness** — no direct
evidence ties this specific incident's `LOCATIONCHANGE` volume to a
specific pane's reflow, or establishes that the (mostly cheap, debounced)
work it triggers is significant enough to matter here. Worth checking
together given the coincidence in timing and host, nothing stronger than
that.

## 6. Impact

**Established:** opening a persistent agent cross-channel for the first
time can freeze the whole app (not just that agent's pane) for minutes —
this incident is real and reproduced live, independent of what causes it.
**Not established:** whether the specific mechanism is "an unrelated
agent's `output.idx` rebuild blocking the shared runtime" (§4 — withdrawn
as a confirmed explanation, though the architectural gap it describes is
real on its own terms) or something else entirely. Treat the *symptom*
(multi-minute full-app freeze on cross-channel agent open) as confirmed
and worth fixing; treat the *specific mechanism* in §4 as an unconfirmed
lead, not the diagnosis.

## 7. Recommended next steps (not implemented here)

1. **Instrument first, before implementing a fix aimed at §4's
   hypothesis.** Add start-time (or duration) logging to
   `rebuild_output_idx`, and/or capture Tokio runtime worker-occupancy
   metrics, then reproduce live. Without that, a `spawn_blocking` fix
   would be shipped on faith that it addresses this incident rather than
   on evidence that it does.
2. **`rebuild_output_idx` having no `tokio::task::spawn_blocking` offload
   is a real, independently-worth-fixing gap** regardless of whether it
   explains this specific incident — a large synchronous scan on a shared
   async runtime is a latent risk either way. Worth fixing on its own
   merits once instrumented per #1 confirms (or doesn't) that it's what
   happened here.
3. **Investigate incremental/append-only indexing** for `output.idx` so a
   *necessary* rebuild after a small append doesn't have to re-scan the
   entire (potentially ~1GB) file from byte zero — flagged as a known,
   separate tradeoff in the prior retro, still unaddressed, independent of
   #1's open question.
4. **Correlate with the scroll-oscillation `[wrr]`/`LOCATIONCHANGE`
   signal (§5)** — a lead, not a confirmed contributor. If large-document
   layout instability is genuinely driving native window-event volume,
   fixing the render-side instability (already tracked in issue #2718)
   may reduce this incident class too, but this needs its own
   verification before treating it as part of the fix.
5. **Surface a "still loading…" signal to the user** instead of a silent
   full-app freeze, regardless of root cause — matching the same "don't
   fail silently" principle from the prior stale-session-id status doc.
   This is a good idea independent of whatever §1's instrumentation finds.

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
