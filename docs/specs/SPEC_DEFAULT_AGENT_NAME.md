# Default agent name on launch

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-09
**Driving observation:** Creating a new agent currently requires the user to type a name before they can click Launch. There's nothing useful to type — the user just wants to spin up an agent. The empty-name requirement is friction with no payoff.

## Symptom

Open agent pane → name field is empty → Launch button is disabled (or fails on submit) until the user types something. Even worse: typing can be blocked by the zombie-HWND bug (#779), trapping the user with no way to launch.

## Proposed behavior

When the launch form opens, **pre-populate the name field** with a default derived from the selected provider:

- Provider "Claude" → default name `Claude Agent`
- Provider "OpenAI" → default name `OpenAI Agent`
- Provider "Gemini" → default name `Gemini Agent`
- ... etc, one per supported provider

If a default-named agent already exists, suffix with `2`, `3`, ... — e.g., `Claude Agent`, `Claude Agent 2`, `Claude Agent 3`. Take the lowest unused suffix; don't fill gaps (`Claude Agent 5` is fine even if `Claude Agent 3` was deleted).

User can still edit the field. The default just removes the friction of needing to type something to proceed.

## Implementation outline

### Where the default is computed

The name field's initial value lives in the launch form / agent picker component. Likely `frontend/app/view/agent/components/AgentPicker.tsx` or wherever the launch modal is constructed.

### Algorithm

```ts
function defaultAgentName(providerName: string, existingNames: Set<string>): string {
    const base = `${providerName} Agent`;
    if (!existingNames.has(base)) return base;
    let n = 2;
    while (existingNames.has(`${base} ${n}`)) n++;
    return `${base} ${n}`;
}
```

### When to compute

- On form open: compute default from the currently-selected provider.
- On provider change: re-compute (e.g., user picks Claude → default `Claude Agent`; switches to OpenAI → default updates to `OpenAI Agent`).
  - **Caveat:** if the user has already edited the field, don't overwrite their input. Track an `isDirty` flag — switching providers only updates the default while `isDirty === false`.

### `existingNames` source

`useForgeAgents()` already exposes the full agent list. Map to a `Set<string>` of `agent.name` once per render, pass to `defaultAgentName`.

### Validation

Currently the form likely rejects empty names. With a default, that case mostly goes away — but keep the validation (user might delete the default). Empty name on submit → keep current error behavior.

## Effort

| Component | LOC | Days |
|---|---|---|
| `defaultAgentName` helper + tests | ~30 | 0.25 |
| Wire to launch form (compute on open + provider change, respect `isDirty`) | ~25 | 0.25 |
| **Total** | ~55 | **~0.5 day** |

## Out of scope

- **Smart names from context** (e.g., "Refactor Auth Module" derived from current branch / cwd / open file). Could come later as a "name suggestion" affordance, not a default.
- **Renaming UI** for existing agents. Separate concern.
- **Provider-specific defaults beyond `<Provider> Agent`** — e.g., model-specific names like `Claude Sonnet Agent`. Keep v1 simple; revisit only if users ask.

## Cross-references

- #779 — zombie HWND eating keystrokes makes the empty-name case currently un-typeable in some sessions; this spec is a partial mitigation by removing the need to type at all.
- `frontend/app/view/agent/components/AgentPicker.tsx` — likely host for the change.
- `useForgeAgents()` hook — source of existing agent names for collision detection.

## Driving observation (verbatim)

> "lets also add a default name for an agent, so they can click launch immediatly. the name is just the provider then agent like Claude Agent .. and if there was already one made, add a Claude Agent 2"
