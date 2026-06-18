# Report: Context & Token Visibility — findings and next steps

**Date:** 2026-06-17
**Scope:** Why the agent-pane context meter is wrong, what token/context data AgentMux can actually get from the CLIs it drives, how to source model context windows reliably, and the recommended path forward.
**Design detail:** `docs/specs/SPEC_CONTEXT_VISIBILITY_2026_06_17.md` (this report is the executive summary + decisions).

---

## 1. The reported bug, diagnosed

The context meter in the agent pane's composer aux bar (`ContextWindowBar.tsx`) overflows (e.g. shows `300k / 200k`, pinned full) and never appears to compress. **Root cause: one wrong constant.**

- `frontend/app/view/agent/providers/index.ts` hardcodes the **Claude provider** `contextWindow: 200_000`. That's Haiku's window — **Opus and Sonnet are 1,000,000**. Running Opus, the meter divides real context by 200K instead of 1M.
  - *Overflow* = real ~300K Opus context ÷ 200K = 150%.
  - *"Never compresses"* = the CLI auto-compacts near Opus's *real* ceiling (~987K), so at 200–400K it correctly hasn't compacted — but the meter thinks 200K is the wall.
- The meter's **numerator is correct**: `contextTokens` is fed per turn from `message_start.usage` as `input_tokens + cache_creation + cache_read` (the true prompt size) and is **overwritten, not accumulated** (`useAgentStream.ts:689`, reducer `TokensIn`). So once the denominator is right, it will track compaction.

**It's both a wrong value and a wrong *shape*:** `contextWindow` is per-**provider**, but the `claude` provider spans Opus/Sonnet (1M) and Haiku (200K) — no single constant is correct.

---

## 2. What the wire actually gives us (and what it doesn't)

Verified against a live agent + the API reference:

| Want | Available? | Source |
|---|---|---|
| Tokens **per turn/query** | ✅ | `result.usage` (already captured) |
| Tokens **per assistant message** | ✅ | each `assistant` event's `usage` (full cache split) |
| **Current context size** | ✅ | `input_tokens + cache_read + cache_creation` of the latest request — **not** `input_tokens` alone (that's only the uncached remainder; undercounts by the cached portion) |
| Tokens **per tool call** | ❌ | a `tool_use` is one block in a message; usage is per-message only |
| Tokens **per reasoning block** | ❌ | thinking is billed inside `output_tokens`, never broken out |
| The **effective context window** | ❌ (not in stream) | CLI `system/init` and `result` carry `model` but **no** window field (all keys inspected) |

Per-tool / per-reasoning can only ever be a *local estimate* (tokenize the rendered text), clearly labelled — never an authoritative figure.

---

## 3. Context compaction is the CLI's, not ours

AgentMux drives the **Claude Code CLI** (not the Agent SDK, not the raw API). The **CLI auto-compacts** on its own: threshold ≈ `effective_window − 13K` (`effective = window − min(max_output, 20K)`), overridable via `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`, manual `/compact`. The API's `context_management`/compaction betas are a separate mechanism we don't use. So the meter should display fill **against the compaction threshold** ("about to compact"), not the raw window.

---

## 4. Sourcing context windows reliably (the hard part)

Sonnet is the problem child: **200K by default, 1M only with the `context-1m-2025-08-07` beta** — a runtime toggle, not a model property. So a static `model→window` map can't stay aligned. Findings on every candidate source:

- **`claude --help`** — ❌ no windows, no enumerable model list, no `models`/`config` subcommand (verified). `--betas` exists but is "API key users only."
- **CLI stream-json** — ❌ no window field anywhere (verified).
- **Anthropic Models API** (`GET /v1/models` → `max_input_tokens`) — ✅ **VERIFIED reachable with the agents' own OAuth (Claude Max) credentials: HTTP 200, no API key, no beta header needed, standard scopes suffice.** It's the live, authoritative catalog and auto-tracks new models. **Caveat:** it reports the model **ceiling** (Sonnet 4.6 → 1M), not the per-session *effective* window.

**Conclusion:** the Models API resolves single-window models definitively (Opus → 1M, Haiku → 200K) and gives the *upgrade ceiling* for Sonnet — but the effective Sonnet window still needs runtime resolution.

---

## 5. Recommended design (drift-proof, self-aligning)

A **learned window**, not a static constant — three layers that converge on the truth:

1. **Seed from the Models API** (cached) — kills hardcoded numbers, auto-tracks new models. Seed Sonnet conservatively (200K) and use its API ceiling (1M) as the upgrade target.
2. **Own the 1M toggle (cleanest)** — if AgentMux gates Sonnet's 1M (a per-agent setting → pass the beta/variant to the CLI), the window is deterministic from (model, setting).
3. **Self-align from observed behavior (bulletproof net)** — the prompt size can never exceed the real window, so (a) **high-water upgrade**: if context crosses the assumed window without compacting, promote to the next tier (catches Sonnet-1M); (b) **compaction calibration**: the pre-compaction high-water mark reveals the exact window — lock it for the session.

**Sync cadence — on CLI install/upgrade** (AgentMux owns these; infrequent; aligned with when supported models change). Sufficient because: a **bundled static fallback** covers first-run/offline/pre-auth, the **observed-behavior layer** self-corrects any seed regardless of catalog age, and an **auth-ready guard** skips the live fetch when no valid token. *Cadence only affects catalog freshness — it does not affect the Sonnet effective-window correctness, which is resolved per-session.*

Meter everything **against the compaction threshold**, never imply >100%, and verify the bar visibly drops after a real compaction.

---

## 6. Best next steps (prioritized)

| # | Step | Effort | Why now |
|---|---|---|---|
| **1** | **Quick win:** resolve `contextWindow` per-**model** (from the agent's `--model`), correct values (Opus/Sonnet 1M, Haiku 200K), and meter against the compaction threshold in `ContextWindowBar`. Fix the >100% presentation. | Small (3 files: `providers/index.ts`, `agent-view.tsx`, `ContextWindowBar.tsx`) | Directly fixes the reported "/200,000 on Opus" + overflow; verifiable live in the dev build today. |
| **2** | **Observed-behavior layer:** high-water upgrade + compaction-boundary calibration of the window (per pane). | Small–medium (reducer + a per-pane window signal) | The bulletproof net; makes the meter correct for Sonnet and any future model without a catalog. Highest reliability-per-effort. |
| **3** | **Models API catalog sync on CLI install:** fetch `/v1/models`, cache `max_input_tokens` per model, with a bundled static fallback + auth-ready guard. | Medium (install hook + cache + fallback; Rust side) | Removes hardcoded windows; "always in sync" as Anthropic adds models. Depends on step 1's per-model plumbing. |
| **4** | **Compaction visibility:** detect the auto-compaction (confirm the wire signal first) and drop a "context compacted" marker in the transcript. | Medium | Explains the context discontinuity to the user; needs a quick wire-signal verification first. |
| 5 | **Cache-hit indicator** + per-turn/message usage drill-down (extend `token-usage.ts` with cache fields). | Small–medium | Nice-to-have; cheap once per-message usage is captured. |
| — | Per-tool / per-reasoning token figures | — | **Not feasible authoritatively** (protocol limit). Only as labelled "≈" estimates, if ever. |

**Recommended sequence:** ship **1 + 2** together (small, fully fixes the reported bug and makes the meter trustworthy for every model including Sonnet), verify in the dev build, then do **3** (install-time catalog sync) to retire the last hardcoded numbers. **4** after a quick compaction-signal probe.

---

## 7. Pointers

- Design spec (full detail): `docs/specs/SPEC_CONTEXT_VISIBILITY_2026_06_17.md`
- Touch points: `ContextWindowBar.tsx`, `AgentComposerStrip.tsx`, `agent-view.tsx`, `providers/index.ts`, `useAgentStream.ts`, `claude-translator.ts`; per-pane state in `agent-pane-state/`.
- Agent OAuth creds (for the Models API call): `~/.agentmux/shared/providers/claude/.credentials.json` (token expires — fetch when fresh).
- Both this report and the spec are **uncommitted** — commit when ready.
