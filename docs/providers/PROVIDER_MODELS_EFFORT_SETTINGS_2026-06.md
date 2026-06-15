<!--
Copyright 2026, AgentMux Corp.
SPDX-License-Identifier: Apache-2.0
-->

# Provider Models, Effort & Settings — Reference (2026-06-14)

Reference for updating AgentMux's provider docs/config. Covers the latest models,
reasoning/effort levels, and key settings for **every provider AgentMux ships**
(`claude`, `codex`, `muxcode`, `gemini`, `qwen`, `kimi`, `openclaw`, `pi`, `copilot`).

- **Claude/Anthropic data is authoritative** (pulled from the bundled `claude-api`
  reference, cache 2026-06-04). **All other providers** are from web sources dated
  2026 — cited at the bottom; **verify before pinning**, model lines move fast.
- AgentMux's current values are quoted from
  `agentmux-srv/src/backend/providers.rs`, `frontend/app/view/agent/providers/index.ts`,
  `frontend/app/view/agent/types.ts`, and `buildRuntimeArgs.ts`.

---

## 0. TL;DR — what's stale in AgentMux today

| Gap | Current AgentMux | Should be | Where |
|---|---|---|---|
| **Effort missing `xhigh`** | `EffortLevel = "low"\|"medium"\|"high"\|"max"` | add **`xhigh`** (between `high` and `max`) | `frontend/app/view/agent/types.ts:209` |
| **Effort is "Claude-only"** | effort flag only built for `claude` | effort/reasoning is now **common** — Codex (`-c model_reasoning_effort`), Gemini (`thinking_level`) all have it | `buildRuntimeArgs.ts`, providers |
| **Codex default model stale** | `gpt-5.4` | `gpt-5.5` (current frontier; `gpt-5.1-codex-max`/`gpt-5.3-codex` for Codex-tuned) | `buildRuntimeArgs.ts:38` |
| **Claude effort default** | `"medium"` | Anthropic default is **`high`**; **`xhigh`** is the Claude-Code coding/agentic default | `types.ts:236` |
| **Effort errors on Haiku** | effort flag sent regardless of model | effort **400s on Haiku 4.5** — gate it off when model=`haiku` | provider arg build |
| **Per-provider model lists** | model picker is Claude-only (`opus/sonnet/haiku`) | each provider now has a distinct, fast-moving model line (below) | provider defs |

**The unifying insight** (the "slightly different but common" framing): every modern
coding agent now exposes the same three knobs — **(1) a model tier, (2) a
reasoning/effort level, (3) a permission/auto-approve mode.** Only the *names* and
*allowed values* differ. AgentMux's `AgentRuntimeConfig {permissionMode, model, effort}`
already models this; it just needs the value sets widened per provider (see §11).

---

## 1. Claude / Anthropic (`claude`) — AUTHORITATIVE

CLI: `claude` (`@anthropic-ai/claude-code`). AgentMux aliases `opus/sonnet/haiku`.

### Models (current)

| Tier | Model ID | Context | In/Out $ per 1M | Notes |
|---|---|---|---|---|
| Most capable | `claude-fable-5` | 1M | $10 / $50 | Most capable widely-released model; thinking always on; new tokenizer (~30% more tokens); not under ZDR |
| **Default (Opus)** | `claude-opus-4-8` | 1M | $5 / $25 | Anthropic's default — "use unless user names another"; state-of-the-art agentic |
| Opus prev | `claude-opus-4-7` | 1M | $5 / $25 | |
| Opus older | `claude-opus-4-6` | 1M | $5 / $25 | |
| Balanced (Sonnet) | `claude-sonnet-4-6` | 1M | $3 / $15 | Best speed/intelligence balance |
| Fast (Haiku) | `claude-haiku-4-5` | 200K | $1 / $5 | Fastest/cheapest; **effort param errors here** |

> AgentMux's `opus/sonnet/haiku` aliases resolve via the `claude` CLI. Map them to
> the IDs above in docs. Consider adding a **"fable"/"most-capable"** tier. Note
> AgentMux defaults to `sonnet` (cost) while Anthropic's API default is `opus-4-8`.

### Effort (reasoning depth)

`output_config: {effort: ...}` — **`low | medium | high | xhigh | max`**

- **Default `high`** (equivalent to omitting). `medium` is a balanced point; `low` for subagents/simple tasks.
- **`xhigh`** (added Opus 4.7; between `high` and `max`) — **best for most coding/agentic work; the Claude Code default.** Use ≥`high` for intelligence-sensitive work.
- **`max`** — Fable 5, Opus 4.6+, Sonnet 4.6 only (not Haiku/older Sonnets). Use when correctness > cost.
- Effort **works on** Fable 5, Opus 4.5/4.6/4.7/4.8, Sonnet 4.6. **Errors on Sonnet 4.5 / Haiku 4.5.**

### Thinking

- **Adaptive thinking** is the mode: `thinking: {type: "adaptive"}` (Claude decides depth; auto-interleaves between tool calls).
- **`budget_tokens` is gone** on Fable 5 / Opus 4.7 / 4.8 (400 error); deprecated on Opus 4.6 / Sonnet 4.6. Control depth with `effort` instead.
- Thinking **display defaults to `"omitted"`** on Fable 5 / Opus 4.7 / 4.8 (set `display: "summarized"` to surface reasoning).
- Fable 5: thinking always on — omit the `thinking` param (an explicit `{type:"disabled"}` 400s).

### Key settings / breaking changes (Opus 4.7 / 4.8 / Fable 5)

- **`temperature`, `top_p`, `top_k` are removed** — sending any → 400. Steer via prompting.
- **No last-assistant-turn prefill** → 400 (use structured outputs / system prompt).
- Stream when `max_tokens` > ~16K. Output ceilings: 128K (Fable/Opus), 64K (Sonnet), 64K (Haiku).
- Fable 5 only: requires 30-day data retention; safety classifiers can return `stop_reason:"refusal"` (HTTP 200).

---

## 2. OpenAI Codex (`codex`)

CLI: `codex` (`@openai/codex`, AgentMux pin `0.116.0`). AgentMux launch: `exec --json --dangerously-bypass-approvals-and-sandbox -`. Default model `gpt-5.4` (**stale**). Context 200K.

### Models (current, 2026)

| Model | Role |
|---|---|
| **`gpt-5.5`** | Newest frontier — more intelligent *and* more token-efficient than 5.4 |
| `gpt-5.4` | Prior frontier (AgentMux's current default) |
| `gpt-5.1-codex-max` | Codex-tuned, long-horizon agentic |
| `gpt-5.3-codex` | Codex-tuned |

### Reasoning effort

GPT-5.5 supports **`none | low | medium | high | xhigh`** (default **`medium`**).
- `low` efficient; `medium` balanced; `high` for complex agentic where latency matters less; **`xhigh`** for the hardest async agentic tasks/evals.
- Codex CLI passes effort as `-c model_reasoning_effort="..."` (or via TUI). **Note the shared `xhigh` value with Claude.**

### Settings
- Permission/sandbox baked into AgentMux base args (`--dangerously-bypass-approvals-and-sandbox`); no `--permission-mode`. Positional `-` must stay last.
- Session field: `thread_id`. Output: `codex-json`. `CODEX_HOME` must pre-exist.

---

## 3. Google Gemini (`gemini`)

CLI: `gemini` (`@google/gemini-cli`, AgentMux pin `0.32.1`). AgentMux launch: `--output-format stream-json --yolo -p`. Context **1M**.

### Models (current, 2026)

| Model | Released | Role |
|---|---|---|
| `gemini-3.1-pro` | 2026-02-19 | Latest Pro (preview in Gemini CLI) |
| `gemini-3.5-flash` | 2026-05-19 | Latest Flash |
| `gemini-3-flash` | — | Fast tier (in Gemini CLI) |
| `gemini-3-deep-think` | 2026-02-12 | Deep reasoning |

### Reasoning ("thinking_level")

`thinking_level: **minimal | low | medium | high**` (3.1 Pro added `MEDIUM`). Balances quality/latency/cost — the Gemini analog of Claude/Codex effort.

### Settings
- Permission: `--yolo` (bypass) vs none (default); no `--permission-mode`.
- `GEMINI_FORCE_FILE_STORAGE=true` set by AgentMux (skip macOS Keychain). Output: `gemini-json`. Resume `-r`.

---

## 4. Qwen Code (`qwen`) — Alibaba, OpenRouter-backed

CLI: `qwen` (`@qwen-code/qwen-code`, fork of Gemini CLI). AgentMux drives it OpenAI-compatible via `OPENAI_BASE_URL=https://openrouter.ai/api/v1` + `OPENAI_API_KEY` (+ optional `OPENAI_MODEL`). Same `--yolo`, `gemini-json` surface.

### Models (current, 2026)
- **`qwen3.6-max-preview`** (2026-04-20) — tops several coding benchmarks (SWE-bench Pro, Terminal-Bench 2.0).
- **`qwen3.6-plus`** (2026-03-30).
- `qwen3-coder` line — long-horizon autonomous coding.

Reasoning is model/endpoint-dependent (OpenAI-compatible `reasoning_effort` where the OpenRouter route supports it). Qwen OAuth free tier retired 2026-04-15 → API-key only.

---

## 5. Kimi Code (`kimi`) — Moonshot AI

CLI: `kimi` (**Python**, `pip install kimi-cli`). AgentMux launch: `--print --output-format stream-json --yolo -p`. Context 128K. Output `kimi-stream-json`.

### Models (current, 2026)
- **`kimi-k2.6`** (2026-04-20) — 1T params (32B active MoE), self-hostable (Modified MIT). Built for plan→write→test→debug loops lasting days; **agent-swarm**: native 300 parallel sub-agents / 4,000 coordinated steps.
- `kimi-k2.5` — prior.

Permission: `--yolo` only. Auth: API-key (`["info"]` / `["login"]`).

---

## 6. GitHub Copilot CLI (`copilot`) — Microsoft

CLI: `copilot` (`@github/copilot`). AgentMux runs **ACP** (`--acp`) — `-p`/stdin mode not yet supported. Context 128K. Output `acp`. Model selection is via Copilot's own config (backed by GPT-5.x / Claude / Gemini depending on the user's Copilot plan). No AgentMux-side model/effort flags.

---

## 7. OpenClaw (`openclaw`) — model-agnostic

CLI: `openclaw acp` (ACP bridge → local OpenClaw Gateway over WebSocket; **gateway daemon must be running**). Backing LLM chosen in OpenClaw config (defaults to Pi; can wire Claude/Codex/Gemini/local). No AgentMux-side model/effort.

---

## 8. Pi (`pi`) — Plandex / standalone

CLI: `pi --json` (ACP). Lightweight coding agent that also powers OpenClaw; read/write/bash/edit tools. Model chosen in Pi config (`["config","get","provider"]`). No AgentMux-side model/effort.

---

## 9. Mux Code (`muxcode`) — AgentMux first-party

CLI: `muxcode run -p` (`@a5af/muxcode`). Flexible backend: local GGUF, Anthropic, OpenAI, or any OpenAI-compatible endpoint (selected by `OPENAI_BASE_URL`/`OPENAI_API_KEY` etc.). Claude-compatible NDJSON (`claude-stream-json`), context 200K. Models/effort follow whichever backend is wired.

---

## 10. Cross-provider comparison — the "common but different" knobs

| Provider | Reasoning/effort knob | Values | Default | Permission/auto |
|---|---|---|---|---|
| **Claude** | `effort` | low, medium, high, **xhigh**, max | high | 5 modes (`--permission-mode`, `--dangerously-skip-permissions`) |
| **Codex** | `model_reasoning_effort` | none, low, medium, high, **xhigh** | medium | baked bypass (no mode flag) |
| **Gemini** | `thinking_level` | minimal, low, medium, high | (model) | `--yolo` / none |
| **Qwen** | `reasoning_effort` (OpenAI-compat) | route-dependent | — | `--yolo` / none |
| **Kimi** | (none surfaced via CLI) | — | — | `--yolo` |
| **Copilot / OpenClaw / Pi / Mux** | provider-internal | — | — | ACP / config |

**Observation:** Claude, Codex, and Gemini have converged on a 4–5 step reasoning
scale; **`xhigh` is now shared by Claude (Opus 4.7/4.8) and OpenAI (GPT-5.5)** as the
"hardest agentic" tier. AgentMux's effort enum should add `xhigh` and the docs should
present effort as a cross-provider concept (mapped per provider), not a Claude-only one.

---

## 11. Concrete AgentMux changes implied (for the doc/config update)

1. **`frontend/app/view/agent/types.ts:209`** — `EffortLevel`: add `"xhigh"` →
   `"low" | "medium" | "high" | "xhigh" | "max"`. Update the `/effort` slash-command
   choices and `buildRuntimeArgs.ts` accordingly.
2. **Claude effort default** — docs should state Anthropic's default is `high` and the
   coding sweet spot is `xhigh`; decide whether AgentMux's `"medium"` default
   (`types.ts:236`) should move to `high`.
3. **Gate effort by model** — don't emit `--effort` for `haiku` (errors on Haiku 4.5).
4. **`buildRuntimeArgs.ts:38`** — bump Codex default `gpt-5.4` → `gpt-5.5`; surface
   Codex `model_reasoning_effort` (it shares the `low/medium/high/xhigh` scale).
5. **Generalize effort beyond Claude** — wire Codex (`-c model_reasoning_effort`) and
   Gemini (`thinking_level`) through the same `AgentRuntimeConfig.effort` with a
   per-provider value map.
6. **New canonical doc** — there is no central provider/model reference today
   (info is scattered in `providers.rs`, `index.ts`, `provider-auth-isolation.md`).
   This file is the seed; promote a maintained `docs/providers/` set.
7. **Model lists** — capture each provider's current line (above) so the picker can
   show real model choices, not Claude-only.

---

## Sources

Claude/Anthropic: bundled `claude-api` skill reference (cache 2026-06-04) — authoritative.

Non-Claude (web, 2026 — verify before pinning):
- [Introducing GPT-5.5 | OpenAI](https://openai.com/index/introducing-gpt-5-5/)
- [Using GPT-5.5 | OpenAI API](https://developers.openai.com/api/docs/guides/latest-model)
- [Building more with GPT-5.1-Codex-Max | OpenAI](https://openai.com/index/gpt-5-1-codex-max/)
- [Codex Changelog | OpenAI Developers](https://developers.openai.com/codex/changelog)
- [Gemini 3.1 Pro | Google blog](https://blog.google/innovation-and-ai/models-and-research/gemini-models/gemini-3-1-pro/)
- [Gemini 3 Flash | Google blog](https://blog.google/products/gemini/gemini-3-flash/)
- [Gemini 3 Flash | Google Cloud docs (thinking_level)](https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/gemini/3-flash)
- [Moonshot AI releases Kimi K2.6 | MarkTechPost](https://www.marktechpost.com/2026/04/20/moonshot-ai-releases-kimi-k2-6-with-long-horizon-coding-agent-swarm-scaling-to-300-sub-agents-and-4000-coordinated-steps/)
- [Kimi K2.6 vs Qwen3.6 Max vs DeepSeek V4 | DeepLearning.AI The Batch](https://www.deeplearning.ai/the-batch/kimi-k2-6-matches-open-qwen3-6-max-anddeepseek-v4-falls-just-behind-top-closed-models)
- [Qwen3.6-Max-Preview vs Plus vs Kimi K2.6 | Lushbinary](https://lushbinary.com/blog/qwen-3-6-max-preview-vs-plus-vs-kimi-k2-6-comparison/)
