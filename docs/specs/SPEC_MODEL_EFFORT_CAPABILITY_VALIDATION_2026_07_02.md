<!--
Copyright 2026, AgentMux Corp.
SPDX-License-Identifier: Apache-2.0
-->

# SPEC — Per-model effort-capability validation for the composer strip

**Date:** 2026-07-02
**Author:** AgentA
**Status:** Proposed
**Depends on:** #1926 (model-catalog overlay) — this stacks on top.
**Source of truth for the matrix:** `docs/providers/PROVIDER_MODELS_EFFORT_SETTINGS_2026-06.md §1`

---

## 1. Problem

The composer-strip **Effort** drop-up offers a fixed set of five levels for **every**
model, but the levels a model actually accepts differ per model — and sending an
unsupported one **400s the turn**. Today nothing validates this beyond a single
hardcoded string check.

### 1.1 Current state (evidence)

| Concern | Where | Behavior |
|---|---|---|
| Effort list is static + universal | `AgentComposerStrip.tsx:44` (`EFFORT_OPTIONS`) | `low/medium/high/xhigh/max` rendered identically for every model; no per-model filtering. |
| `/effort` slash command likewise | `commands/global/runtime.ts:105-109` (`effortChoices`) | Same five choices, unconditional. |
| Only guard that exists | `buildRuntimeArgs.ts:110` | `if ((!providerId || providerId === "claude") && config.model !== "haiku") args.push("--effort", …)` — a single literal `!== "haiku"`. |
| Effort type | `types.ts:626` | `type EffortLevel = "low" \| "medium" \| "high" \| "xhigh" \| "max"` |
| Default | `types.ts:641` | `effort: "high"` |

### 1.2 The real matrix (from the authoritative doc §1)

- Effort **works on**: Fable 5, Opus 4.5–4.8, Sonnet 4.6.
- Effort **400s on**: **Haiku 4.5** and **Sonnet 4.5** (and older Sonnets).
- `max` is **narrower still** — valid only on **Fable 5, Opus 4.6+, Sonnet 4.6**; **not** Haiku, **not** older Sonnets.

So two independent facts must be modeled per model: **(a) does it accept `--effort` at
all**, and **(b) what is its highest valid level** (`max` vs `xhigh`).

### 1.3 Why the catalog can't supply this

`GET /v1/models` returns only `{ id, display_name, type, created }` — **no
effort/reasoning capability field**. Unlike the model *labels* (which #1926 refreshes
from the API), effort validity has **no authoritative machine-readable source**. It must
be a **curated capability map**, seeded from the doc above and updated on CLI pin bumps —
same cadence as the curated model metadata already is.

### 1.4 Interaction with #1926 (important)

#1926's overlay changes model **values** for families the curated list doesn't cover
(e.g. Fable → concrete id `claude-fable-5`) and keeps curated families on their aliases
(`opus`/`sonnet`/`haiku`). Two consequences:

- The existing `config.model !== "haiku"` guard still fires for the curated haiku entry
  (value stays `"haiku"`) — good, but it is **capability-blind**: a string match, not data.
- Auto-appended families (Fable today) have **no effort metadata**, so the resolver needs
  a **safe default** for unknown models (see §4.4). Fable *does* support effort incl.
  `max`, but the mechanism must not assume that for the *next* unknown family.

---

## 2. Goals / non-goals

**Goals**
1. The Effort drop-up shows **only levels valid for the currently-selected model**.
2. `--effort` is emitted **iff** the model supports effort — replacing the `!== "haiku"`
   special case with data-driven gating (Sonnet 4.5 is also unsupported today and slips
   through).
3. The `/effort` slash command offers the same validated subset.
4. Switching to a model that doesn't support the current effort **clamps** the effort to a
   valid value instead of carrying an invalid one into the next turn.
5. New families auto-appended by the catalog get **safe, non-crashing defaults**.

**Non-goals**
- Generalizing effort to Codex/Gemini (`model_reasoning_effort` / `thinking_level`). The
  doc §11.5 tracks that; this spec is Claude-only to stay scoped. The data model below is
  shaped so it *can* extend there later.
- Fetching effort capability from any API (there is none — §1.3).
- Changing the effort **default** (`high`) or the model default. Out of scope.

---

## 3. Capability data model

Add optional effort metadata to `ProviderModel` (`providers/index.ts:36`). Optional so
non-Claude providers and auto-appended models are unaffected.

```ts
export interface ProviderModel {
    value: string;
    label: string;
    default?: boolean;
    description?: string;
    aliases?: string[];
    /**
     * Reasoning-effort capability. Omitted → provider default applies
     * (see resolveEffortCapability). `supported: false` means the model 400s on
     * any `--effort` (Haiku 4.5, Sonnet 4.5). `maxLevel` caps the offered levels
     * (e.g. "xhigh" for models where `max` 400s). Source of truth:
     * docs/providers/PROVIDER_MODELS_EFFORT_SETTINGS_2026-06.md §1.
     */
    effort?: {
        supported: boolean;
        /** Highest valid level; levels above it are hidden. Default "max". */
        maxLevel?: EffortLevel;
    };
}
```

Curated Claude entries (`providers/index.ts:196-200`) become:

```ts
models: [
    { value: "opus",   label: "Opus 4.8",   description: "…", aliases: ["claude-opus"],
      effort: { supported: true, maxLevel: "max" } },
    { value: "sonnet", label: "Sonnet 4.6", default: true, description: "…", aliases: ["claude-sonnet"],
      effort: { supported: true, maxLevel: "max" } },
    { value: "haiku",  label: "Haiku 4.5",  description: "…", aliases: ["claude-haiku"],
      effort: { supported: false } },
],
```

> Note: the curated `sonnet` alias resolves to the *current* Sonnet (4.6), which supports
> `max`. The doc's "Sonnet 4.5 400s" caveat only matters if a concrete older Sonnet ever
> becomes selectable; the alias insulates us. The generic default (§4.4) covers that case
> anyway.

### 3.1 Ordering of `EffortLevel`

Introduce a single canonical rank so "levels above `maxLevel`" is well-defined and reused
by clamping:

```ts
export const EFFORT_ORDER: EffortLevel[] = ["low", "medium", "high", "xhigh", "max"];
```

Place next to `EffortLevel` in `types.ts` so both the UI and `buildRuntimeArgs` import one
source.

---

## 4. Resolver — the single decision point

One pure function that every consumer calls. Lives in a new
`frontend/app/view/agent/agent-effort.ts` (mirrors the existing `agent-model.ts`).

```ts
import { EffortLevel, EFFORT_ORDER } from "./types";
import { getProvider, ProviderModel } from "./providers";

export interface EffortCapability {
    supported: boolean;
    /** The allowed levels, in order, for this model (subset of EFFORT_ORDER). */
    allowed: EffortLevel[];
}

/** Effort capability for (providerId, modelValue). Data-driven; safe defaults
 *  for unknown models (§4.4). */
export function resolveEffortCapability(providerId: string, modelValue: string): EffortCapability;

/** Clamp a desired effort to the nearest valid level for a model — highest
 *  allowed level ≤ desired, else the model's lowest allowed. Returns null when
 *  the model supports no effort at all. */
export function clampEffort(providerId: string, modelValue: string, desired: EffortLevel): EffortLevel | null;
```

### 4.1 UI — Effort drop-up (`AgentComposerStrip.tsx`)

`EFFORT_OPTIONS` becomes derived, not static:

```tsx
const effortOptions = createMemo(() => {
    const cap = resolveEffortCapability(props.providerId ?? "", runtime()?.model ?? "");
    return EFFORT_OPTIONS_ALL.filter((o) => cap.allowed.includes(o.value as EffortLevel));
});
```

- If `cap.allowed` is empty (model doesn't support effort, e.g. Haiku): **hide the Effort
  drop-up entirely** (render nothing) rather than an empty menu. This is cleaner than a
  disabled trigger and needs no new `MenuItem` field.
- **Why filter, not grey-out:** the global `MenuItem` type (`custom.d.ts:405`) has no
  `disabled`/`enabled` field, so a disabled row would require extending `MenuItem` +
  FlyoutMenu render. Filtering needs neither. Greying-out is deferred to §7 (optional).

### 4.2 `/effort` slash command (`commands/global/runtime.ts`)

`effortChoices` filters the same way, reading the block's current model:

```ts
function effortChoices(ctx: SlashCommandContext): SlashChoice[] {
    const meta = ctx.block()?.meta;
    const cfg = getRuntimeConfig(meta);
    const cap = resolveEffortCapability(providerOf(meta), cfg.model);
    return ALL_EFFORT_CHOICES.filter((c) => cap.allowed.includes(c.value as EffortLevel));
}
```

If the model supports no effort, `/effort` returns zero choices and the command surfaces a
"model X does not support reasoning effort" message rather than silently accepting one.

### 4.3 `buildRuntimeArgs.ts` — data-driven gate

Replace the `!== "haiku"` literal (`:110`) with the resolver + clamp:

```ts
if (!providerId || providerId === "claude") {
    const eff = clampEffort(providerId ?? "claude", config.model, config.effort);
    if (eff) args.push("--effort", eff);   // null → model supports no effort → omit
}
```

This is the safety net: even if the UI leaks an invalid value (stale meta, race with a
model switch), the arg builder clamps or drops it, so **no turn 400s on effort**.

### 4.4 Safe default for unknown models (§1.4)

`resolveEffortCapability` when the model has no `effort` metadata (auto-appended families,
or an id the curated list doesn't recognize):

- **Claude provider, unknown model → `supported: true, allowed = [low..xhigh]`** (omit
  `max`). Rationale: every current Claude model accepts `low..high`; `xhigh` is the coding
  default; `max` is the *only* level with a genuinely narrow support set, so excluding it
  by default is the conservative choice that won't 400. Fable (which *does* support `max`)
  can be promoted to `maxLevel: "max"` via a one-line curated `effort` entry keyed on the
  `claude-fable` family when we choose to — but it works safely without it.
- **Non-Claude provider → `supported: false`** (matches today; effort is Claude-only).

This makes the whole system **fail-safe**: a brand-new family the catalog surfaces gets a
non-crashing effort set with zero code changes, and can be tuned later.

### 4.5 Clamp on model switch (Goal 4)

When the user changes **model** (`updateRuntime({ model })` in `AgentComposerStrip.tsx`,
and the `/model` command), run `clampEffort(provider, newModel, currentEffort)` and, if it
differs from the stored effort, patch effort in the **same** runtime update. Example:
Haiku selected while effort was `max` → effort clamped to `null`→ dropped; switch to Opus
→ effort restored from default (`high`) if it was previously dropped. Keep the rule simple:
persist the *user-intended* effort, but never send an invalid one (the arg builder is the
final guard, §4.3).

> Decision needed (§8-Q1): do we (a) silently clamp, or (b) clamp + a one-line toast
> ("Haiku doesn't support effort — reasoning level hidden")? Recommend (b) for the strip's
> first occurrence per pane, silent thereafter.

---

## 5. Files touched

| File | Change |
|---|---|
| `frontend/app/view/agent/types.ts` | Add `EFFORT_ORDER`; keep `EffortLevel`. |
| `frontend/app/view/agent/providers/index.ts` | Add `effort?` to `ProviderModel`; populate curated Claude entries. |
| `frontend/app/view/agent/agent-effort.ts` | **New** — `resolveEffortCapability`, `clampEffort`, defaults. |
| `frontend/app/view/agent/components/AgentComposerStrip.tsx` | Derive `effortOptions`; hide drop-up when empty; clamp on model switch. |
| `frontend/app/view/agent/commands/global/runtime.ts` | `effortChoices` filters; `/effort` on unsupported model messages. |
| `frontend/app/view/agent/buildRuntimeArgs.ts` | Replace `!== "haiku"` with `clampEffort`. |
| `docs/providers/PROVIDER_MODELS_EFFORT_SETTINGS_2026-06.md` | Cross-link this spec as the implementation of §11.3. |

No backend/Rust changes. No `providers.models` RPC change. No `MenuItem`/FlyoutMenu change
(unless §7 is taken).

---

## 6. Testing

- **Unit (`agent-effort.test.ts`)**: `resolveEffortCapability`/`clampEffort` for
  opus (max allowed), sonnet (max), haiku (unsupported → empty), unknown-claude
  (low..xhigh), non-claude (unsupported); clamp cases (`max`→`xhigh` when capped, `max`→
  dropped on haiku, in-range unchanged).
- **`buildRuntimeArgs.test.ts`**: extend — no `--effort` for haiku (existing, now
  data-driven); `--effort` present for opus/sonnet; `max` clamped to `xhigh` for a
  capped model; effort dropped for a non-claude provider.
- **Contract/UI**: strip renders 5 levels for opus, ≤4 for a capped model, hides Effort for
  haiku; `/effort` choice count matches.

---

## 7. Optional enhancement — grey-out instead of hide

If product prefers showing invalid levels greyed (discoverability) over hiding them:
add `enabled?: boolean` (or reuse the existing `visible?`) to the global `MenuItem`
(`custom.d.ts:405`) and have `FlyoutMenu` render a non-clickable dimmed row. Then
`StripSelect` passes `enabled: cap.allowed.includes(o.value)`. Larger blast radius
(touches the shared menu component) — **not** in the base implementation; filtering (§4.1)
ships first.

---

## 8. Open decisions

- **Q1 — clamp UX:** silent vs one-time toast on effort clamp/hide (§4.5). *Recommend:
  toast once per pane, then silent.*
- **Q2 — Fable `max`:** ship Fable via the §4.4 safe default (`low..xhigh`, no `max`), or
  add a curated `claude-fable` effort entry granting `max` now? *Recommend: safe default
  first (correct, never 400s), promote to `max` in a follow-up once verified live.*
- **Q3 — hide vs disable:** filtering (hide) is the base plan (§4.1); grey-out is §7.
  *Recommend: hide.*

---

## 9. Rollout

Single PR, stacked on #1926. No migration (metadata is additive; missing `effort` →
safe default). Reversible by dropping the `effort` fields (resolver falls back to
defaults). Verify with `cargo` untouched, `tsc --noEmit`, the new unit tests, and the
`rpc-contract` test (unaffected — no RPC surface change).
