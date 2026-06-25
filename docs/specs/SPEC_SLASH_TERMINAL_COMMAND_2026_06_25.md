# SPEC: `/terminal` Slash Command — Open Agent CWD in New Terminal Pane

**Date:** 2026-06-25
**Status:** Draft
**Scope:** New slash command + one registration line

---

## Goal

Typing `/terminal` in an agent pane opens a new terminal block in a pane **below**
the agent (vertical split, "after" position), pre-seeded with the agent's current
working directory.

---

## Why not to the side

The existing pane-action helpers default to horizontal splits (right). For a
terminal that supplements a running agent session, "below" is more ergonomic:

- The agent conversation is tall and narrow; a side terminal compresses both panes.
- "Below" mirrors the conventional IDE layout (editor above, terminal below).
- Horizontal splits already exist in the right-click header menu; this command
  fills the gap for the vertical/below case.

---

## System map

### Slash command pipeline

```
AgentComposerInput
  → useAgentCommands.sendMessage()
  → dispatchSlashCommand(input, registry, ctx)   ← commands/dispatch.ts
  → registry.lookup("terminal")
  → terminalCommand.handler(ctx)
```

### Key files

| Role | File |
|---|---|
| Command type + context interface | `frontend/app/view/agent/commands/types.ts` |
| Registry class + buildRegistry() | `frontend/app/view/agent/commands/registry.ts` |
| Global command registration | `frontend/app/view/agent/commands/global/index.ts` |
| Block split helpers | `frontend/app/store/global.ts` — `createBlockSplitVertically()` |
| Agent CWD storage | `meta["cmd:cwd"]` set in `agent-model.ts:555` during `launchAgent()` |
| Terminal view meta key | `meta.view = "term"` (standard block meta) |

---

## Split direction mapping (pane-actions.ts:12–95)

```
"down" → createBlockSplitVertically(blockDef, targetBlockId, "after")
"up"   → createBlockSplitVertically(blockDef, targetBlockId, "before")
"right"→ createBlockSplitHorizontally(blockDef, targetBlockId, "after")   ← default
"left" → createBlockSplitHorizontally(blockDef, targetBlockId, "before")
```

We want `createBlockSplitVertically(…, "after")` — splits below the agent pane.

---

## CWD resolution

```typescript
const cwd = ctx.block()?.meta?.["cmd:cwd"];
```

`ctx.block()` returns the raw WOS block object for the agent pane. `cmd:cwd` is
written at agent launch time (`agent-model.ts:555`). It is always an absolute path.

If the field is absent (agent never launched, or block not found), the terminal
opens without a `cwd` override — the OS default applies.

### Terminal CWD handoff

The terminal view reads its starting directory from `meta["cmd:cwd"]` on the block
(same key, different block). Setting this on the `BlockDef` before creation causes
the subprocess to `cd` to that path on spawn.

---

## Implementation

### 1 — New file: `frontend/app/view/agent/commands/global/terminal.ts`

```typescript
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createBlockSplitVertically } from "@/app/store/global";
import type { SlashCommand, SlashResult } from "../types";

export const terminalCommand: SlashCommand = {
    name: "terminal",
    category: "session",
    description: "Open the agent's working directory in a new terminal pane below",
    arg: { kind: "none" },
    availability: "any-agent",
    handler: async (ctx): Promise<SlashResult> => {
        const cwd = ctx.block()?.meta?.["cmd:cwd"] as string | undefined;
        const blockDef = {
            meta: {
                view: "term",
                ...(cwd ? { "cmd:cwd": cwd } : {}),
            },
        };
        await createBlockSplitVertically(blockDef, ctx.blockId, "after");
        return {
            kind: "ok",
            message: cwd ? `Terminal opened at ${cwd}` : "Terminal opened",
        };
    },
};
```

### 2 — Register in `frontend/app/view/agent/commands/global/index.ts`

```typescript
import { terminalCommand } from "./terminal";

export function registerGlobalCommands(registry: SlashCommandRegistry): void {
    registry.register(loginCommand);
    registry.register(clearCommand);
    registry.register(helpCommand);
    registry.register(toolsCommand);
    registry.register(terminalCommand);   // ← add
    // ...RUNTIME_COMMANDS
}
```

---

## Error handling

| Condition | Behaviour |
|---|---|
| `cmd:cwd` absent (agent not yet started) | Terminal opens; no `cmd:cwd` on block; OS default CWD applies |
| `createBlockSplitVertically` throws | Exception propagates to `dispatchSlashCommand` → logged as error result in conversation |
| `/terminal someArg` typed | `arg: { kind: "none" }` → dispatcher strips unknown args; handler receives no arg |

---

## Files to create / change

| File | Change |
|---|---|
| `frontend/app/view/agent/commands/global/terminal.ts` | New — 28 lines |
| `frontend/app/view/agent/commands/global/index.ts` | +1 import, +1 `registry.register()` call |

No changes needed to:
- `commands/types.ts` — `SlashCommandContext.block()` already exposes raw meta
- `commands/registry.ts` — no new category or dispatch path
- `store/global.ts` — `createBlockSplitVertically` already exists and is exported
- Terminal view model — reads `cmd:cwd` from block meta already on spawn

---

## Open questions

1. **Should `/terminal` accept an optional path arg?**
   e.g. `/terminal ~/projects/foo` — would override `cmd:cwd`. Useful but adds
   path validation complexity. Defer to v2.

2. **Focus after open?**
   After split, focus moves to the new terminal pane automatically (layout default).
   Confirm this is the desired UX — alternative is to keep focus on the agent.

3. **What if the pane is already in a vertical split (multiple rows)?**
   `createBlockSplitVertically` adds another row within the same column. This is
   correct — terminal goes below the agent in the same column regardless of existing
   layout.
