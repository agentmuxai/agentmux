# Spec: The Memory pane in tokens — what memory costs each agent's context

**Status:** proposed. Nothing here is built yet.
**Date:** 2026-10-15
**Author:** Lazo
**Verified against:** `agentmux` `main` @ `3fc33360be` (2026-10-15), by reading code.

## 1. Ask

The owner, 2026-10-15: "we need the memory pane to be token, not KB focused. … revamp it so [it brings] out the token information, use best practices aggregates too."

A memory entry matters for what it costs an agent's context window, and that cost is counted in tokens. The pane counts it in bytes.

## 2. Today

**Every size in the Memory pane is bytes.** It has no token figure anywhere, no total per agent or per scope, and no measure against a context window.

| Surface | Shows | Comes from |
|---|---|---|
| Global tile (`global-bundle/global-bundle-manager.tsx:40-44`, `entryMetaLabel`) | "1.2 KB · 2d ago" | frontend `utf8Bytes(entry.instructions)` |
| Global "CLAUDE.md" tile (:167-184) | "`<size>` · provider config" | frontend, from `GetClaudeGlobalConfigCommand` |
| Global "Combined preview" tile (:186-195) | "N entries · `<size>`" | frontend `formatGlobalBundleBlock` |
| Personal agent card (`MemoryAgentCard.tsx:44-53`) | "N files" | `agent:memory:list` length |
| Personal file tile (`MemoryFileCard.tsx:45-50,69-71`) | "439 B · 2d ago" | srv `NativeMemoryFileMeta.size_bytes` (on-disk length) |
| Full views, history (`GlobalMemoryFullView.tsx`, `NativeMemoryFileView.tsx`, `MemoryHistory.tsx`) | no size | — |
| Personal sort (`MemoryAgentFilterBar.tsx`) | `name \| count \| provider` | file count |

**The byte figures are also not what an agent receives.**
- **Global tiles** count the raw `instructions`. What is injected also carries the `# [Workspace] <name>` heading, the `---` separators and, for system entries, the override preamble (`storage/bundles.rs:186-243`).
- **"Combined preview"** ignores Operator Config's host/container targeting (`operator_config_seed.rs:131-150`), so it matches no single agent exactly.
- **Two costs are shown nowhere:**
  - an agent's picked Bundles, composed into its startup file after Global Memory (`format_agent_bundle_block`);
  - the files the CLI loads by itself (`backend/startup_files.rs`).

**Tokens already exist, outside the pane.**
- **Context delivery card** (`agent/components/ContextDeliveryCard.tsx`): it shows "~N tok (est.)" per item and in total, from srv's `agentmux_memory_injected` frame (`size_bytes`, `tokens`, `source_tokens`).
  - It covers only what one Claude session was given, and you see it in the transcript, not in the Memory pane.
- **Estimators:**
  - TS: `estimateTokenCount(text) = ⌈UTF-16 length / 4⌉` (`frontend/util/format-count.ts:62-64`). Its doc says every use must read "~N tok (est.)".
  - Rust: `⌈chars / 4⌉`, written inline five times in `memory_delivery_handlers.rs` (:751, 845, 896, 1041, 1043).
  - No tokenizer library is a dependency.
- **Context windows** are per model: `resolveContextWindow` (`store/agent-pane-state/context-reading.ts:182-210`) takes a reported window, then a learned one, then the model table, else unknown. The compaction point is the window − 33K (`context-window.ts:87-89`).
- **`memorySizeBand`** (`memory-reinjection.ts:50-61`) bands Personal Memory tokens against 10% of the window, but only for the reinjection card.

**What reaches each provider.** The Memory pane must show the cost per agent, because it differs by provider.
- **Every provider except Kimi:** the startup file. It holds Operator Config, then Workspace Global Memory, then the agent's picked Bundles, then soul/agentmd and the skills index (`agent_open.rs:1070-1090`, `agent_config.rs:60-139`). Kimi gets no file (`agent_config.rs:55-57`).
- **Claude Code only:**
  - Personal Memory topic files arrive in full through the SessionStart hook, capped at 8 × 9,000 characters per delivery (`memory_delivery.rs:22,27`).
  - The CLI also loads `MEMORY.md` itself, but only its first 200 lines / 25 KB. AgentMux never checks that limit.
- **Non-Claude providers** get no Personal Memory today (`SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md` P4).
- **A user-owned AGENTS.md, GEMINI.md or QWEN.md** is never overwritten, so Global Memory doesn't reach that agent by file (`agent_config.rs:1191-1224`).
- **Operator Config** is about 4.5K estimated tokens over 6 entries, in every agent's file. Container agents leave out the host-only entries.

## 3. Principles

1. **Tokens first, bytes second.** Every size reads in tokens. Bytes move to the tooltip, next to the token count and the line count.
2. **Count what is delivered, not what is stored.** A Global entry's tokens are those of its injected text (heading, separators, preamble), for the agent kind in view. An agent's total is what its next launch will receive, from the same composition code the launch uses.
3. **Say it is an estimate.** Every count reads "~1.2k tok" and has a legend: "Estimated: characters ÷ 4". A count calibrated from a real session (§6, phase 4) says so instead. This keeps the house rule (`format-count.ts:56-58`, `SPEC_CONTEXT_VISIBILITY_2026_06_17.md`).
4. **Measure against the context, and only when the window is known.** A share of the window is shown only where the agent's window is known (`resolveContextWindow`). Otherwise the pane shows tokens alone; it never guesses a window.
5. **One estimator.** The same rule in srv and the frontend, so the pane, the delivery card and the reinjection card agree to the token.

## 4. Counting

### 4.1 The estimator

- **One function in each language, with the same rule:** `⌈Unicode scalar values / 4⌉`.
  - Rust: `agentmux_common::tokens::estimate_tokens(&str) -> u64`. The five inline copies in `memory_delivery_handlers.rs` call it.
  - TS: `estimateTokenCount` counts code points instead of UTF-16 units.
  - The two agree byte for byte, and a shared test vector, including emoji and CJK, checks it in both.
- **No tokenizer and no network count.**
  - A BPE tokenizer (e.g. o200k) is model-specific, adds megabytes, and is still wrong for Claude.
  - Anthropic's count API needs a key and a network call per edit.
  - Characters ÷ 4 is the honest baseline. Phase 4 corrects it per agent from that agent's own measured usage (§6).

### 4.2 srv fields

Every new field is wire-only, computed when served. None needs a database column: a new column would bump the schema and lock older builds out of a shared store (`operator_config_seed.rs:76-80`).

| RPC | New fields | Notes |
|---|---|---|
| `agent:memory:list` (`NativeMemoryFileMeta`) | `tokens_est`, `lines` | srv already reads a changed file to mirror it (`native_memory_handlers.rs:1048-1086`). For an unchanged file it counts from the mirror row's content, so the list reads no extra files. |
| `globalmemory:sections` (`GlobalMemorySection`) | `tokens_est` | Of the injected `text`, per agent kind (`block_id`, or a new `agent_kind: host \| container` when there is no pane). |
| `agent:memory:history`, `globalmemory:history` (`*VersionMeta`) | `size_bytes`, `tokens_est` | From the version row's stored content (`migrations.rs:974-982`). This drives the size-over-time aggregates (§5.4). |
| **new** `memory:launchcost` | per agent, below | A dry run of the launch composition. |

**`memory:launchcost { agent_id } →`**

```
{
  agent_id, provider, startup_file,          // e.g. "AGENTS.md", or null (Kimi)
  model, context_window, window_source,      // from the agent's last reading (block meta `agent:context`), else the model table, else null
  delivered_by_file: bool,                   // false when the startup file is user-owned (§2)
  parts: [ { kind, id, name, tokens_est, size_bytes, lines, via } ],
  total_tokens_est,
  memory_md: { lines, size_bytes, loaded_lines, over_limit } | null   // Claude only
}
```

- **`kind`** is one of `operator_config | workspace_global | bundle | soul | agentmd | skills_index | personal | cli_startup_file`.
- **`via`** is one of `startup_file | session_hook | cli`.
- **Each part comes from the code the launch itself uses:**
  - `global_bundles_for_agent` and `format_global_bundle_block`;
  - `format_agent_bundle_block`;
  - `build_config_files`, without writing anything;
  - for Claude, the hook's Personal Memory list;
  - `startup_files.rs` for the files the CLI loads by itself.
- **The test:** the sum of the startup-file parts equals the estimate of the file a real launch writes.

## 5. The pane

### 5.1 Units everywhere

- **`MemoryTile`** (`{icon, title, badges, meta}`, shared by both grids) gains a `tokens` chip, "~1.2k tok" via `formatCompactNumber`, which replaces the bytes in `meta`.
  - Its tooltip reads "~1,234 tokens (est.) · 4.8 KB · 86 lines".
- **Sort orders:**
  - Personal's sort gains `tokens`, which becomes the default.
  - Global gains "by tokens" beside injection order.
- **Full views** show the same counts in their header.
- **History rows** show each version's tokens, and the change from the version before it ("+312 tok").

### 5.2 Global section

- **A summary strip above the grid:**
  - "Every agent starts with ~5.1k tok of Global Memory"
  - a stacked bar: Operator Config, then Workspace entries
  - a **Host / Container** toggle, so the figure is what that kind of agent really gets
- **Each tile** shows its tokens and its share of the strip's total ("~892 tok · 17%").
- **The "Combined preview" tile becomes "Delivered to a host agent".** It shows that agent kind's total, from `globalmemory:sections`.
- **A note under the strip** names the providers for which Global Memory isn't sent by file:
  - Kimi, which has no startup file;
  - agents whose startup file is user-owned (`delivered_by_file: false`).
  - `noFileProvidersAtom` already computes the first group (`global-bundle-model.ts:252-256`).

### 5.3 Personal section

- **Agent card:** "~18.4k tok · 22 files", a budget bar, and its share of the window when the window is known ("4% of 200k"). The bar is drawn against the agent's own window, not a global constant.
- **The bar's colour** comes from its share of the window. The thresholds are explicit, and the words beside the bar carry the meaning, not the colour:

  | Band | Share of window | Words |
  |---|---|---|
  | low | under 5% | — |
  | mid | 5–7.5% | — |
  | high | 7.5–9% | "large" |
  | critical | 9% and over | "very large: trim it or split it" |

  The bands are those of `memorySizeBand` (0.5 / 0.75 / 0.9 of a budget of 10% of the window, `memory-reinjection.ts:50-61`), applied to the agent's **launch total** (§4.2) instead of its Personal Memory alone. The pane and the reinjection card then name the same band for the same agent.
- **File tiles** show tokens and their share of the agent's Personal Memory.
- **`MEMORY.md` (Claude):** "Loaded by Claude Code: first 200 lines / 25 KB". When it is over either limit, the tile shows "212 lines: lines 201–212 aren't loaded" in warning text.
- **Providers that get no Personal Memory** (every non-Claude provider today): the card says "Not delivered to Codex agents", and their files show tokens without a share of any window. Otherwise the pane would imply a cost the agent doesn't pay.

### 5.4 What this agent starts with

- **A new view, opened from an agent card ("Launch cost").** It is the §4.2 breakdown as a stacked bar and a table: part, how it arrives (startup file / session hook / loaded by the CLI), tokens and share.
- **Its footer:** the total; the share of the window; and the room left before compaction at launch (window − 33K − total, or the CLI's reported threshold).
- **"Last launch actually delivered"** (Claude only): the matching figures from the agent's most recent `agentmux_memory_injected` frame, beside the estimate. Items cut by the 72,000-character hook cap read "cut".

### 5.5 Aggregates

These are the common views of a cost breakdown, sized to what this pane holds: tens of agents and hundreds of entries. The pane works them out from the RPCs above, and nothing new is stored.

| Aggregate | Where | Why |
|---|---|---|
| Total per scope (Operator Config, Workspace, each agent's Personal Memory) | summary strips, cards | The first question: how much? |
| Share of total, and share of window | every tile, card and bar | Puts a number in proportion |
| Top contributors: "The 3 largest entries are 61% of Global Memory" | Global strip, launch-cost view | Pareto: where trimming pays |
| Distribution: median and p90 file size; "4 files over p90" | Personal strip | Spots outliers without a list |
| Fleet summary: agents with memory; total; median per agent; agents over 2× the median, listed | Personal strip | Spots the agent whose memory grew |
| Growth: change over 7 and 30 days, and a sparkline of the total over time | agent card, Global strip, file full view | From the version history (§4.2). Shows growth before it becomes a problem |
| Grouped by startup file (CLAUDE.md, AGENTS.md, GEMINI.md, QWEN.md, none) | Global strip's provider note | Uses `groupProvidersByStartupFilename()`, which exists but isn't shown |

- **Medians, not means.** A single 70K-token file would make a mean misleading.
- **No aggregate claims precision the estimate doesn't have.** Totals round like single counts ("~18.4k").

## 6. Phases

1. **Tokens in place of bytes.**
   - `estimate_tokens` in `agentmux_common`; `estimateTokenCount` counts code points; a shared test vector.
   - The `tokens_est` and `lines` fields on `agent:memory:list` and `globalmemory:sections`, with Host / Container for Global.
   - The token chip on `MemoryTile`; tooltips with bytes and lines; sort by tokens.
   - The Global summary strip, and the Global tiles counted on injected text.
2. **Per-agent cost.**
   - `memory:launchcost`.
   - Agent card totals and the budget bar.
   - The launch-cost view, including "Last launch actually delivered".
   - The `MEMORY.md` limit check, and the "not delivered to …" notes.
3. **Aggregates over time.**
   - Version-meta `size_bytes` / `tokens_est`.
   - History deltas, sparklines, the 7- and 30-day growth figures, and the fleet summary.
4. **Calibration (optional).**
   - The pane already knows real token counts for a Claude agent: the first call's `message_start` usage minus its first user message.
   - A per-agent factor (real ÷ estimated, from the last launch) can scale that agent's estimates. Its label then reads "~ (calibrated)".
   - The same method applies to any provider whose usage is measured (`opencode-acp`, #4930).

## 7. Out of scope

- Changing delivery: caps, what reaches which provider, Personal Memory for non-Claude providers (`SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md` P4).
- Real tokenizers and count APIs (§4.1).
- Editing tools such as "trim this file". The pane shows the cost; acting on it is a later spec.

## 8. Tests

- **Estimator:** Rust and TS give the same count on the shared vector, including emoji, CJK and combining marks. The delivery card's counts are unchanged except where code points and UTF-16 units differ.
- **Global tokens:** a section's `tokens_est` is the estimate of the exact injected text. Host and container totals differ by exactly the host-only entries.
- **`memory:launchcost`:**
  - the startup-file parts sum to the estimate of the file a real `build_config_files` would write (golden test per provider family: CLAUDE.md, AGENTS.md, GEMINI.md, none);
  - `delivered_by_file` is false for a user-owned AGENTS.md;
  - `memory_md.over_limit` is true at 201 lines and at 25 KB + 1.
- **Pane:**
  - the chip and tooltip render;
  - sorting by tokens;
  - a band's words appear when the colour does;
  - no window share is shown when the window is unknown;
  - non-Claude Personal cards say "Not delivered".
- **Aggregates:** median and p90 on small sets (1, 2 and 3 files), top-N shares that sum correctly, and growth over an empty history.

## 9. Decisions for the owner

1. **Estimator.** *Recommended:* characters ÷ 4, labeled, then phase 4 calibration. The alternative is a bundled BPE tokenizer: more exact for OpenAI models, still wrong for Claude, and several MB.
2. **Default sort.** *Recommended:* tokens, largest first, in both sections.
3. **Budget bands.** *Recommended:* the `memorySizeBand` thresholds on the launch total (§5.3), so the pane and the reinjection card agree. The alternative is fixed token limits (e.g. 20K / 50K), which are simpler but ignore a 1M window.
4. **Phase 4.** *Recommended:* after phases 1–3, once the estimates have been compared with real launches.
