# SPEC: Context & Token Visibility — what we can show the user, and how

**Date:** 2026-06-17
**Status:** Draft — research complete, design proposed
**Owner:** Agent pane (token-usage store, translators) + status bar
**Builds on:** `SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md` (the existing session-wide tally)
**Scope:** How AgentMux learns token usage and context state from the agent CLIs it drives, what granularity is actually available, how context compaction works, and a design to surface it to the user.

---

## 0. The framing correction (important)

AgentMux does **not** call the Anthropic Messages API directly, and it does **not** use the Claude **Agent SDK** (`@anthropic-ai/claude-agent-sdk`). It **spawns the Claude Code CLI** (a native binary, currently v2.1.x) in stream-json mode (`--input-format stream-json --output-format stream-json --verbose --include-partial-messages`) and reads the NDJSON it emits. So:

- **Token usage** is whatever the CLI puts in its stream-json `usage` blocks — not something we query.
- **Context compaction** is governed by the **CLI's own auto-compaction**, not by the API's `context_management` beta. We are a *consumer* of the CLI's context behavior, not its driver.

Both points shape everything below. (The same applies to the other providers — Codex, Gemini, Kimi — each via its own translator; this spec details Claude and notes where the others differ.)

---

## 1. What token data the stream actually carries

### 1.1 Per **assistant message** `usage` (captured live, bundled CLI v2.1.178)

Every `assistant` event carries a `usage` object. Real bytes captured from a running dev agent:

```jsonc
"usage": {
  "input_tokens": 1,                       // NEW uncached prompt tokens this request
  "cache_creation_input_tokens": 27868,    // tokens written to cache (~1.25× price)
  "cache_read_input_tokens": 15621,        // tokens served from cache (~0.1× price)
  "cache_creation": { "ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 27868 },
  "output_tokens": 8,
  "service_tier": "standard",
  "inference_geo": "not_available"
}
```

Also on the assistant event: `diagnostics.cache_miss_reason` (why the prefix cache missed) and **`context_management`** (null in the capture; populated when API-side context edits/compaction apply — see §3).

**The current prompt (context) size for that request = `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`.** `input_tokens` alone is only the *uncached remainder* — reading it as "context size" undercounts by the cached portion (here, 1 vs ~43.5K). This is the single most important correctness point for any context gauge.

### 1.2 Per **turn** — the `result` event

At the end of each query/turn the CLI emits:

```jsonc
{ "type": "result", "subtype": "success", "is_error": false,
  "num_turns": 12, "duration_ms": …, "duration_api_ms": …, "total_cost_usd": …,
  "usage": { "input_tokens", "output_tokens", "cache_creation_input_tokens",
             "cache_read_input_tokens", "service_tier", "server_tool_use" },
  "modelUsage": { … per-model breakdown … } }
```

This is cumulative for the turn. **AgentMux already reads this** (`claude-translator.ts:61-76`): it sums input+cache_creation+cache_read → `input_tokens`, takes `output_tokens`, and `num_turns`, then `useAgentStream.ts:415` feeds it to `recordTurn()`.

### 1.3 Granularity ceiling — what we **cannot** get

| Question | Answer | Why |
|---|---|---|
| Tokens **per query/turn** | ✅ Yes | `result.usage` (and we already capture it) |
| Tokens **per assistant message** (model request) | ✅ Yes | each `assistant` event's `usage` |
| Tokens **per tool call** | ❌ **No** | a `tool_use` is one content block inside an assistant message; `usage` is reported for the *whole message*, never per block. The *cost of a tool's result* shows up as `input_tokens` in the **next** assistant message, not attributed to the tool. |
| Tokens **per line/block of reasoning** (thinking) | ❌ **No** | thinking is billed inside `output_tokens` but never broken out. With `display:"omitted"` (our current default) the thinking text isn't even emitted. |

**Best we can do for per-tool / per-reasoning is *estimation*, not truth:** locally tokenize the rendered text of a block (e.g. via `count_tokens`, or a cheap heuristic) and label it "≈". The wire protocol gives no authoritative per-block number. Any UI must mark these as approximate or omit them. Do **not** invent a per-tool token figure that looks authoritative.

---

## 2. The one real lever for finer attribution

Because each **assistant message** has its own `usage`, we can attribute at the message level (which the current per-turn-only capture throws away):

- **output side:** each assistant message's `output_tokens` is exactly the tokens that message produced (text + thinking + any tool_use args). Attribute it to that message/step.
- **input side:** the jump in `input_tokens + cache_read + cache_creation` between consecutive assistant messages ≈ what the intervening tool_result(s) + user turn added to context. This lets us say "that big file read cost ~X tokens of context" — at message granularity, which is the honest unit.

This is the highest-value upgrade: move from a single session counter to **per-message context accounting**, then roll up to per-turn and per-agent.

---

## 3. How context compresses

### 3.1 The CLI's auto-compaction (what actually governs our agents)

The Claude Code CLI auto-compacts on its own:

- **Trigger:** when the conversation approaches the context window. Default threshold ≈ **`effective_window − 13,000` tokens**, where `effective_window = model_context_window − min(max_output_tokens, 20,000)`. (Buffer was ~33K/16.5% in earlier builds; it's been tightened.) Overridable via the **`CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`** env var (e.g. `60` = compact at 60% capacity); community guidance is to compact early (~60%) for quality. Manual: the `/compact` command.
- **Effect:** the CLI summarizes earlier history, preserving decisions/state, and continues with a smaller transcript.
- **Our visibility:** there is **no documented dedicated "compacted" event** in stream-json. Detection options: (a) watch for a large *drop* in `input_tokens+cache_*` across turns (the post-compaction prompt is much smaller); (b) the CLI may emit a `system` line around it; (c) the `context_management` field on the assistant event (§1.1) is the API-side signal. We should empirically confirm which signal the bundled CLI emits before relying on one — same "verify the wire, don't trust the doc" discipline as `SPEC_AGENT_CONTROL_PROTOCOL`.
- **Knob we control:** AgentMux launches the CLI, so we can set `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` per agent (a user-facing "compact at N%" setting) — see §5.

### 3.2 API-level context management (NOT what we use, but worth knowing)

The Messages API has its own context features (beta): **compaction** (`compact-2026-01-12`, `context_management:{edits:[{type:"compact_20260112"}]}` → returns a `compaction` block to echo back), and **context editing** (`clear_tool_uses_*`, `clear_thinking_*` — prune stale blocks). These are for code that calls the API directly. AgentMux does not; the CLI may use them internally and surface state via the `context_management` field. We don't implement them ourselves.

### 3.3 Other providers

Codex / Gemini / Kimi each compact differently and report usage in their own shapes (`codex-translator.ts`, `gemini-translator.ts`). The context gauge must be per-provider: read the model's window + the provider's usage fields. Where a provider doesn't report cache tokens, the gauge degrades to input+output only.

---

## 4. What AgentMux shows today (the baseline)

Two separate surfaces exist:

- **Session spend counter** — `frontend/app/store/token-usage.ts`: a session-wide `{input, output}` tally **per provider** (no cache fields, no per-agent split, no context %). `recordTurn()` adds each turn's totals from the `result` event; the status bar (`StatusBar.tsx`) shows `↑Xk ↓Yk`, popover shows per-provider breakdown.
- **Per-agent context meter** — `frontend/app/view/agent/components/ContextWindowBar.tsx`, rendered in the composer strip's aux bar above the text input (`AgentComposerStrip.tsx:265`). It shows `<tokens> / <contextWindow>` as a banded bar. `tokens` is fed live per turn from `message_start.usage` (`useAgentStream.ts:689-703`) as `input_tokens + cache_creation_input_tokens + cache_read_input_tokens` (the real prompt size), dispatched as `TokensIn` and **overwritten, not accumulated** (`reducer.ts:481`). `contextWindow` comes from `provider().contextWindow` (`agent-view.tsx:1207`).

The meter's *source* is correct (current-context, overwrite — it would drop on compaction). The bugs are in the **denominator** and the **presentation** — §4.1.

### 4.1 Confirmed bugs in the existing context meter

**Bug A — wrong, per-PROVIDER context window (root cause of both reported symptoms).**
`providers/index.ts` gives the Claude provider a single `contextWindow: 200_000`. But the window is **per-model**: Opus 4.x and Sonnet 4.x are **1,000,000**; only Haiku is 200K. So a Claude agent running Opus is metered against 200K when its real window is 1M. Consequences exactly match the report:
- **"Overflowing":** a real ~300K Opus context ÷ 200K = 150% → the bar pins at 100% (fill is `Math.min(100, …)`, `ContextWindowBar.tsx:35`) and the numeric label reads `300k / 200k`.
- **"Not compressing":** the CLI auto-compacts near Opus's *real* threshold (~`1M − 13K` ≈ 987K, or the `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` value), so at 200–400K it legitimately hasn't compacted — but the meter, believing 200K is the ceiling, shows it blowing past with no relief. **Fixing the window largely fixes the "no compaction" appearance**, because the meter overwrites per turn and *will* drop once the CLI actually compacts near the true ceiling.

The deeper issue is that `contextWindow` is a **per-provider constant**, but `claude` spans models with different windows — and worse, **the window isn't even fixed per model**: Sonnet 4.x is **200K by default but 1M with the `context-1m-2025-08-07` beta enabled**. So a static `model→window` map is *also* unreliable (it'd be wrong for Sonnet half the time).

**Verified (this is what makes it hard):** the CLI's stream-json does **not** report the effective window anywhere — `system/init` and `result` both carry `model` but no `context_window`/`max_input_tokens`/beta field (all keys inspected on a live agent). So we cannot just read it. The window must be **resolved from the agent's model AND derived/learned at runtime** (§5 P1).

**Bug B — overflow presentation.** Even with a correct window, the label prints raw `tokens / window`, so any genuine overflow (or a model whose window we don't know) reads as ">100%". The bar should (a) never imply >100% numerically, and (b) mark the **compaction threshold** (not just 100%), so "full" means "about to compact," which is the number the user actually cares about.

---

## 5. Proposed design — show context, not just spend

Five additions, in priority order. Each is independently shippable.

**P1 — Fix the existing context meter (the headline; addresses the reported bug).**
The `ContextWindowBar` already exists and is already fed the right *numerator* (current-context, per-turn, overwrite). The work is:
1. **Make the window a LEARNED value, not a static constant** (Bug A). The CLI doesn't expose it and Sonnet's window is a runtime beta toggle, so a static table can't stay aligned. Use a three-layer resolution that converges on the truth:
   - **(seed / catalog — "always in sync" source)** Do **not** hardcode windows. Verified: `claude --help` does *not* expose context windows or an enumerable model list (`--model` only documents the alias/full-name shape; there's no `models`/`config` subcommand; no `context`/`window`/`1m` anywhere in help). The authoritative live catalog is the **Anthropic Models API** — `GET /v1/models` / `GET /v1/models/{id}` → **`max_input_tokens`** (the window) + a `capabilities` tree — which tracks new models and their windows automatically.

**VERIFIED reachable with the agents' own credentials (2026-06-17):** `GET /v1/models` returns **HTTP 200** using the agent's OAuth (Claude Max subscription) Bearer token — and **does not require the `oauth-2025-04-20` beta header** (a control request without it also 200'd). The standard agent scopes (`user:inference`, `user:profile`, …) suffice. No API key needed. Live values: Opus 4.x / Fable 5 / Sonnet 4.6 = 1,000,000; Opus 4.5 / Haiku 4.5 = 200,000.

**Two caveats from the test:**
- **It reports the model's *ceiling*, not the effective window.** `claude-sonnet-4-6` returns `max_input_tokens: 1,000,000` (its max capability) — *not* the 200K a session without the 1M beta actually gets. So the API definitively resolves single-window models (Opus → 1M, Haiku → 200K) but the Sonnet 200K-vs-1M effective value still needs layer 2 (own the toggle) or layer 3 (observed behavior).
- **OAuth tokens expire** (the long-idle agent's token was 11 days expired → 401; a recently-used one is valid). So don't fetch on demand against a possibly-stale token — **cache the catalog.**

**Sync cadence — on CLI install/update (chosen).** AgentMux already manages CLI install/upgrade (`npm install @anthropic-ai/claude-code@latest`, `claude update`), and a CLI version bump is exactly when the supported-model set tends to change — so refresh the cached catalog **as a post-install/upgrade step** (per provider). This is the right primary trigger: AgentMux-controlled, infrequent, semantically aligned, no polling. Three things make it sufficient:
  1. **Bundled static fallback** baked into the build — covers first run, offline, and *install-before-auth* (a fresh install may run before the user logs in, so the live `GET /v1/models` 401s; keep the fallback and retry the sync on the next install/auth).
  2. **Observed-behavior layer is always on** (§5 P1 layer 3) — it self-corrects any seed regardless of catalog age, so a stale catalog is low-stakes: a brand-new model released between CLI updates just gets a best-guess seed that self-heals on first overflow/compaction.
  3. **Auth-ready guard** at install — only do the live fetch if a valid token is present; otherwise keep the last-good cache / bundled fallback.
  - *Optional belt-and-suspenders:* a lazy staleness check (cache older than ~30d **and** a fresh token available → refetch) catches new models released *without* a CLI update, without any polling. Not required given (2); add only if cheap.
  - **Cadence ≠ the Sonnet fix.** Sync frequency only governs catalog freshness (new models / changed ceilings). The Sonnet 200K-vs-1M *effective* window is unaffected by sync cadence — it's resolved per-session by seed-conservative + layer 2/3, every session, regardless of when the catalog was last synced.

Net: seed single-window models directly from the Models-API `max_input_tokens` (no hardcoding, auto-tracks new models); for Sonnet, seed conservative (200K) and use the API's 1M as the layer-3 upgrade target.
   - **(own the toggle — most reliable)** If AgentMux gates Sonnet's 1M context (a per-agent "1M context" setting that makes AgentMux pass the `context-1m-2025-08-07` beta / `[1m]` model variant to the CLI), then the window is a **deterministic function of (model, that setting)** — AgentMux is the single source of truth. This is the clean fix; the layers below are the safety net for anything set outside AgentMux or by a future CLI change.
   - **(self-align from observed behavior — the bulletproof net)** Two invariants the stream *does* give us, regardless of model/beta:
     - **High-water upgrade:** the per-turn prompt size (`contextTokens`) can never exceed the real window. If it ever exceeds the current assumed window *without* a compaction, the assumption is provably too low → promote to the next known tier (200K → 1M). This catches Sonnet-1M the moment context passes 200K.
     - **Compaction calibration:** when a compaction is detected (sharp `contextTokens` drop, §3.1), the pre-compaction high-water mark ≈ `effectiveWindow − buffer` → back-calculate and **lock the exact window for the session.**

   Window is learned-up-only within a session (model/beta are fixed per session; a new session re-seeds). Net effect: the meter converges to the true window for any model/beta combo without depending on a field the CLI never emits.
2. **Meter against the compaction threshold, not the raw window** (Bug B). Compute `threshold ≈ effectiveWindow − 13K` (`effectiveWindow = window − min(maxOutput, 20K)`), honoring `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` if set. Mark it on the bar so "full" = "about to compact." Never render a numeric ratio implying >100%; clamp the label and show an explicit "over / compacting" state instead of a pinned 100% bar.
3. Confirm the meter visibly **drops after a real compaction** (it should, given the overwrite source + correct window — verify once Bug A is fixed).

This is the highest-value, smallest change and directly fixes "shows / 200,000 on Opus" and "overflows without compressing."

**P2 — Compaction awareness.**
Detect a compaction (per §3.1, after we confirm the signal) and (a) drop the gauge accordingly, (b) drop a "context compacted — earlier history summarized" marker into the transcript so the user understands the context discontinuity. Optionally expose the **`CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`** as a per-agent "compact at N%" setting in the agent settings panel.

**P3 — Cache-hit visibility.**
Surface `cache_read / (input+cache_read+cache_creation)` as a small "cache: 92%" indicator. High cache-read = cheap, fast; a sudden drop signals a cache-busting change (model switch, edited system prompt). Cheap to add once per-message usage is captured.

**P4 — Per-turn / per-message breakdown.**
In a pane-level "usage" popover: a list of turns, each with input/output/cache + `total_cost_usd` (from `result`), expandable to per-message rows. This is the honest drill-down — turn and message granularity only.

**P5 — Approximate per-tool / per-reasoning (clearly labelled).**
Only if there's appetite: locally estimate tokens for big tool outputs and thinking blocks and show "≈X tok" on the tool/thinking block. Must be visibly approximate. Skippable — it's the lowest-confidence data and the most likely to mislead.

### Files to touch

| File | Change |
|---|---|
| `frontend/app/view/agent/providers/index.ts` | **P1:** replace per-provider `contextWindow: 200_000` (Claude) with a model→window map; resolve by the agent's model. Optional `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` wiring. |
| `frontend/app/view/agent/agent-view.tsx` | **P1:** pass `contextWindow` resolved from the agent's **model** (`block.meta["agent:model"]`), not `provider().contextWindow`; pass the compaction threshold. |
| `frontend/app/view/agent/components/ContextWindowBar.tsx` | **P1:** meter against the compaction threshold; never imply >100%; mark the threshold; explicit over/compacting state (Bug B). |
| `frontend/app/view/agent/components/AgentComposerStrip.tsx` | **P1/P3:** thread the threshold + cache-hit indicator through to the bar. |
| `frontend/app/view/agent/providers/claude-translator.ts` | **P3/P4:** preserve the cache split + `context_management` from per-message usage (currently the `result` path drops the cache split). |
| `frontend/app/store/token-usage.ts` | **P4:** extend `ServiceUsage` with cache fields for the spend popover. |
| transcript reducer / `stream-parser.ts` | **P2:** compaction marker node. |
| `codex/gemini-translator.ts` + provider model maps | per-provider/per-model window + usage parity. |

---

## 6. Open questions / things to verify on the wire first

1. **Compaction signal:** what exactly does the bundled CLI emit when it auto-compacts? (new `system`/`init`? a visible message? only an `input_tokens` drop? a populated `context_management`?) — capture it before building P2.
2. **`context_management` field semantics** on the assistant event from the CLI (vs the API beta) — when is it non-null?
3. **`modelUsage`** shape on `result` — useful for multi-model turns (subagents on a cheaper model).
4. **Effective-window constants** per model/provider — confirm `min(max_output, 20000)` and the 13K buffer against the current CLI, and whether `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` shifts it.
5. **Per-tool estimation** — is an approximate figure worth the risk of looking authoritative? Default: omit unless asked.

---

## 7. Sources

- Claude API / Agent SDK reference (bundled `claude-api` skill): `usage` schema, `context_management`/compaction/context-editing, token-counting, per-message vs per-turn granularity.
- Claude Code auto-compaction threshold + `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`: anthropics/claude-code issues #41037, #41818; claudefast.st "Context Buffer" guide; justin3go / danielvaughan compaction deep-dives.
- stream-json event/usage surface: takopi stream-json cheatsheet; backgroundclaude.com "stream-json"; code.claude.com Agent SDK streaming-output docs.
- Empirical: usage/result bytes captured live from the running dev agent (`agentmuxsrv-v0.46.0` log) and AgentMux source (`token-usage.ts`, `claude-translator.ts`, `useAgentStream.ts`).
