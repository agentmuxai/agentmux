# Agent Self-Compaction and Token Use: Measurements, Economics and a Decision Rule

**Date:** 2026-10-23
**Status:** research report behind [`SPEC_AGENT_SELF_COMPACTION_2026_10_23.md`](../specs/SPEC_AGENT_SELF_COMPACTION_2026_10_23.md). This public version gives shares, token counts and ratios; spend totals and per-agent figures are kept internally.
**Question:** Can agents be compacted, or compact themselves, to save tokens? Can they judge their own context and the next piece of work, and decide when to compress?
**Builds on:** [`REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md`](REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md) (proved `/compact` works over stream-json), [`REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md`](REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md) (context readings and the CLI's auto-compact point), [`SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`](../specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md), [`SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md`](../specs/SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md) (the continuation packet).

---

## 0. TL;DR

- **The cost is the size of the context, not compaction.** Over 30 days of agents working on one development machine, priced at API list prices, **63% of the cost was cache reads**, 27% cache writes and 11% output. The median main-session call carried **422K tokens** of context. Calls above 700K were 20% of calls and 35% of the cost.
- **Compaction is rare, late and mostly mid-task.** 25 natural compactions in 30 days (about one per 850 calls). Claude Code auto-compacts at window minus 33K, which is **~967K on these 1M-window models**, and **17 of 20 auto compactions (85%) landed in the middle of a tool loop**. Only 16 of 51 sessions ever compacted.
- **One compaction is cheap.** About **$0.76** on Opus 5.5 (a ~$0.36 summary pass plus a ~$0.40 cache rewrite) and about 65 seconds. It pays for itself after **about 4 calls**, because every later call reads ~900K fewer tokens.
- **Compacting earlier saves real money.** Replaying the measured sessions with an earlier compaction point: at **400K** they would have cost **32% less** for 84 extra compactions; at 600K, 23% less. Below 300K the saving levels off (37% at 300K, 42% at 200K) while compactions multiply.
- **What gets lost is real but small.** In 18 of 25 compactions the agent re-read files it had just touched (~60K tokens in all), and a handful of times it forgot something that cost minutes (a helper script, a folder, a check it had just run). No person had to repeat themselves because of a compaction.
- **What exists today:** compaction is triggered only by the literal user message `/compact` (typed, or the pane's Compact button). Agents can't trigger it, can't read their own context fill, and a jekt containing `/compact` can't trigger it because jekts arrive wrapped. A "fresh session with a handoff note" already exists inside srv (the continuation packet) but isn't exposed as an action.
- **Recommendation:** (1) lower the CLI's auto-compact point per agent as a backstop (configuration, no code); (2) add a MuxAPI action that lets an agent read its context and **compact itself at the end of a turn with a focus note**, with srv offering a one-line hint when compaction would pay; (3) add **a fresh session with a handoff** for task boundaries. The decision is made by srv's arithmetic plus the agent's own knowledge of whether the next work is related, so it costs almost no tokens. §6 has costs, risks and the files touched.

---

## 1. Measurements

Source: every Claude Code transcript and Codex session file of the agents working on one development machine, 2026-09-23 to 2026-10-23 (UTC days), read-only. Each API response is counted once (one response is written as several lines repeating the same usage; without this the totals are ~2.3× too high). Costs are **API-equivalent at list prices** (§2.1) and are given as shares; these agents run on subscriptions, where the same tokens count against usage limits instead.

### 1.1 Where the cost goes

| | Share of API-equivalent cost |
|---|---:|
| Uncached input | 0% |
| Cache read | 63% |
| Cache write | 27% |
| Output | 11% |

By model: Opus 5.5 78%, Sonnet 5 9%, Sonnet 5.5 7%, Opus 5 4%, Fable 5.1 2%, Haiku 4.5 2% (AgentMux's own naming and summary calls). The busiest agents made 300–500 calls a day; per call, nearly all of the cost is reading the cached context.

Main sessions write only to the 1-hour cache; subagents only to the 5-minute cache. Uncached input is negligible: nearly every input token is a cache read or a cache write.

### 1.2 Context size per call

Context = input + cache read + cache write of each call, main agent sessions, n = 23,773 calls.

| p10 | p25 | median | p75 | p90 | p99 | max |
|---:|---:|---:|---:|---:|---:|---:|
| 129K | 231K | 422K | 651K | 818K | 951K | 967K |

| Context at the call | Share of calls | Share of cost |
|---|---:|---:|
| under 200K | 20% | 8% |
| 200K–400K | 27% | 18% |
| 400K–700K | 33% | 39% |
| 700K–1M | 20% | 35% |

Session max context: 10 sessions stayed under 100K, 9 reached 100–300K, 14 reached 300–700K and 18 reached 700K–1M. Eleven sessions ended above 500K without ever compacting.

### 1.3 How often auto-compaction fires, and where

| | Count |
|---|---:|
| Compactions in 30 days | 28 (20 auto, 8 manual); 25 natural, 3 in tests |
| Per active agent-day | 0.33 |
| Main-session calls per compaction | ~850 |
| Sessions that ever compacted | 16 of 51 |

Auto compaction fired at a context of **966,796–969,609 tokens** (median 967,204), which is Claude Code's documented point for a 1M window: window − min(max output, 20K) − 13K = window − 33K ([REPORT_AGENT_PANE_CONTEXT_METER §9](REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md)). Manual compactions were run between 438K and 934K (median 899K), mostly by agents told to.

**Mid-task or at a boundary.** A compaction is *mid-task* when the last response before it asked for a tool and the work continued afterwards without a new prompt, and *at a boundary* when it came right after a turn had ended or a new prompt arrived.

| Trigger | Mid-task | At a boundary | Unknown |
|---|---:|---:|---:|
| Auto | 17 (85%) | 2 (10%) | 1 |
| Manual | 0 | 8 (100%) | 0 |

Both boundary auto compactions were set off by an automated review notification arriving at a full context.

### 1.4 What each compaction cost, and how long it took

| | Median |
|---|---:|
| Context before | 965,520 |
| Context on the first call after | 68,741 (a 14× drop) |
| Summary written (`postTokens`) | 22,800 |
| Cache written by the first call after | ~50K (the ~17.8K system and tool prefix stays cached) |
| Duration (`durationMs`) | 65 s auto, 66 s manual (max 164 s, after a model switch) |

Claude Code doesn't record the summary call's usage in the transcript. Two compactions that sit between cumulative cost snapshots show it reads the whole context from cache plus ~2.1K new input, and writes 6.7–8.6K output. At Opus 5.5 prices:

| | Cost |
|---|---:|
| Summary pass (965K cache read + 2.1K input + ~8K output) | $0.36 |
| Cache rewrite on the first call after (~50K at the 1-hour rate) | $0.40 |
| **One compaction** | **$0.76** |
| Saved on every later call (~897K fewer tokens read) | $0.18 |
| **Break-even** | **~4 calls** |

Codex records the usage of its own compaction call; the one real Codex compaction measured read 12.6K tokens (12.4K cached) and wrote 36 tokens of output (its summary is an encrypted item, §3.2).

### 1.5 What was lost

The objective signal: in **18 of 25** compactions the agent opened again, within its next 20 calls, files it had touched in the 50 calls before: 49 files, ~60K tokens in all. Claude Code itself re-attaches about 5 recently used files after every compaction.

Cases, described rather than quoted:

1. **A helper forgotten (2026-09-24).** After an auto compaction mid-task, the agent no longer knew the GitHub wrapper script it had used 154 times that session. Plain `gh` failed, then a push failed; it found the helper again through its memory notes and an old transcript. About 9 calls and 7 minutes.
2. **A refactor's files re-read (2026-10-10).** Compacted halfway through a multi-file refactor; re-checked the working tree and re-read two files it had just read (~11K tokens, ~5 extra calls). The refactor finished correctly.
3. **A fix re-read (2026-10-07).** Re-read the file it had fixed two minutes earlier, plus its test and benchmark.
4. **A folder lost (2026-09-24).** The summary kept a document's name but not its folder; the agent looked at the repository root, got "path does not exist", and had to search.
5. **A check repeated (2026-09-24).** Ran again a PR review check it had just finished.
6. **CI gates re-read (2026-10-13).** Re-read seven CI gate scripts it had been editing.

People didn't have to repeat themselves: of 37 human prompts in the 3 hours after a compaction, one repeated something, and it referred to an earlier session.

### 1.6 Sessions and restarts

A main session's median is 265 calls over 15.7 hours (4.6 active). **29 of 51 sessions started as an AgentMux continuation** (the previous session couldn't be resumed, for example after an account switch), which is the fresh-session-with-handoff path of §3.4 happening by accident. Only 6 Codex sessions ran in the window, mostly probes; one real session peaked at 94K of a 258.4K window, too little for frequency figures.

---

## 2. The economics

### 2.1 Prices

Per million tokens, Anthropic first-party list prices, as read on 2026-10-23 ([pricing](https://platform.claude.com/docs/en/about-claude/pricing), [prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)):

| Model | Input | Output | Cache read | 5-minute write | 1-hour write | Window |
|---|---:|---:|---:|---:|---:|---:|
| Fable 5.1 | $10 | $50 | $0.25 (0.025×) | $12.50 | $20 | 1M |
| Opus 5.5 | $4 | $20 | $0.20 (0.05×) | $5 | $8 | 1M |
| Opus 5 | $5 | $25 | $0.50 (0.1×) | $6.25 | $10 | 1M |
| Sonnet 5.5 | $2 | $10 | $0.10 (0.05×) | $2.50 | $4 | 1M |
| Sonnet 5 | $2 | $10 | $0.20 (0.1×) | $2.50 | $4 | 1M |
| Haiku 4.5 | $1 | $5 | $0.10 (0.1×) | $1.25 | $2 | 200K |

A cache write costs 1.25× input for the 5-minute lifetime and 2× for the 1-hour lifetime; a read refreshes the lifetime at no extra cost. The cache is a prefix match: tools, then system, then messages, and changing anything invalidates everything after it. There is no long-context surcharge on these models: a 900K-token request is billed at the same rate per token as a 9K one. Claude Code on a subscription caches the main conversation for 1 hour, and subagents and the compaction call for 5 minutes ([Claude Code prompt caching](https://code.claude.com/docs/en/prompt-caching)), which matches what the transcripts show.

OpenAI, for Codex ([pricing](https://developers.openai.com/api/docs/pricing), [prompt caching](https://developers.openai.com/api/docs/guides/prompt-caching)): cached input is 0.1× input (0.05× on GPT-6.1 Sol). From GPT-5.6 on, cache writes are billed too, at 1.25× (30-minute lifetime) or 2.5× (12-hour). Example: gpt-6-astra costs $10 input, $1 cached, $12.50 per 30-minute write and $50 output.

### 2.2 Why compaction pays at all with reads this cheap

With caching, a long context isn't paid at the input price on every call; it's paid at the cache-read price. On Opus 5.5 that is $0.20 per million, so a 900K context costs **$0.18 per call** just to re-read. That sounds small, but a busy agent makes 300–500 calls a day: at the median 422K context it is about $0.08 a call before writing anything new. That is why cache reads are 63% of the cost.

Compaction trades a one-time cost for a smaller context on every later call:

- **One-time:** summary pass (read context C from cache, write summary S as output) + cache rewrite on the next call (~50K at the write price) + rework (re-reading files; measured average ~2.4K tokens, plus a few calls).
- **Per later call:** saves (C − C_after) × cache-read price.

**Rule:** compact when the calls still to come in this session, N, satisfy
N × (C − C_after) × read price > C × read price + S × output price + (rewrite + rework) × write price.

On Opus 5.5, with C_after ≈ 70K, S ≈ 8K and rewrite plus rework ≈ 62K:

| Compact at C | One-time cost | Saved per later call | Break-even N |
|---:|---:|---:|---:|
| 967K (today) | $0.85 | $0.18 | 5 calls |
| 600K | $0.78 | $0.11 | 7 calls |
| 400K | $0.74 | $0.07 | 11 calls |
| 250K | $0.70 | $0.04 | 20 calls |
| 150K | $0.69 | $0.02 | 43 calls |

Agents here make hundreds of calls per session, so even at 250K compaction almost always pays *if the session continues*. Where it doesn't pay is at the end of a session, or right before a task that would start fresh anyway.

### 2.3 Replaying the measured sessions with an earlier compaction point

I replayed every main-session call: when the context crossed a threshold T, the session paid one compaction (summary pass, ~50K rewrite, ~12K rework at the 1-hour write price) and every later call read less, by the amount dropped. Writes and output were left as they were.

| Compact at | Main-session cost, relative to today | Extra compactions in 30 days |
|---:|---:|---:|
| today (~967K) | 100% | — |
| 800K | 88% | +30 |
| 600K | 77% | +40 |
| **400K** | **68%** | **+84** |
| 300K | 63% | +131 |
| 200K | 58% | +253 |
| 150K | 58% | +438 |

The model doesn't price lost work beyond the measured re-reads, so below ~300K, where compactions multiply, it overstates the saving. 400K is the knee: a third of the cost for about three compactions per agent-day.

### 2.4 Compact, or start fresh?

A **fresh session with a handoff** skips the summary pass. AgentMux already keeps a running summary of every conversation, written by a cheap Haiku call after turns (§3.4). The new session starts with the system prompt and tools (cached), memory, and a packet of at most ~24K characters (~6K tokens). Its one-time cost is one cache write of ~30–40K tokens (~$0.30 on Opus 5.5) plus a catch-up turn, against ~$0.75 for a compaction, and it starts at ~40K instead of ~70K. It keeps less: the packet holds the running summary, identifiers and the recent exchange, not the files the agent had open. So a fresh session suits a **new, unrelated task**, and compaction suits **the same task continuing**.

### 2.5 Quality, not just cost

Model accuracy falls as context grows, well before the window is full: performance is worst for information in the middle of a long context ([Lost in the Middle](https://arxiv.org/abs/2307.03172)); 18 models degraded with input length even on simple tasks ([Chroma, Context Rot, 2025](https://www.trychroma.com/research/context-rot)). Running at 400K–967K for most of a session costs money **and** accuracy, so a lower threshold is not only cheaper.

---

## 3. Options

### 3.1 Claude Code

| | (a) Compact itself between turns with a focus note | (b) Fresh session at a task boundary with a handoff | (c) Leave it to the CLI's auto-compaction |
|---|---|---|---|
| **What the CLI allows from outside** | A stream-json user message whose content is exactly `/compact` triggers a manual compaction (verified on CLI 2.1.218, [REPORT_TOKEN_ACCOUNTING §6](REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md)). `/compact <focus>` sent the same way works too: verified on the pinned CLI 2.1.288 (§3.3). There is no compact control request; srv already sends `interrupt`, `get_settings`, `get_context_usage` and MCP control requests on the same stdin. | Spawn without `--resume`: srv's continuation path already builds a handoff packet for the new session's first message. | The auto-compact window can be set from 100K to 1M: `CLAUDE_CODE_AUTO_COMPACT_WINDOW` (wins), the `autoCompactWindow` setting, `--autocompact` or `/autocompact` ([model config](https://code.claude.com/docs/en/model-config)). `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` can only lower it, and the CLI's reported threshold ignores it, so the pane's countdown would be wrong ([REPORT_AGENT_PANE_CONTEXT_METER §9](REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md)). |
| **What AgentMux has today** | The pane's Compact button sends `/compact` as the user, idle only (`AgentComposerStrip.tsx` `canCompact`). A `PreCompact` hook shows "Compacting…"; the `compact_boundary` frame is detected; memory is re-delivered by the `SessionStart` hook with source `compact`; a catch-up turn follows a manual compact. | The continuation packet (`backend/continuity.rs`), with the running summary (`continuity_state.rs`). Used when a resume fails, not on request. "New session" in the pane archives the transcript and the summary: a reset, not a handoff. | Nothing configured: every agent compacts at ~967K. |
| **What a MuxAPI action needs** | A self action deferred to the end of the turn; srv writes `/compact <focus>` straight to the process's stdin, not through message delivery (which would wrap it and append held `[AgentMux]` notes). | A controller method: graceful stop, clear the session id, respawn without `--resume`; append the agent's own handoff note to the packet. | A per-agent launch setting (environment variable). |
| **Mid-task risk** | None if deferred to the turn's end. | None if deferred. | High: 85% of auto compactions land mid-task. |
| **Keeps** | The summary plus ~5 re-attached files; the cache prefix (tools, system) survives. | Running summary, ≤40 identifiers, the last request, the recent exchange (≤24K chars). | Same as (a), without a focus. |

### 3.2 Codex

| | (a) Compact | (b) Fresh session | (c) Auto-compaction |
|---|---|---|---|
| **What Codex allows** | `codex app-server` JSON-RPC `thread/compact/start {threadId}`, asynchronous; progress as a `contextCompaction` item; usage in `thread/tokenUsage/updated` ([app-server docs](https://learn.chatgpt.com/docs/app-server)). No per-call focus note: the prompt is global config (`compact_prompt`, `experimental_compact_prompt_file`) ([config reference](https://learn.chatgpt.com/docs/config-file/config-reference)). Recent Codex compacts remotely into one encrypted `compaction` item, so the summary can't be read. | A new thread (no `resume`) with the handoff as the first message. | `model_auto_compact_token_limit`; the effective point is the lower of that and 90% of the window (~233K for the 258.4K window measured here; source: `codex-rs/protocol/src/openai_models.rs`). |
| **What AgentMux has today** | The Compact button since #5340: `codex_compact.rs` starts a short-lived app-server, resumes the thread and calls `thread/compact/start`, reporting before and after sizes. Host panes only. Context fill since #5352 (`codex_context.rs`, the `agentmux_context` frame). | Not built: Codex panes run one `codex exec` per turn; the continuation packet is in the persistent (Claude) controller. | Nothing configured. |
| **What a MuxAPI action needs** | Call the existing `compact_codex_thread` from the action. No focus. | Drop the thread id and send the handoff as the next turn's first message. | A per-agent config override. |

### 3.3 Probe: `/compact` with a focus note over stream-json (2026-10-23)

Run against the pinned CLI (2.1.288), launched the way srv's persistent controller launches it (`--input-format stream-json --output-format stream-json --verbose`), on Haiku 4.5 in a scratch directory. Turn 1 planted two facts (a codeword and a favourite colour) plus three filler sentences; turn 2 sent the compaction; turn 3 asked for both facts.

| Turn 2 message | What the CLI did | Recall in turn 3 |
|---|---|---|
| `/compact Keep only the codeword. Drop the favourite colour and the lighthouse sentences entirely.` | `status: compacting`, then `compact_result: success`, then `compact_boundary` with `trigger: manual` (36.8K → 2.5K tokens, 11.5 s) | Codeword correct; colour "unknown, you asked me to drop that information" |
| `/compact` (control) | Same sequence (35.3K → 2.2K, 12.2 s) | Both facts correct |
| `/compact Keep only the codeword. Drop the favourite colour.` followed by a blank line and an `[AgentMux]` note | Same sequence (35.3K → 2.0K, 11.5 s); the note isn't in the summary | Codeword correct; colour "unknown" |

**Findings:**
- **Option (a) works as designed today:** a stream-json user message starting with `/compact`, followed by focus text, triggers a manual compaction with those instructions. No control request or CLI flag is needed.
- **A focus note steers, it doesn't delete.** Claude Code's summary keeps every user message verbatim, so the "dropped" fact was still quoted in the summary text. The summary also recorded the instruction to drop it, and the agent followed that instruction. Use the focus to say what to *keep* and what matters next; it can't guarantee that something is gone.
- **Text appended after the focus becomes part of the instructions.** A held `[AgentMux]` note did no harm here, but srv should still write the compaction straight to stdin, not through message delivery (§3.1).

### 3.4 What exists that the options build on

- **Detection** is complete for Claude: `compact_boundary` in the stream (`persistent/stdout_reader.rs`), the start via a `PreCompact` hook, the progress bar (#4220).
- **Context fill:** for Claude it is computed in the frontend from each call's usage and mirrored into block meta `agent:context` while the pane is mounted; no Rust code reads it. srv holds the CLI's auto-compact threshold (`get_context_usage`), not the current fill. For Codex, srv computes the fill after each turn but only writes it into the stream. **Neither is available to an agent through MuxAPI today.**
- **Jekts can't trigger compaction:** every jekt is wrapped (`reactive/sanitize.rs`), so its text never starts with `/`. This is the right behaviour: another agent must not be able to compact you.
- **The continuation packet** (`continuity.rs`) is a working fresh-session handoff: running summary, identifiers, the last request and the recent exchange, capped at ~24K characters, prefixed to the new session's first message, followed by a catch-up turn.

---

## 4. The decision rule

The rule runs in srv at the end of every turn, using numbers srv already has, so it costs no model tokens. It only asks the agent something when acting would pay, and the agent answers with what it already knows: whether its next work is related to what is in its context.

**Inputs** (srv, per pane, at turn end): the context C and window W from the last call's usage; the CLI's auto-compact point A; the calls since the last compaction or fresh start; the agent's model (for prices); whether the turn just completed a task boundary that srv can see (a Kanban task completed or claimed, a PR merged, `Tasks submit`).

**Never mid-turn.** The rule runs only after a turn ends, and an action an agent requests during its turn is deferred until the turn ends. A compaction that would land mid-task is exactly the 85% case this replaces.

**The rule (Opus 5.5 numbers; scaled by the model's prices):**

1. **C < 250K:** do nothing. (Break-even is ≥20 more calls and the saving is small.)
2. **A task boundary just happened** (srv saw it, or the agent says so) **and C ≥ 150K:** recommend a **fresh session with a handoff (b)**, unless the agent says the next work continues the same task.
3. **C ≥ 400K and the work continues:** recommend **compaction with a focus note (a)**. The agent writes the focus ("keep: the PR number, the failing test, the files I'm editing; drop: the CI logs and the investigation that didn't pan out").
4. **Backstop:** a lowered CLI auto-compact point (e.g. a 600K auto-compact window, so ~567K) catches sessions that never reach a boundary. It may land mid-task, but at 567K instead of 967K, and it should rarely fire.
5. **Rate limit:** no compaction within 50 calls of the previous one, and never in the same turn as a catch-up turn (a compaction triggers a catch-up turn, which must not trigger another compaction).

**How the agent decides without spending more than it saves.** srv puts one `[AgentMux]` note after the turn, only when rule 2 or 3 applies: for example `Context 512K of 1M. Compacting now costs about $0.75 and saves about $0.09 a call; start fresh instead if your next work is unrelated.` That is ~40 tokens, once per crossing. The agent replies by calling the action with a focus note or a handoff note (~100–300 output tokens, $0.002–0.006), or by ignoring it. The judgment that needs the model, whether the next work is related, is information the agent already holds at the end of its turn; everything else is arithmetic in srv. The agent never has to estimate its own tokens.

**Why not let the agent decide unprompted.** It can't see its context fill today, and even if it could, checking every turn would cost tokens and attention every turn. srv sees the numbers for free.

---

## 5. Best practice elsewhere

**Anthropic.**
- The context-engineering guidance treats context as a finite resource with diminishing returns: clear raw tool results first (the lightest form of compaction), compact long sessions, keep notes outside the context (memory), and use sub-agents that return 1–2K-token summaries ([Effective context engineering for AI agents, 2025-09-29](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)).
- The API has server-side **compaction** (beta `compact-2026-01-12`, a `compaction` block that replaces earlier history) and **context editing**, which clears old tool results or thinking blocks without summarizing (beta `context-management-2025-06-27`, strategies `clear_tool_uses_20250919` and `clear_thinking_20251015`), plus a **memory tool** (`memory_20250818`) ([compaction](https://platform.claude.com/docs/en/build-with-claude/compaction), [context editing](https://platform.claude.com/docs/en/build-with-claude/context-editing), [memory tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)). These are API features; AgentMux runs the Claude Code CLI, which manages its own context, so they are not levers AgentMux can turn on from outside.
- **Claude Code** auto-compacts at the model's limit: about 967K on 1M-window models, 200K otherwise. The window can be set from 100K to 1M with `/autocompact 500k`, `--autocompact`, the `autoCompactWindow` setting or `CLAUDE_CODE_AUTO_COMPACT_WINDOW` (the variable wins); `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` can only lower the point; `DISABLE_AUTO_COMPACT` keeps `/compact` but turns off automatic compaction ([model config](https://code.claude.com/docs/en/model-config), [environment variables](https://code.claude.com/docs/en/env-vars)). `/compact focus on the auth bug fix` takes focus instructions, and a "Compact instructions" section in CLAUDE.md applies to every compaction ([commands](https://code.claude.com/docs/en/commands), [costs](https://code.claude.com/docs/en/costs)). After a compaction the system prompt, project CLAUDE.md, auto memory and the plan are re-injected, up to 5 recently modified files are re-read (larger than 5K tokens: as a path only), and nested CLAUDE.md files and path-scoped rules are summarized away ([context window](https://code.claude.com/docs/en/context-window)).
- The guidance is to compact "at a natural break… between tasks", with a focus "before starting a long new task", and to `/clear` between unrelated tasks ([prompt caching](https://code.claude.com/docs/en/prompt-caching), [best practices](https://code.claude.com/docs/en/best-practices)). The summary request reuses the cached prefix while it is warm, so compacting mid-session costs a fraction of what the context size suggests; compacting after the cache expired reprocesses everything ([prompt caching](https://code.claude.com/docs/en/prompt-caching); background in [Prompt caching is everything, 2026-04-30](https://claude.dev/blog/lessons-from-building-claude-code-prompt-caching-is-everything)). With the 1-hour cache, `idleCompaction` lets Claude Code compact an idle session shortly before the cache would expire.
- **Hooks:** `PreCompact` (matcher `manual` or `auto`) can block a compaction with exit code 2; `PostCompact` can't; `SessionStart` runs with source `compact` and can add context ([hooks](https://code.claude.com/docs/en/hooks)). Blocking a mid-task auto compaction until the turn ends looks tempting but would let the context overflow at the window, where Claude Code stops with a "thrashing" error ([troubleshooting](https://code.claude.com/docs/en/troubleshooting)); a lower window plus self-compaction at boundaries is safer.
- **From a program:** the Agent SDK triggers it by sending `/compact` as a prompt and reports a `compact_boundary` message with `trigger` and `pre_tokens`; there is no `compact()` method, and `getContextUsage()` returns the tokens, the window and the auto-compact threshold ([SDK slash commands](https://code.claude.com/docs/en/agent-sdk/slash-commands), [TypeScript SDK](https://code.claude.com/docs/en/agent-sdk/typescript)). AgentMux already uses the stream-json equivalents (`/compact` as a user message, the `get_context_usage` control request).
- **Long-running work:** Anthropic's harness for long-running agents doesn't rely on compaction alone. It keeps a progress file, a feature list and git commits, and does one feature per session ([Effective harnesses for long-running agents, 2025-11-26](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)). That is the fresh-session-with-handoff pattern of option (b).

**OpenAI Codex.** Auto-compaction at 90% of the window by default, configurable with `model_auto_compact_token_limit`; a manual `/compact`; `PreCompact` and `PostCompact` hooks (non-blocking); a handoff-style default compaction prompt ("create a handoff summary for another LLM") ([config reference](https://learn.chatgpt.com/docs/config-file/config-reference), [commands](https://learn.chatgpt.com/docs/developer-commands?surface=cli), [openai/codex#19905](https://github.com/openai/codex/pull/19905)). The Responses API offers `POST /responses/compact` and server-side compaction via `context_management` ([compaction guide](https://developers.openai.com/api/docs/guides/compaction)).

**Other agents.**

| Agent | Approach |
|---|---|
| Cursor | Auto-summarizes near the top of the active window; staff advise running `/summarize` "at natural task boundaries, not in the middle" ([forum, 2026-08-13](https://forum.cursor.com/t/how-does-summarize-actually-work/167893)). RL-trained self-summaries: ~1K tokens, 50% fewer compaction errors than prompted summaries ([Cursor blog, 2026-03-17](https://cursor.com/blog/self-summarization)). |
| Cline | `/smol` (`/compact`) compresses in place; `/newtask` hands the plan, files and next steps to a fresh task: a handoff ([docs](https://docs.cline.bot/core-workflows/using-commands.md)). |
| Goose | Auto-compacts at 80% of the limit or 225K tokens, whichever comes first; `GOOSE_AUTO_COMPACT_THRESHOLD` ([docs](https://goose-docs.ai/docs/guides/sessions/smart-context-management)). |
| Roo Code | Condensing on by default, threshold slider, a manual Condense button, custom prompt ([docs, 2026-05-15](https://roocodeinc.github.io/Roo-Code/features/intelligent-context-condensing)). |
| OpenHands | LLM-summarizing condenser at a size threshold; ~2× cheaper per turn with no loss on SWE-bench (54% vs 53%) ([blog, 2025-04-09](https://openhands.dev/blog/openhands-context-condensensation-for-more-efficient-ai-agents)). |
| SWE-agent | Collapses observations older than the last 5 to one line; better than the full history (18.0% vs 15.0%) ([paper](https://arxiv.org/html/2405.15793v3)). |
| Manus | KV-cache hit rate is "the single most important metric"; keep the prefix stable and append-only; use the file system as memory; prefer reversible compaction (keep the path, drop the content) over summaries ([blog, 2025-07-18](https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus)). |

**Research.**
- Masking old observations matched LLM summarization's solve rate at about half the cost of no management; summaries made runs ~15% longer and their calls cost up to 7.2% of a run ([The Complexity Trap, JetBrains, 2025](https://arxiv.org/abs/2508.21433)).
- In GitHub Copilot production traces, 7.8% of sessions compact and those sessions use 44.2% of all tokens; a compaction cuts the cache hit rate by a median 66.1% ([arXiv 2608.00101, 2026](https://arxiv.org/html/2608.00101v1)).
- Optimize tokens per task, not per request: over-compression forces re-fetching ([Factory, 2025-12-16](https://factory.com/news/evaluating-compression)); at 5× compression, retrieval calls went from 21 to 64 ([arXiv 2608.16370, 2026](https://arxiv.org/abs/2608.16370)).

**Consensus:** compact at task boundaries, not mid-task; fire well before the window is full but not every turn; keep the cache prefix stable; keep enough to avoid re-fetching (paths, decisions, the next step); prefer a handoff to a fresh context when the next task is unrelated.

---

## 6. Recommendation

### 6.1 Phase 1: a lower auto-compact backstop (configuration only)

Launch Claude agents with a 600K auto-compact window (`CLAUDE_CODE_AUTO_COMPACT_WINDOW=600000`, so compaction at ~567K) instead of the full 1M.

- **Saves:** ~23% of main-session cost in the replay, for ~40 more compactions a month on the measured machine.
- **Risk:** auto compactions still land mid-task, more often than today (but at 567K, with a smaller summary to write). Check that the pane's context meter picks up the lower threshold (`get_context_usage` reports it).
- **Touches:** the Claude launch environment in `crates/srv/src/backend/providers/claude.rs` or a per-agent setting; possibly a setting in the agent's configuration UI.

### 6.2 Phase 2: self-compaction at the end of a turn

- **MuxAPI:** `Agents` action `context` (the caller's own tokens, window, auto-compact point) and `compact` (self only, optional `focus`, deferred to the end of the current turn; Claude and Codex).
- **srv:** a `compact_when_idle(focus)` on `PersistentSubprocessController` that writes `/compact <focus>` directly to stdin at the turn boundary, marked as a system-origin turn; Codex calls the existing `compact_codex_thread`. srv keeps each pane's latest context reading itself (parse usage in `stdout_reader.rs`; keep the Codex reading).
- **The rule of §4** with the one-line hint, and the rate limit.
- **Saves:** compacting at ~400K at boundaries saved 32% in the replay, with compactions moving from mid-task to boundaries.
- **Risk:** loss of detail (§1.5) at three compactions per agent-day instead of one every three days; mitigated by the focus note and by boundaries. A loop of compaction and catch-up turns; mitigated by the rate limit. `/compact <focus>` over stream-json works on the pinned CLI (§3.3); recheck it when the pin moves.
- **Touches:** `crates/mcp/src/tool_schemas.rs` and the `Agents` family handler; `crates/srv/src/server/routes.rs` (`/api/v1/agent/self/context`, `/api/v1/agent/self/compact`); a handler modelled on `server/app_api/mcp_health.rs` (resolves the caller's own pane) and the turn-end scheduling of `sagas/self_quit.rs`; `blockcontroller/persistent/input.rs` and `stdout_reader.rs`; `subprocess/codex_compact.rs`; the hint in `reactive/held_notes.rs`.

### 6.3 Phase 3: a fresh session with a handoff

- **MuxAPI:** `Agents` action `fresh` (self only, `handoff`: task number, spec path, open PRs, decisions, next step), deferred to the end of the turn.
- **srv:** a controller method that stops gracefully, clears the session id and respawns without `--resume`, so the continuation path builds the packet; the agent's handoff note is added to the packet (`continuity.rs` `build_continuation_packet`). For Codex, drop the thread id and send the handoff as the first message.
- **Risk:** detail beyond the packet survives only in the running summary; forked panes and the session lease need care; must not use the pane's "New session", which archives the summary.
- **Touches:** `blockcontroller/persistent/lifecycle.rs`, `spawn.rs`, `resume_retry.rs`, `backend/continuity.rs`, the Codex host spawn.

Compacting **other** agents stays the user's decision: if it is ever added, it takes the 15-second user-override window that stopping another agent's pane uses, and an audit entry.

### 6.4 What to measure afterwards

Run Phase 1 on all agents and Phase 2 on two of the four busiest for two weeks, then compare with the other two and with this baseline:

- tokens and API-equivalent cost per agent per day, split into cache read, cache write and output (the scripts behind §1 can be rerun);
- the median and p90 context per call;
- compactions per agent-day, split mid-task, boundary and self-initiated; fresh sessions per day;
- re-reads after compaction (files touched in the 50 calls before, opened again in the 20 after) and calls to finish the interrupted step;
- people repeating themselves after a compaction (the repeat-phrase scan);
- weekly usage-limit hits, which are what the operator actually feels;
- task outcomes: PRs merged per agent-day, review rounds per PR, to catch a quality loss the token numbers would hide.

**Success:** ≥20% lower cost per agent-day with no rise in review rounds or re-reads per task.

---

## Appendix: method notes

- Days are UTC. A few calls carry earlier timestamps from history copied into newer transcripts; they are counted once by response id.
- The summary call's usage is estimated (Claude Code doesn't log it), calibrated on the two compactions that sit between cumulative cost snapshots. If that call missed the cache it would cost about ten times more; the snapshots show it didn't.
- The replay in §2.3 keeps each session's writes and output unchanged and moves only cache reads. It prices rework as 12K written tokens per compaction (the measured average is lower, ~2.4K) but does not price extra calls or lost work.
- Sample: 25 natural compactions from three agents. Treat the percentages as descriptive, not as estimates for every workload.
- No agent names, spend totals, transcript text, paths outside this repository, account details or credentials are reproduced here.
