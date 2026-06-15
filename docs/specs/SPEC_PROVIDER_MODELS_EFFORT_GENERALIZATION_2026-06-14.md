<!--
Copyright 2026, AgentMux Corp.
SPDX-License-Identifier: Apache-2.0
-->

# SPEC: Per-Provider Models + Generalized Reasoning/Effort

- **Date:** 2026-06-14
- **Status:** Draft / proposed
- **Area:** `frontend/app/view/agent` (runtime config, `buildRuntimeArgs`, control bar, `/model` + `/effort` slash commands, provider defs), `agentmux-srv/src/backend/providers.rs`
- **Source:** `docs/providers/PROVIDER_MODELS_EFFORT_SETTINGS_2026-06.md` (provider report)
- **Related:** PR #1411 (landed the `xhigh` effort level + Claude default → Opus/xhigh), provider report §0/§11

---

## 0. One-line

The agent runtime config models a single Claude-only `(model, effort)`, but **every
provider AgentMux ships now has its own model line and its own reasoning/effort
knob**. Generalize `model` and `effort` to be **per-provider** — provider-supplied
choice lists + a per-provider mapping to CLI flags — and refresh the stale values
(Codex `gpt-5.4` → `gpt-5.5`, gate Claude `--effort` off for Haiku).

### Already landed (PR #1411) — do not redo

- `EffortLevel` gained **`xhigh`** (between `high` and `max`); surfaced in `/effort`, the control-bar dropdown, and `EFFORT_LABELS`.
- `DEFAULT_RUNTIME_CONFIG`: Claude default → `model: "opus"` (resolves to Opus 4.8 via the CLI), `effort: "xhigh"`.

This spec covers the **remaining** gaps from the report (§11 items 1, 3, 5, 7).

---

## 1. Background — current behavior (code-grounded)

- `AgentRuntimeConfig` (`frontend/app/view/agent/types.ts:528`) is global:
  `{ permissionMode: PermissionMode, model: ModelChoice, effort: EffortLevel }`.
- `ModelChoice = "opus" | "sonnet" | "haiku"` (`types.ts:525`) — **Claude names only**.
- `EffortLevel = "low" | "medium" | "high" | "xhigh" | "max"` (`types.ts:526`).
- `buildRuntimeArgs.ts` emits flags **Claude-only**:
  - `--model {model}` only when `!providerId || providerId === "claude"` (`buildRuntimeArgs.ts:100-103`).
  - `--effort {effort}` only when `!providerId || providerId === "claude"` (`:104-106`).
  - Codex hardcodes `--model CODEX_DEFAULT_MODEL` = `"gpt-5.4"` (`:38`, `:110-114`), placed before the trailing `-` prompt positional. **No Codex reasoning flag.**
  - Gemini/Qwen/Kimi: no `--model`, no reasoning flag.
- The control-bar **model dropdown** (`AgentControlBar.tsx:~290`) and `/model` choices
  (`commands/global/runtime.ts`) are **hardcoded to opus/sonnet/haiku** regardless of
  the pane's provider — so for a Codex/Gemini pane the dropdown shows Claude models
  that are ignored.

### Problems

1. **Model picker is wrong for non-Claude panes** — shows Claude models; the selection is dropped (Codex falls back to a hardcoded default; Gemini/Kimi/Qwen use their CLI default).
2. **Reasoning is Claude-only** — but Codex (`model_reasoning_effort`) and Gemini (`thinking_level`) have the same kind of knob (report §10). Users can't tune them.
3. **Stale values** — Codex default `gpt-5.4` (current is `gpt-5.5`); each provider's model line has moved (report §1–§9).
4. **Effort errors on Haiku** — Claude `--effort` 400s on Haiku 4.5; we emit it unconditionally.

---

## 2. Goals / Non-goals

**Goals**
1. `model` and `effort` become **per-provider**: each provider supplies a model choice list and a reasoning-knob descriptor; the picker + arg builder read from the active provider.
2. Refresh model lists + defaults to current values (report §1–§9); bump Codex default to `gpt-5.5`.
3. Emit each provider's reasoning flag correctly (Claude `--effort`, Codex `-c model_reasoning_effort=…`, Gemini `thinking_level`), with the canonical effort scale mapped per provider.
4. Gate effort by model (skip Claude `--effort` on `haiku`).
5. Keep the abstraction in `ProviderDefinition`/`ProviderConfig` so adding a provider = adding data, not branches.

**Non-goals**
- Per-model pricing/context UI (the report is the reference; not surfaced in-app here).
- OpenClaw / Pi / Copilot / Mux model selection (model is chosen in their own config / ACP; AgentMux passes nothing — leave as today).
- Migrating stored `agent:runtime` blobs beyond a forward-compatible default (see §3.5).

---

## 3. Design

### 3.1 Per-provider model lists

Add to `ProviderDefinition` (TS, `providers/index.ts:35`) and `ProviderConfig`
(Rust, `providers.rs:34`):

```ts
interface ProviderModel { value: string; label: string; default?: boolean }
// new field on ProviderDefinition:
models?: ProviderModel[];          // omitted/empty ⇒ provider has no AgentMux-side model picker
modelFlag?: string | null;         // e.g. "--model"; null ⇒ don't emit (model chosen in provider config)
```

- `AgentRuntimeConfig.model` becomes a **free `string`** (provider-scoped value), not the
  Claude `ModelChoice` enum. Keep `ModelChoice` as a Claude-local alias type for clarity,
  but `runtime.model` is just `string`.
- The control-bar dropdown and `/model` choices render `provider.models` for the active
  provider; if `models` is empty, hide the model control.
- `buildRuntimeArgs`: emit `provider.modelFlag` + `runtime.model` when both are present
  (replaces the `claude`-only branch and the Codex special-case). The **Codex positional
  `-` rule still applies** — append model flag before the trailing `-`.

### 3.2 Generalized reasoning / effort

Add a reasoning descriptor to each provider:

```ts
interface ReasoningSpec {
  // canonical AgentMux effort values this provider accepts, in display order
  levels: EffortLevel[];                 // e.g. claude: [low,medium,high,xhigh,max]
  // how to emit: a flag template; {value} substituted with the mapped provider value
  emit: { kind: "flag"; flag: string }   // claude: "--effort {value}"
       | { kind: "config"; key: string } // codex: -c model_reasoning_effort="{value}"
       | { kind: "flag-eq"; flag: string }; // gemini: "--thinking-level={value}" (verify)
  // map canonical EffortLevel → provider's wire value (identity unless noted)
  map?: Partial<Record<EffortLevel, string>>;
  default?: EffortLevel;
}
reasoning?: ReasoningSpec;               // omitted ⇒ no reasoning control for this provider
```

- The `/effort` command + control-bar effort dropdown render `provider.reasoning.levels`.
- `buildRuntimeArgs` emits the reasoning arg via `reasoning.emit` with the mapped value;
  for `config` kind, emit `-c <key>="<value>"` **before** Codex's trailing `-`.
- Canonical scale stays `low | medium | high | xhigh | max`. Providers that lack a level
  map it (Gemini has no `xhigh`/`max` → map both to `high`; Gemini has `minimal` → expose
  as an extra level only on Gemini, or map our `low` → `minimal`/`low` per §6 decision).

### 3.3 Model-gated effort

`reasoning` emission is also gated by model: a provider may declare models for which
effort must not be sent.

```ts
// on ProviderModel: noReasoning?: true
```

Claude `haiku` → `noReasoning: true` (effort 400s on Haiku 4.5). `buildRuntimeArgs` skips
the reasoning arg when the active model has `noReasoning`. (Also covers `max` being
invalid on non-Opus — optionally clamp `max`→`xhigh` for `sonnet`; see §8.)

### 3.4 Refreshed values (report §1–§9)

Encode current model lines as `provider.models` (verify each against the CLI before
pinning — model strings move):

- **claude**: `opus` (default), `sonnet`, `haiku` (+ optional `fable`/most-capable). Aliases → current IDs (Opus 4.8 / Sonnet 4.6 / Haiku 4.5).
- **codex**: `gpt-5.5` (default), `gpt-5.4`, `gpt-5.1-codex-max`, `gpt-5.3-codex`. **Bump `CODEX_DEFAULT_MODEL` → `gpt-5.5`** (`buildRuntimeArgs.ts:38`) until the picker lands; then drive from `models`.
- **gemini**: `gemini-3.1-pro`, `gemini-3.5-flash`, `gemini-3-flash`, `gemini-3-deep-think`.
- **qwen**: `qwen3.6-max-preview`, `qwen3.6-plus`, `qwen3-coder` (OpenRouter ids via `OPENAI_MODEL`).
- **kimi**: `kimi-k2.6`, `kimi-k2.5`.
- **muxcode / openclaw / pi / copilot**: no AgentMux-side model list (`models` omitted).

### 3.5 Migration / back-compat

- `getRuntimeConfig` (`buildRuntimeArgs.ts:122`) already falls back to `DEFAULT_RUNTIME_CONFIG`.
- Stored `agent:runtime.model` values (`"opus"|"sonnet"|"haiku"`) remain valid for Claude.
- When the active provider's `models` doesn't contain the stored `model`, fall back to that
  provider's default model (don't pass a foreign model string to a CLI). Same for `effort`
  not in `reasoning.levels`.

---

## 4. Per-provider matrix (target)

| Provider | Model list (default ⋆) | Reasoning knob | Emit | Notes |
|---|---|---|---|---|
| claude | opus⋆, sonnet, haiku | low,medium,high,xhigh,max | `--effort {v}` | gate off on haiku; `max` Opus-only |
| codex | **gpt-5.5⋆**, gpt-5.4, gpt-5.1-codex-max, gpt-5.3-codex | none,low,medium,high,xhigh | `-c model_reasoning_effort="{v}"` | before trailing `-` |
| gemini | gemini-3.1-pro, gemini-3.5-flash, gemini-3-flash, gemini-3-deep-think | minimal,low,medium,high | `--thinking-level={v}` *(verify)* | map xhigh/max→high |
| qwen | qwen3.6-max-preview, qwen3.6-plus, qwen3-coder | (route-dependent) | OpenAI `reasoning_effort` via env/`-c` | OpenRouter-backed |
| kimi | kimi-k2.6, kimi-k2.5 | — | — | no CLI reasoning flag surfaced |
| muxcode/openclaw/pi/copilot | — (provider config) | — | — | model chosen in their config / ACP |

> Exact CLI flags (`-c model_reasoning_effort`, gemini `--thinking-level`) must be
> **verified against each CLI's `--help`** during implementation — flags move between
> CLI versions. The pinned CLI versions are in the provider defs (`codex 0.116.0`,
> `gemini-cli 0.32.1`).

---

## 5. Touch points

| File | Change |
|---|---|
| `frontend/app/view/agent/types.ts:525-526` | `model: string` on `AgentRuntimeConfig`; keep `EffortLevel` as the canonical scale |
| `frontend/app/view/agent/providers/index.ts:35-88` | add `models`, `modelFlag`, `reasoning` to `ProviderDefinition`; populate per provider (§4) |
| `agentmux-srv/src/backend/providers.rs:34-83` | mirror the new fields on `ProviderConfig` (kept in parity with TS) |
| `frontend/app/view/agent/buildRuntimeArgs.ts:96-114` | replace Claude-only `--model`/`--effort` + Codex special-case with provider-driven `modelFlag`/`reasoning.emit`; honor `noReasoning`; bump `CODEX_DEFAULT_MODEL`→`gpt-5.5` (interim) |
| `frontend/app/view/agent/commands/global/runtime.ts` | `/model` + `/effort` choices read the active provider's `models` / `reasoning.levels` |
| `frontend/app/view/agent/components/AgentControlBar.tsx` | model + effort dropdowns render the active provider's lists; hide when empty |

---

## 6. Phasing

- **P1 — interim refresh (small, low-risk):** bump `CODEX_DEFAULT_MODEL` → `gpt-5.5`; gate Claude `--effort` off for `haiku`. No schema change. (Could ship immediately, like #1411.)
- **P2 — data model:** add `models`/`modelFlag`/`reasoning` to `ProviderDefinition` + `ProviderConfig`; populate Claude + Codex + Gemini. `model` → `string`.
- **P3 — wire UI + args:** picker reads provider lists; `buildRuntimeArgs` emits per-provider model + reasoning; back-compat fallback (§3.5).
- **P4 — remaining providers:** qwen reasoning via OpenRouter; verify/confirm flags; Gemini `minimal` handling.

---

## 7. Testing

- Unit (`buildRuntimeArgs`): for each provider, assert the emitted argv — Claude `--effort xhigh` (and **absent** for `haiku`); Codex `-c model_reasoning_effort="high"` placed **before** the trailing `-`; Gemini thinking-level flag; non-Claude panes get their own model, never a Claude alias.
- Back-compat: a stored `{model:"opus", effort:"xhigh"}` on a Codex pane falls back to Codex defaults, not `--model opus`.
- Manual (`task dev`): switch a pane's provider → model/effort dropdowns repopulate; pick a model/effort → correct flag reaches the CLI (check `muxlog srv`); a real turn runs on each of claude/codex/gemini.

## 8. Open questions

1. **Gemini effort granularity** — expose `minimal` as a Gemini-only level, or map our `low`→`minimal`? (Keeps the canonical scale 5-wide vs. per-provider extra.)
2. **`max` on non-Opus Claude** — clamp `max`→`xhigh` for `sonnet` (effort `max` is Opus-tier only), or just document it?
3. **Codex reasoning surface** — confirm `-c model_reasoning_effort="…"` is the stable CLI path vs. a dedicated flag in `codex 0.116.0`.
4. **Qwen** — does the OpenRouter route honor `reasoning_effort`, and is it set via env (`OPENAI_*`) or `-c`?
