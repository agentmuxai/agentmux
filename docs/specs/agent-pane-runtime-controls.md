# Agent Pane Runtime Controls

**Status:** Proposed
**Date:** 2026-04-09

## Summary

Surface Claude Code CLI runtime flags as interactive controls in the agent
pane. The user can change permission mode, model, effort level, and tool
restrictions **between turns** without restarting the session.

## Background

The agent pane spawns a CLI subprocess per turn via `SubprocessController`.
Each turn reads `cmd:args` and `cmd:env` from block metadata
(`websocket.rs:739-762`), appends `--resume <session_id>`, then spawns a
fresh process. This means **any metadata change the frontend writes between
turns takes effect on the next spawn** — no protocol changes needed.

Currently, all flags are set once at launch (`agent-model.ts:73,173`) and
never updated. The CLI supports rich runtime modes that users can't access.

## CLI Flags to Surface

### 1. Permission Mode (Claude only)

| Mode | Flag | Behavior |
|------|------|----------|
| Bypass | `--dangerously-skip-permissions` | Current default. No prompts. |
| Auto | `--permission-mode auto` | AI classifier approves safe ops, blocks risky ones. |
| Accept Edits | `--permission-mode acceptEdits` | Auto-approve file edits, prompt for commands. |
| Plan | `--permission-mode plan` | Read-only research. No edits, no commands. Outputs a plan. |
| Default | `--permission-mode default` | Prompt for everything. |

**Note:** `--dangerously-skip-permissions` and `--permission-mode` are
mutually exclusive. When switching away from Bypass, remove the
`--dangerously-skip-permissions` flag and add `--permission-mode <mode>`.

**Gemini equivalent:** `--yolo` (bypass) vs removing `--yolo` (prompt).
**Codex equivalent:** `--dangerously-bypass-approvals-and-sandbox` vs
`--full-auto` / no flag.

### 2. Model Selection (Claude only)

| Value | Flag |
|-------|------|
| Default (provider decides) | *(no flag)* |
| Opus | `--model opus` |
| Sonnet | `--model sonnet` |
| Haiku | `--model haiku` |

### 3. Effort Level (Claude only, Opus)

| Value | Flag |
|-------|------|
| Low | `--effort low` |
| Medium | `--effort medium` |
| High (default) | `--effort high` |
| Max (Opus only) | `--effort max` |

### 4. Tool Restrictions (Claude only)

| Control | Flag |
|---------|------|
| Allow specific tools | `--allowedTools "Read,Grep,Glob"` |
| Block specific tools | `--disallowedTools "Bash"` |

### 5. Budget Guard (Claude only)

| Control | Flag |
|---------|------|
| Max spend per turn | `--max-budget-usd 5.00` |
| Max agentic turns | `--max-turns 20` |

## Architecture

### Data Flow

```
User changes control in pane header
  → Frontend updates block metadata (cmd:args and/or cmd:env)
  → User sends next message
  → Backend reads metadata (websocket.rs:739)
  → spawn_turn() uses updated args
  → CLI starts with new flags + --resume <session_id>
```

No backend changes required. The backend already re-reads `cmd:args` and
`cmd:env` from block metadata on every turn.

### Block Metadata Schema

Add a new metadata key for runtime overrides, separate from the base
`cmd:args` set at launch. This avoids stomping the provider's base flags.

```
"agent:runtime" → {
    "permissionMode": "bypass" | "auto" | "acceptEdits" | "plan" | "default",
    "model": null | "opus" | "sonnet" | "haiku",
    "effort": null | "low" | "medium" | "high" | "max",
    "allowedTools": null | string[],
    "disallowedTools": null | string[],
    "maxBudgetUsd": null | number,
    "maxTurns": null | number
}
```

### Arg Assembly (Frontend)

When the user sends a message, the frontend should:

1. Start with the provider's base `launchArgs` (from `providers/index.ts`).
2. Read `agent:runtime` from block metadata.
3. Apply overrides:
   - If `permissionMode !== "bypass"`: remove `--dangerously-skip-permissions`,
     add `--permission-mode <mode>`.
   - If `model` is set: add `--model <model>`.
   - If `effort` is set: add `--effort <effort>`.
   - If `allowedTools` is set: add `--allowedTools` entries.
   - If `disallowedTools` is set: add `--disallowedTools` entries.
   - If `maxBudgetUsd` is set: add `--max-budget-usd <n>`.
   - If `maxTurns` is set: add `--max-turns <n>`.
4. Write the assembled args to `cmd:args` in block metadata.
5. Then send the `AgentInput` RPC as normal.

This can be done in a new function `buildRuntimeArgs(provider, runtime)`
called from `agent-view.tsx` before sending the message, or from
`agent-model.ts` in a new `updateRuntimeArgs()` method.

### Alternative: Backend Arg Assembly

Instead of the frontend assembling args, the backend could read
`agent:runtime` and merge flags in `websocket.rs` before spawning. This
keeps flag logic in one place but requires a Rust change. The frontend
approach is simpler for Phase 1 since it requires zero backend changes.

**Recommendation:** Frontend for Phase 1, migrate to backend if the logic
gets complex.

## UI Design

### Controls Location

Add a **control bar** between the agent header and the document view. It
should be collapsible (chevron toggle) to avoid taking space when not needed.

```
┌─────────────────────────────────────────────────┐
│ 🟢 Agent: agentx        PID: 12345  Connected  │  ← existing header
├─────────────────────────────────────────────────┤
│ ▾ Controls                                      │  ← new control bar
│  Mode: [Bypass ▾]  Model: [Default ▾]          │
│  Effort: [High ▾]  Budget: [___] USD            │
├─────────────────────────────────────────────────┤
│                                                  │
│  (document view)                                 │
│                                                  │
├─────────────────────────────────────────────────┤
│ Send message to agentx...              [Enter]   │  ← existing footer
└─────────────────────────────────────────────────┘
```

When collapsed:

```
│ ▸ Controls: Bypass · Opus · High                │
```

The collapsed line shows current settings as a compact summary.

### Control Components

1. **PermissionModeSelect** — Dropdown with 5 modes. Color-coded:
   - Bypass = red badge (dangerous)
   - Auto = blue
   - Accept Edits = yellow
   - Plan = green (safe/read-only)
   - Default = gray

2. **ModelSelect** — Dropdown: Default, Opus, Sonnet, Haiku.
   Only shown for Claude provider.

3. **EffortSelect** — Dropdown: Low, Medium, High, Max.
   Only shown for Claude provider. "Max" only enabled when model is Opus.

4. **BudgetInput** — Number input for max USD per turn. Optional.

5. **ToolRestrictions** — Future phase. Checkbox list of tools to
   allow/block. Complex UX — defer to Phase 2.

### State Management

Runtime settings live in block metadata (`agent:runtime`). The control bar
reads from and writes to this metadata via `RpcApi.SetMetaCommand`. This
means:

- Settings persist across page refreshes (metadata is in SQLite).
- Settings are per-pane (each agent pane has its own block).
- Changes take effect on the next turn (not mid-turn).

### Visual Feedback

When settings differ from the provider's defaults, show a small indicator
in the collapsed control bar:

```
│ ▸ Controls: Plan · Sonnet · Low  ⚠ non-default │
```

This warns the user that the agent is running in a non-standard config
(e.g., Plan mode won't make edits, which might confuse them if they
expect code changes).

## Phases

### Phase 1: Permission Mode + Model + Effort

- New `AgentControlBar` component
- `buildRuntimeArgs()` function
- `agent:runtime` metadata key
- Claude provider only (Gemini/Codex have simpler flag sets)
- No backend changes

**Files to create:**
- `frontend/app/view/agent/components/AgentControlBar.tsx`

**Files to modify:**
- `frontend/app/view/agent/agent-view.tsx` — insert control bar
- `frontend/app/view/agent/agent-model.ts` — add `updateRuntimeArgs()`
- `frontend/app/view/agent/agent-view.scss` — control bar styles
- `frontend/app/view/agent/types.ts` — `AgentRuntimeConfig` interface

### Phase 2: Budget Guards + Tool Restrictions

- `--max-budget-usd` and `--max-turns` inputs
- Tool allow/block list UI
- Per-provider flag mapping (Gemini `--yolo`, Codex bypass flag)

### Phase 3: Backend Arg Assembly

- Move flag assembly from frontend to `websocket.rs`
- Backend reads `agent:runtime` and merges with base `cmd:args`
- Enables future features like server-side policy enforcement

## Provider Compatibility Matrix

| Control | Claude | Codex | Gemini |
|---------|--------|-------|--------|
| Permission mode | `--permission-mode` | partial (`--full-auto`) | `--yolo` toggle |
| Model selection | `--model` | `--model` | `--model` |
| Effort level | `--effort` | — | — |
| Tool restrictions | `--allowedTools` | — | — |
| Budget guard | `--max-budget-usd` | — | — |
| Max turns | `--max-turns` | — | — |

Phase 1 targets Claude only. Gemini/Codex support can be added by extending
`buildRuntimeArgs()` with provider-specific flag mapping.

## Testing

1. Launch an agent pane with default settings → verify `--dangerously-skip-permissions` is in args.
2. Switch to Plan mode → send a message → verify `--permission-mode plan` in spawn args (check sidecar log).
3. Switch back to Bypass → verify `--dangerously-skip-permissions` returns.
4. Set model to Sonnet → verify `--model sonnet` in next spawn.
5. Collapse/expand control bar → verify state persists.
6. Close and reopen pane → verify settings persist from block metadata.
7. Verify controls are hidden for non-Claude providers.
