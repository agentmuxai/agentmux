# SPEC: Explicit Runtime Summary in Agent Control Bar

Status: Draft
Owner: TBD
Date: 2026-04-20

## Problem

The agent control bar, collapsed, currently shows a summary like:

    Mode: Bypass · Model: Default · Effort: Default

When the user hasn't picked a model or effort, `AgentRuntimeConfig.model`
and `AgentRuntimeConfig.effort` are `null`. `buildRuntimeArgs` skips the
`--model` and `--effort` flags in that case, so the Claude Code CLI picks
its own default. AgentMux doesn't know what that default is, so the UI
shows the literal word "Default".

The user wants the collapsed summary to always show the **explicit**
model and effort the agent is running with — never the placeholder
"Default".

## Goals

1. Collapsed summary always shows three fields: `Mode`, `Model`, `Effort`.
2. Each field shows the concrete value in use (e.g. `Sonnet`, `Medium`),
   not the word "Default".
3. The displayed value matches what the CLI actually runs with.

## Non-goals

- Changing the set of available models or effort levels.
- Reworking the expanded dropdown's select controls.
- Per-provider default resolution (scoped to Claude Code provider only
  for now).

## Approach

Two viable options. Pick one.

### Option A — Pin explicit defaults in `DEFAULT_RUNTIME_CONFIG`

Change `DEFAULT_RUNTIME_CONFIG` in `frontend/app/view/agent/types.ts`
from `{ model: null, effort: null }` to explicit values, e.g.
`{ model: "sonnet", effort: "medium" }`.

- `buildRuntimeArgs` will then always emit `--model sonnet --effort
  medium`, and the UI summary renders the same explicit labels.
- Deterministic: the CLI no longer silently picks.
- Drawback: removes the "let the CLI decide" escape hatch. If Claude
  Code changes its internal default, AgentMux won't follow unless we
  bump this constant.

### Option B — Display-only defaults

Keep `DEFAULT_RUNTIME_CONFIG` as `null` for model/effort. In
`AgentControlBar.compactSummary`, substitute the hardcoded display
fallback (e.g. `"Sonnet"`, `"Medium"`) when the field is null.

- CLI behavior unchanged; "Default" means "CLI picks".
- UI drifts if Claude Code's default changes silently.

### Recommendation

**Option A.** Explicit beats implicit for runtime config the user is
about to send to a subprocess. If we later want a "let CLI decide"
option, we can add a distinct `"auto"` sentinel that the UI can
render as such.

## Concrete changes (Option A)

1. `frontend/app/view/agent/types.ts`
   - `DEFAULT_RUNTIME_CONFIG.model`: `null` → `"sonnet"`
   - `DEFAULT_RUNTIME_CONFIG.effort`: `null` → `"medium"`

2. `frontend/app/view/agent/components/AgentControlBar.tsx`
   - `compactSummary`: drop the `r.model ? … : "Default"` fallback — it
     will never hit null now. Render labels straight from
     `MODEL_LABELS` / `EFFORT_LABELS`.
   - `isNonDefault`: re-check that "non-default" indicator still works
     when defaults are no longer null.

3. `frontend/app/view/agent/buildRuntimeArgs.ts`
   - No code change needed; the existing `if (config.model)` branch
     fires automatically.

4. Tests / smoke
   - New session: summary reads `Mode: Bypass · Model: Sonnet · Effort:
     Medium`. Confirm `--model sonnet --effort medium` appear in
     launched CLI args.
   - Existing sessions with `model: null` in block metadata: confirm
     `getRuntimeConfig` falls back to the new defaults, so the summary
     updates on next open.

## Open questions

- Effort default: `medium` or leave unset for now? Claude Code's own
  default for `--effort` isn't documented here — confirm before
  picking.
- Should "Default" remain in the expanded dropdown as a distinct
  selectable option meaning "let CLI decide"? Under Option A, that
  option currently maps to `null`; keeping it means preserving the
  opt-out.

## Affected files

- `frontend/app/view/agent/types.ts`
- `frontend/app/view/agent/components/AgentControlBar.tsx`
- `frontend/app/view/agent/buildRuntimeArgs.ts` (read-only)
- `frontend/app/view/agent/commands/global/runtime.ts` (review "Default" entry)
