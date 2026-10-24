# Agent Self-Compaction: Context, Compact, Fresh Session, and a Lower Auto-Compact Backstop

**Date:** 2026-10-23
**Status:** active — approved by the operator on 2026-10-23. PR 1 (§3.1, the backstop setting) shipped in #5393; PR 2 (§3.2–§3.6: context, compact, the compact hint) in #5399; PR 3 (fresh, and the fresh hint) remains.
**Owner:** agent-pane lifecycle
**Research:** [`REPORT_AGENT_SELF_COMPACTION_AND_TOKEN_USE_2026_10_23.md`](../reports/REPORT_AGENT_SELF_COMPACTION_AND_TOKEN_USE_2026_10_23.md) (measurements, economics, options, probe)
**Related:** [`SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`](SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md) (detection and display), [`SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md`](SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md) (running summary and continuation packet), [`SPEC_RESUME_PHASE_CATCH_UP_TURN_2026_10_13.md`](SPEC_RESUME_PHASE_CATCH_UP_TURN_2026_10_13.md) (catch-up turn), [`REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md`](../reports/REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md) (context readings, `get_context_usage`)

---

## 1. Why

Agents run with very large contexts and compact late, mostly in the middle of their work. Measured over 30 days of agents on one development machine (report §1–§2):

- **Cost follows context size.** Cache reads are 63% of the API-equivalent cost; the median call carries 422K tokens; calls above 700K are 20% of calls and 35% of the cost.
- **Compaction is rare, late and mid-task.** Claude Code auto-compacts at the window minus 33K, about 967K on 1M-window models; 85% of auto compactions landed inside a tool loop.
- **One compaction is cheap** (about $0.76 on Opus 5.5 at list prices, ~65 s) and pays for itself after about 4 calls.
- **Compacting earlier saves about a third:** replaying every session with compaction at 400K cut main-session cost by 32%; at 600K, 23%.
- **Agents can't act on any of this today.** Compaction is triggered only by the literal user message `/compact`; an agent can't trigger it or read its own context fill; a jekt containing `/compact` can't trigger it (jekts are wrapped).

The operator approved the direction on 2026-10-23: a lower auto-compact backstop now, then agent-called actions.

## 2. Goals and non-goals

**Goals**
1. A **backstop**: Claude agents auto-compact at a lower, operator-set point (default 600K) instead of ~967K. An AgentMux setting; srv seeds it into every Claude config dir it prepares.
2. An agent can **read its own context**: tokens, window, auto-compact point.
3. An agent can **compact itself** at the end of its turn, with a focus note on what to keep.
4. An agent can **start a fresh session** at a task boundary, carrying a handoff note, without losing its pane, memory, workspace or task.
5. srv **hints** at the end of a turn when compacting or starting fresh would pay, so the agent spends almost no tokens deciding.
6. Every action stays under the agent's explicit control and only ever runs between turns.

**Non-goals**
- Compacting or restarting **another** agent. If it is ever added, it takes the user's 15-second override window and an audit entry, like stopping another agent's pane.
- Changing how Claude Code or Codex summarize. AgentMux supplies the focus note and the handoff; the CLIs do the summarizing.
- The Anthropic API's server-side compaction and context editing: AgentMux runs the CLIs, which manage their own context.
- History storage and retention (transcripts on disk, archive, disk reclaim).

## 3. Design

### 3.1 The backstop: `agent:compactat`

**Setting.** `agent:compactat`, a number of tokens, default **600000**. `0` means "leave Claude Code's own default".

**Why 600K and not the 400K the report recommends.** The two numbers do different jobs:

| Compaction point | Who triggers it | Where it lands | Saving in the report's replay |
|---|---|---|---:|
| 600K backstop (this setting) | Claude Code, automatically | Wherever the context crosses it: 85% of auto compactions landed mid-task | ~23% |
| 400K, when the hint fires (§3.5) | The agent, with `Agents compact` | At the end of its turn, with a focus note | ~32% |

The backstop catches agents that don't compact themselves. The 32% comes from agents compacting at ~400K at a boundary. A 400K backstop would capture the same tokens but mid-task, roughly twice as often as at 600K (84 vs 40 extra compactions in the replay), with the re-reads and lost steps the report measured. It would also fire before the 400K hint could get the agent to compact cleanly. Keep the backstop above the hint, so the hint gets the first chance. Until PR 2 ships, the backstop alone is in effect (~23%). Shown in Settings ▸ Advanced as "Compact agents at", with the hint "Agents compact their context when it reaches this size. Lower saves tokens; higher keeps more detail. Takes effect when each agent next starts."

**Seeding.** srv writes Claude Code's `autoCompactWindow` = `agent:compactat` + 33,000 (Claude Code compacts at the window minus 33K), clamped to Claude Code's accepted range of 100,000–1,000,000, into the config dir's `settings.json`. A new `seed_auto_compact_window(provider, config_dir, compact_at)` in `crates/srv/src/backend/providers.rs`, called next to `seed_transcript_retention_if_missing` in both places that already call it:

- `prepare_provider_auth_dir` (`providers.rs`), for the default and per-agent dirs;
- the identity resolver's spawn path (`crates/srv/src/identity/resolver/inject.rs`), for per-account dirs.

It follows the existing seeding rules: Claude only; never rewrites a `settings.json` it can't parse as a JSON object; writes through a temp file and a rename; soft-fails with a warning and never blocks a spawn.

**Whose value it is.** Unlike `cleanupPeriodDays`, this one must follow the setting when the operator changes it, without overwriting a value a person chose. srv records what it last wrote in a sidecar, `<config_dir>/.agentmux-seeded.json` (`{"autoCompactWindow": 633000}`), and writes `autoCompactWindow` when:

- it is absent; or
- it equals the value in the sidecar (AgentMux's own, so the setting can move it); or
- it is 633,000 and there is no sidecar (the value applied by hand on every host on 2026-10-23, which is AgentMux's own).

Any other value is a person's choice and is left alone. With `agent:compactat` = 0, srv removes `autoCompactWindow` only if it is AgentMux's own, then clears the sidecar.

**When it applies.** Claude Code reads `settings.json` at start, so a change applies at each agent's next CLI start. The pane's context meter already uses the threshold the CLI reports through `get_context_usage`, which reflects `autoCompactWindow`, so its countdown follows.

**Codex** is left at its own default for now: its auto-compaction fires at 90% of a ~258K window, well under any useful backstop.

**Alternative considered:** passing `CLAUDE_CODE_AUTO_COMPACT_WINDOW` at spawn. It needs no file write and applies at once, but it overrides a person's own `/autocompact` or `autoCompactWindow` (the variable wins), and it doesn't reach a `claude` run outside AgentMux with the same config dir. Seeding the file respects both.

### 3.2 srv keeps the context reading

Today the Claude reading is computed in the frontend and mirrored into block meta `agent:context` only while the pane is mounted; srv holds the CLI's threshold but not the current fill. For Codex, srv computes the reading after each turn but only writes it into the stream.

srv becomes the source of truth for both:

- **Claude:** `persistent/stdout_reader.rs` already sees every `message_start` and `assistant` frame. It keeps, in `PersistentInner`, the last main-agent call's context (input + cache read + cache creation), its model, and the window (`result.modelUsage[*].contextWindow`, else `context_usage.window`), with the same filtering the frontend uses (`context-reading.ts`: main agent only, discard impossible readings).
- **Codex:** keep the last `CodexContext` from `subprocess/codex_context.rs` on the controller.
- Both also keep: the time and number of calls since the last compaction or fresh start, and whether a compaction is running.

The frontend keeps computing its own reading for the meter; srv's copy is for MuxAPI and the hint.

### 3.3 MuxAPI: `Agents` actions `context`, `compact`, `fresh`

All three act on the **caller's own pane** only, resolved from its token the way `server/app_api/mcp_health.rs` does (`own_pane_by_token`). No `target` parameter.

**`context`**: read-only.

```json
{ "provider": "claude", "model": "claude-opus-5-5", "tokens": 512304, "window": 1000000,
  "auto_compact_at": 600000, "compacting": false,
  "calls_since_compaction": 212, "last_compaction": "2026-10-23T15:02:11Z",
  "hint": "compact" }
```

`hint` is the §3.5 rule's verdict: `none`, `compact` or `fresh`. Providers without a reading return `tokens: null` and say why.

**`compact`**: `{ "keep": "PR #5401, the failing test in reducer.test.ts, the three files I'm editing; the CI logs don't matter" }` (optional, ≤2,000 characters; `keep`, because the `Agents` tool's `focus` is already `open`'s boolean).

- Returns at once: `scheduled` (runs when the current turn ends), or `refused` with a reason (§3.6).
- **Claude:** at the turn boundary, srv writes `/compact <focus>` straight to the process's stdin as a stream-json user message (`persistent/input.rs`, the locked stdin writer), as a `TurnOrigin::System` turn. It does **not** go through message delivery (`deliver_agent_message_from`), which would wrap it or append held `[AgentMux]` notes; the probe showed appended text becomes part of the compaction instructions (report §3.3).
- **Codex:** at the boundary, srv runs the existing `compact_codex_thread` (`subprocess/codex_compact.rs`, `thread/compact/start`). Codex takes no per-call focus, so a focus is ignored and the result says so.
- **Other providers:** `refused: unsupported`.
- Afterwards the existing machinery runs unchanged: the `compact_boundary` frame, the compaction bar, memory re-delivery through the `SessionStart` hook with source `compact`, and the catch-up turn. The catch-up turn needs no change: Claude Code's summary records the focus note itself (report §3.3), so the agent resumes on what it chose to keep.

**`fresh`**: `{ "handoff": "Task #1791 (spec written, uncommitted at docs/specs/...). Open PRs: #5401 (in review). Decided: 600K default. Next: wait for AgentY's go, then PR 1." }` (required, ≤4,000 characters).

- Returns `scheduled` or `refused`.
- **Claude:** at the turn boundary, srv stops the CLI gracefully (the `stop_for_config_restart` path), clears the session id (`inner.session_id` and the `agent:sessionid` meta), and respawns **without** `--resume`. The existing continuation path (`persistent/spawn.rs`, `resume_retry.rs` `carry_continuation`) builds the packet and prefixes it to the first message. The agent's handoff goes into the packet as its own section, "Handoff from your previous session", ahead of the running summary (`continuity.rs` `build_continuation_packet`). The packet's size cap still applies; the handoff is kept whole and the recent exchange is cut first.
- **Codex:** at the boundary, srv drops the thread id, so the next `codex exec` starts a new thread, and sends the packet, with the handoff, as that turn's first message.
- The pane's existing `fresh_session` catch-up turn follows (`useAgentStream.ts`).
- This is **not** the pane's "New session" button, which archives the transcript and the running summary. The transcript continues in the same pane, so the user sees one conversation with a "fresh session" marker.

**What an agent keeps across a fresh session:**

| Kept | How |
|---|---|
| Its pane, name, identity, account, model and settings | Same block; only the CLI session changes |
| Global and Personal Memory | Startup instructions file and the `SessionStart` hook, as at every start |
| Workspace: working directory, files, git state, branches | Untouched on disk |
| Kanban tasks it claimed, Presence claims, schedules | Server-side state, not part of the conversation |
| Its handoff note | First section of the continuation packet |
| Running summary, up to 40 identifiers (PR numbers, paths, ids), the last request and the recent exchange | The continuation packet (≤ ~24K characters) |
| The transcript the user sees | Same pane, continued |

| Not kept | Why, and what to do |
|---|---|
| The conversation itself: files read, tool output, reasoning | That's the point; anything needed goes in the handoff |
| Claude Code background shells, monitors and `Task` subagents | They die with the CLI process. The tool description tells the agent not to call `fresh` while background work runs |
| Claude Code's own todo list | Per-session in the CLI; put open items in the handoff |

### 3.4 Tool descriptions (what the agent reads)

- **`context`**: "How full your context is and when it will auto-compact. Free; check it at the end of a piece of work, not every turn."
- **`compact`**: "Compact your own context when your current turn ends. Use it when the same task continues and your context is large (the hint says so). Say in `keep` what to keep: PR and issue numbers, file paths you're editing, decisions, the next step. A focus steers the summary; it doesn't delete anything. Never needed mid-task; it waits for the end of your turn."
- **`fresh`**: "Start a fresh session when your current turn ends, keeping your pane, memory, workspace and tasks. Use it when the next work is unrelated to what's in your context. Write a `handoff`: task number, spec or file paths, open PRs, decisions, next step. Don't call it while background shells, monitors or subagents are running; they stop with the session."

### 3.5 The turn-end hint

At the end of every turn, srv evaluates (no model call):

1. **C < 250K** (C = context tokens): nothing.
2. **A task boundary just happened and C ≥ 150K**: hint `fresh`. A boundary srv can see is a `Tasks complete`, `submit` or `claim` by this agent during the turn, or a PR it opened being merged.
3. **C ≥ 400K**: hint `compact`.
4. At most one hint per crossing (it repeats only after the context has dropped below 250K and crossed again), never within 50 calls of the last compaction or fresh start, and never right after a catch-up turn.

The hint is one `[AgentMux]` note appended after the turn (the held-notes path, `reactive/held_notes.rs`), for example:

`[AgentMux] Your context is 512K of 1000K tokens. If your next work continues this task, compact now: Agents compact, with a focus saying what to keep (PR and task numbers, files you're editing, decisions, the next step); it runs when this turn ends.`

The note gives tokens, not dollars: prices change and differ by account, and the agent only needs to know the context is large. PR 3 adds the fresh-session suggestion (rule 2) with `Agents fresh`. The thresholds are settings: `agent:compacthintat` (default 400000), `agent:freshhintat` (default 150000, with PR 3), and `agent:compacthints` (default on). The hint's default sits below the backstop (400K < 600K), so an agent that follows hints compacts at a boundary before the backstop fires mid-task.

The agent decides with one judgment it already holds at the end of its turn: is the next work related? Everything else is srv arithmetic. A hint costs ~40 input tokens; acting on it costs the focus or handoff note, ~100–300 output tokens.

### 3.6 Safety and limits

- **Between turns only.** `compact` and `fresh` are always deferred to the end of the current turn (`schedule_after_turn`-style polling of `turn_active`, as in `sagas/self_quit.rs`, with a grace period). A request made outside a turn runs at once.
- **One at a time.** A request while one is scheduled or running is refused (`already scheduled`); `fresh` cancels a scheduled `compact`.
- **Rate limit.** `compact` and `fresh` are refused within 50 calls or 10 minutes of the previous one (`too soon`), which also stops a compact → catch-up → compact loop.
- **Containers.** `compact` for container Codex panes and `fresh` for container panes are refused (`unsupported in containers`), matching today's Compact button.
- **Audit.** Each scheduled and completed action is logged with the pane, provider, tokens before and after, and the focus or handoff length (not its text).
- **Jekts stay inert.** Nothing here changes jekt wrapping: no other agent can compact or restart you through a message.

## 4. Files touched

| Area | Files |
|---|---|
| Setting | `crates/srv/src/backend/wconfig/types.rs` (`SettingsType`: `agent:compactat`, `agent:compacthintat`, `agent:freshhintat`, `agent:compacthints`), generated `frontend/types/srv-types.d.ts`, `frontend/app/view/settings/sections/advanced-section.tsx` |
| Seeding | `crates/srv/src/backend/providers.rs` (`seed_auto_compact_window`, called from `prepare_provider_auth_dir`), `crates/srv/src/identity/resolver/inject.rs`, `providers/tests.rs` |
| Context in srv | `blockcontroller/persistent/mod.rs` (`PersistentInner`), `persistent/stdout_reader.rs`, `subprocess/codex_context.rs` and the Codex host spawn |
| Compact | `persistent/input.rs` (direct stdin write at the boundary), `persistent/stdout_reader.rs` (boundary hook), `subprocess/codex_compact.rs` |
| Fresh | `persistent/lifecycle.rs`, `persistent/spawn.rs`, `persistent/resume_retry.rs`, `backend/continuity.rs` (handoff section), Codex host spawn (thread id) |
| MuxAPI | `crates/mcp/src/tool_schemas.rs`, the `Agents` family handler in `crates/mcp/src/tools/`, `crates/srv/src/server/routes.rs` (`/api/v1/agent/self/context`, `/compact`, `/fresh`), a handler next to `server/app_api/mcp_health.rs` |
| Hint | `reactive/held_notes.rs` (or the turn-end path in `stdout_reader.rs`), the model price table |
| Frontend | the catch-up turn for a self-requested compaction (`useAgentStream.ts`), the "fresh session" marker in the transcript |
| Docs | MuxAPI reference and the Operator Config MuxAPI note (new actions), this spec, the report |

## 5. Delivery

1. **PR 1, backstop:** the `agent:compactat` setting, `seed_auto_compact_window`, Settings UI, tests; with this spec and the public report. (The hand-applied 633,000 on 2026-10-23 becomes AgentMux's own value under §3.1.)
2. **PR 2, context and compact:** srv's context reading, `Agents context` and `compact`, the hint, its settings, the catch-up turn with the focus.
3. **PR 3, fresh:** `Agents fresh`, the handoff section, the Codex path, the transcript marker.

Each PR is independently useful; PR 2 doesn't need PR 3.

## 6. Testing

- **Seeding** (unit, `providers/tests.rs`): absent → written; AgentMux's own value (sidecar or 633,000) → follows the setting; a person's value → left; unparsable or non-object file → untouched; `compactat` 0 → own value removed; the clamp; never blocks a spawn.
- **Context reading** (unit): Claude frames → tokens, window and model as the frontend computes them (shared fixtures with `context-reading` tests); subagent frames ignored; Codex reading kept.
- **Compact** (integration, fake CLI on stdin/stdout): a request mid-turn writes nothing until the turn ends, then exactly one `/compact <focus>` line, no held notes; rate limit; already-scheduled; Codex path calls `thread/compact/start`.
- **Fresh** (integration): graceful stop, no `--resume` on respawn, the packet contains the handoff first, task claims and memory delivery unchanged, transcript not archived.
- **Hint** (unit): the rule's thresholds, one hint per crossing, suppression after compaction and catch-up.
- **Live check** before PR 2 merges: re-run the report's probe (§3.3) through the new action on the pinned CLI.

## 7. Measuring it (A/B)

**Baseline:** the report's 30 days (2026-09-23 to 2026-10-23).

**Arms, two weeks after PR 2 ships:**

- **Backstop only:** every agent from 2026-10-23, at 600K.
- **Backstop and self-compaction:** two of the four busiest agents, with hints on. The other two keep hints off (`agent:compacthints: false`).

**Measures**, using the report's scripts:

- tokens and API-equivalent cost per agent-day, split into cache read, cache write and output;
- median and p90 context per call;
- compactions per agent-day, split auto mid-task, auto at a boundary, and self-requested; fresh sessions per day;
- re-reads after a compaction (files touched in the 50 calls before, opened again in the 20 after);
- people repeating themselves after a compaction (the repeat-phrase scan);
- weekly usage-limit hits;
- PRs merged per agent-day and review rounds per PR (the quality guard).

**Success:** at least 20% lower cost per agent-day in the self-compaction arm than the baseline, with no rise in review rounds per PR or in re-reads per compaction.

## 8. Open questions

1. ~~**Overlap with the history-store work (disk reclaim after history compression).**~~ **Answered (2026-10-23):** that work only reclaims space inside the shared transcript store's SQLite file (VACUUM or incremental vacuum). It changes no session ids, continuity state, transcript layout or archiving, so a fresh session in an existing pane needs nothing from it.
2. **Defaults.** 600K backstop and 400K hint come from the replay; the A/B may move them. Fable 5.1's cache reads cost half of Opus 5.5's relative to input, which favours larger contexts there; per-model defaults can follow if the data says so.
3. **Codex backstop.** Codex compacts at 90% of its window already; a lower `model_auto_compact_token_limit` per agent can follow if Codex agents start running long sessions.
