# Command Palette — Spec

**Status:** Draft  
**Target:** v0.35.x  
**Authors:** AgentA

---

## 1. What It Is

A global fuzzy-search command palette, triggered by `Ctrl+P`, that lets the user (and agents) discover and execute app-level actions without navigating menus.

The palette is a single unified registry. The UI is one entry point into it. Agents and MCP tools are another entry point — same registry, same IDs, same execution paths. There is no "agent API" separate from the "human API"; both call `run_command`.

Design reference: VS Code `Ctrl+Shift+P`, Linear `Ctrl+K`. The AgentMux palette uses `Ctrl+P` only (no `>` prefix distinction — we have one mode to start).

---

## 2. Scope for v1

### In scope
- Global fuzzy-search over a static command set (defined at app init)
- Keyboard-only activation and navigation (mouse also works)
- Agent/MCP programmatic access via a new `run_command` IPC command
- Five command categories: pane/widget opening, window management, pane splitting, navigation, developer tools

### Out of scope for v1
- Parameterized commands (e.g., "open terminal in /some/path") — see Section 8
- Plugin-registered commands — see Section 8
- Recent commands history
- Command aliases / user remapping
- Search across open pane content

---

## 3. Command Categories and Initial Command Set

Five categories. Each command has a stable string ID used in both the UI and the programmatic API.

### Category: open

Open a widget as a new pane in the current tab. Uses `createBlock()` with the appropriate `BlockDef` from `widgets.json`.

| ID | Label | What It Does |
|----|-------|-------------|
| `open:terminal` | Open Terminal | Creates a new terminal pane (`view: "term"`, `controller: "shell"`) |
| `open:agent` | Open Agent | Creates an agent pane (`view: "agent"`, `controller: "cmd"`) |
| `open:forge` | Open Forge | Creates a forge pane (`view: "forge"`) |
| `open:sysinfo` | Open System Info | Creates a sysinfo pane (`view: "sysinfo"`) |
| `open:identity` | Open Identity | Creates an identity pane (`view: "identity"`) |
| `open:help` | Open Help | Creates a help pane (`view: "help"`) |
| `open:swarm` | Open Swarm | Creates a swarm pane (`view: "swarm"`) |

### Category: split

Split the currently focused pane. Uses `createBlockSplitHorizontally` / `createBlockSplitVertically` with the default new block def (inherits connection and cwd from focused pane, matching existing behavior in `keymodel.ts`).

| ID | Label | What It Does |
|----|-------|-------------|
| `split:right` | Split Right | Splits focused pane horizontally, new pane after |
| `split:left` | Split Left | Splits focused pane horizontally, new pane before |
| `split:down` | Split Down | Splits focused pane vertically, new pane after |
| `split:up` | Split Up | Splits focused pane vertically, new pane before |

### Category: window

Window-level operations. Maps to existing IPC commands.

| ID | Label | What It Does |
|----|-------|-------------|
| `window:new` | New Window | Opens a new AgentMux window (`open_new_window`) |
| `window:close` | Close Window | Closes the current window (`close_window`) |
| `window:minimize` | Minimize Window | Minimizes the current window (`minimize_window`) |
| `window:maximize` | Toggle Maximize | Maximizes or restores the current window (`maximize_window`) |
| `tab:new` | New Tab | Creates a new tab in the current workspace (`createTab()`) |
| `tab:close` | Close Tab | Closes the active tab (`simpleCloseStaticTab` equivalent) |
| `tab:next` | Next Tab | Switches to next tab |
| `tab:prev` | Previous Tab | Switches to previous tab |

### Category: pane

Pane-level operations on the currently focused pane.

| ID | Label | What It Does |
|----|-------|-------------|
| `pane:close` | Close Pane | Closes focused pane (`layoutModel.closeFocusedNode`) |
| `pane:magnify` | Toggle Magnify | Magnifies or restores focused pane (`magnifyNodeToggle`) |
| `pane:focus:right` | Focus Pane Right | Moves focus right (`switchBlockInDirection`) |
| `pane:focus:left` | Focus Pane Left | Moves focus left |
| `pane:focus:up` | Focus Pane Up | Moves focus up |
| `pane:focus:down` | Focus Pane Down | Moves focus down |

### Category: dev

Developer-facing operations.

| ID | Label | What It Does |
|----|-------|-------------|
| `dev:devtools` | Toggle DevTools | Toggles browser inspector (`toggle_devtools` IPC) |
| `dev:restart_backend` | Restart Backend | Restarts the agentmux-srv sidecar (`restart_backend` IPC) |
| `dev:open_settings` | Open Settings File | Opens settings JSONC in external editor (`open_in_editor` IPC) |

---

## 4. UI Component

### Activation

`Ctrl+P` opens the palette. `Escape` closes it. `Enter` executes the selected command and closes.

Add to `globalKeyMap` in `frontend/app/store/keymodel.ts`:

```typescript
globalKeyMap.set("Ctrl:p", () => {
    commandPaletteModel.open();
    return true;
});
```

`Escape` already calls `modalsModel.popModal()` — the palette registers as a modal so this works for free.

### Component Location

```
frontend/app/modals/command-palette.tsx
frontend/app/modals/command-palette.scss
```

### SolidJS Component Sketch

```tsx
// CommandPalette.tsx
import { createSignal, createMemo, For, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { commandRegistry, CommandEntry } from "@/app/store/command-registry";

const [query, setQuery] = createSignal("");

const filtered = createMemo(() => {
    const q = query().toLowerCase().trim();
    if (!q) return commandRegistry.all();
    return commandRegistry.all().filter((cmd) =>
        cmd.label.toLowerCase().includes(q) ||
        cmd.id.toLowerCase().includes(q) ||
        cmd.category.toLowerCase().includes(q)
    );
});
```

The palette renders as a centered overlay (not anchored to a pane like `TypeAheadModal`). It uses `Portal` with `document.body` as the mount target. A backdrop div behind it catches clicks to dismiss.

Width: 560px, max-height: 480px, centered horizontally, positioned 20% from top.

### Keyboard Navigation

- `ArrowUp` / `ArrowDown`: move selection
- `Enter`: execute selected command
- `Escape`: close without executing
- Typing filters results

The palette must call `disableGlobalKeybindings()` on open and `enableGlobalKeybindings()` on close to prevent hotkeys from firing while the user types. Both are exported from `keymodel.ts`.

### Modal Registration

Register in `frontend/app/modals/modalregistry.tsx` so `modalsModel` can manage open/close state and `Escape` handling works automatically.

---

## 5. Command Registry (TypeScript)

### Location

```
frontend/app/store/command-registry.ts
```

### Shape

```typescript
export interface CommandEntry {
    id: string;           // stable, dot-namespaced: "open:terminal"
    label: string;        // human label shown in palette: "Open Terminal"
    category: string;     // grouping label: "Open", "Split", "Window", ...
    icon?: string;        // FA icon name (optional)
    iconColor?: string;   // hex color (optional, used for widget icons)
    execute: () => void | Promise<void>;
}

class CommandRegistry {
    private commands = new Map<string, CommandEntry>();

    register(entry: CommandEntry): void {
        this.commands.set(entry.id, entry);
    }

    get(id: string): CommandEntry | undefined {
        return this.commands.get(id);
    }

    all(): CommandEntry[] {
        return Array.from(this.commands.values());
    }

    run(id: string): boolean {
        const cmd = this.commands.get(id);
        if (!cmd) return false;
        void Promise.resolve(cmd.execute());
        return true;
    }
}

export const commandRegistry = new CommandRegistry();
```

### Registration

Commands are registered at app init, after global signals are available. A `registerDefaultCommands()` function in `command-registry.ts` imports from `global.ts`, `keymodel.ts`, and `getApi()`.

```typescript
// Called once from frontend/wave.ts or cef-init.ts after init
export function registerDefaultCommands(): void {
    commandRegistry.register({
        id: "open:terminal",
        label: "Open Terminal",
        category: "Open",
        icon: "square-terminal",
        execute: () => createBlock({ meta: { view: "term", controller: "shell" } }),
    });
    // ... remaining commands
}
```

Widget metadata (icons, colors) is pulled from the known widget definitions — these are static so they can be hardcoded. Do not fetch `widgets.json` at runtime to build the registry.

---

## 6. IPC Handler (Rust)

### New Command: `run_command`

Agents and MCP tools call this endpoint to execute a palette command programmatically. It does not open the UI.

```json
POST /ipc
{
  "cmd": "run_command",
  "args": { "id": "open:terminal" }
}
```

Response on success:
```json
{ "success": true, "data": null }
```

Response on unknown ID:
```json
{ "success": false, "error": "Unknown command: open:foobar" }
```

### Rust-side Implementation

Add to `route_command` in `agentmux-cef/src/ipc.rs`:

```rust
"run_command" => commands::palette::run_command(state, args).await,
```

New file: `agentmux-cef/src/commands/palette.rs`

```rust
pub async fn run_command(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = args["id"]
        .as_str()
        .ok_or_else(|| "run_command: missing 'id' field".to_string())?;

    // Dispatch to the frontend via a CEF custom event.
    // The frontend listens for "agentmux-run-command" and calls commandRegistry.run(id).
    let js = format!(
        "window.dispatchEvent(new CustomEvent('agentmux-run-command', {{ detail: {{ id: {:?} }} }}));",
        id
    );
    state.execute_js(&js)?;

    Ok(serde_json::Value::Null)
}
```

The Rust side does not know which command IDs exist — it forwards to the frontend, which owns the registry. The frontend fires an `agentmux-run-command` CustomEvent with `detail.id`. If the ID is unknown, the frontend logs a warning; the IPC call still returns success (fire-and-forget semantics). If callers need to know whether the command ID was valid, a future version can add a `list_commands` IPC endpoint that returns all registered IDs.

### Frontend Event Listener

In `command-registry.ts`, at module load:

```typescript
window.addEventListener("agentmux-run-command", (e: CustomEvent) => {
    const id = e.detail?.id as string;
    if (!commandRegistry.run(id)) {
        console.warn(`[command-palette] Unknown command dispatched: ${id}`);
    }
});
```

### Authentication

All IPC calls require `Authorization: Bearer {ipc_token}` — this is already enforced by the existing `handle_ipc` middleware in `ipc.rs`. Agents get the token through the normal `AGENTMUX_AUTH_KEY` / `get_auth_key` bootstrap flow.

---

## 7. Agent / MCP Usage

Agents invoke commands via the same IPC bridge they already use:

```typescript
// From an agent or MCP tool
await invokeCommand("run_command", { id: "open:terminal" });
await invokeCommand("run_command", { id: "split:right" });
await invokeCommand("run_command", { id: "window:new" });
```

MCP via `mcp__windows-mcp` can also trigger commands if needed, but the IPC route is preferred because it does not require simulating keystrokes.

Agent-authored commands (e.g., an agent registering a "run my build" command) are out of scope for v1 but the registry API supports it — see Section 8.

---

## 8. Future Extensibility

### Parameterized Commands

Some commands need input: "Open Terminal in..." needs a path, "Focus Pane #N" needs a number. The registry shape can be extended:

```typescript
interface ParameterizedCommandEntry extends CommandEntry {
    params: CommandParam[];
    execute: (params: Record<string, string>) => void | Promise<void>;
}
```

The palette UI would show a second input step after the command is selected. The `run_command` IPC would accept an optional `params` map alongside `id`.

### Plugin / Agent-Registered Commands

Any ViewModel or agent could call `commandRegistry.register(...)` to add commands at runtime. The palette displays whatever is in the registry at the time it opens.

For agent-registered commands, a naming convention keeps IDs from colliding: `agent:<agentId>:<action>` (e.g., `agent:agenta:run_tests`). A `list_commands` IPC endpoint (future) would let agents discover what is registered.

### Scoped Commands

Commands that only apply when a specific view is focused (e.g., terminal-specific search commands). The `CommandEntry` interface can gain an optional `isAvailable?: () => boolean` predicate. Commands where `isAvailable()` returns false are hidden from results.

### Keyboard Shortcut Display

Each `CommandEntry` can gain an optional `keybinding?: string` field (e.g., `"Ctrl+D"`). The palette renders it right-aligned in the result row. This is cosmetic — keybindings are still registered separately in `keymodel.ts`.

---

## 9. Implementation Sketch

Minimal walking skeleton, in order:

1. **`frontend/app/store/command-registry.ts`** — `CommandEntry`, `CommandRegistry`, `commandRegistry` singleton, `registerDefaultCommands()`, `agentmux-run-command` event listener.

2. **`agentmux-cef/src/commands/palette.rs`** — `run_command()` handler, dispatches JS event via `state.execute_js()`.

3. **Wire Rust** — add `"run_command"` match arm in `agentmux-cef/src/ipc.rs`, add `pub mod palette;` in `agentmux-cef/src/commands/mod.rs`.

4. **`frontend/app/modals/command-palette.tsx`** + **`command-palette.scss`** — overlay component with input, filtered list, keyboard nav.

5. **Wire keybinding** — add `Ctrl:p` to `globalKeyMap` in `frontend/app/store/keymodel.ts`, calling `commandPaletteModel.open()`.

6. **Wire modal** — add `CommandPalette` to `frontend/app/modals/modalregistry.tsx` so Escape dismissal works.

7. **Call `registerDefaultCommands()`** — from `frontend/cef-init.ts` after `initGlobalAtoms()`.

### No New Dependencies

The component uses SolidJS primitives already in the project. Fuzzy match for v1 is a simple `String.includes()` — no fuzzy library needed. Add one later if substring matching proves insufficient.

### Approximate File Additions / Edits

| File | Change |
|------|--------|
| `frontend/app/store/command-registry.ts` | New |
| `frontend/app/modals/command-palette.tsx` | New |
| `frontend/app/modals/command-palette.scss` | New |
| `frontend/app/store/keymodel.ts` | Add `Ctrl:p` binding |
| `frontend/app/modals/modalregistry.tsx` | Register palette |
| `frontend/cef-init.ts` | Call `registerDefaultCommands()` |
| `agentmux-cef/src/commands/palette.rs` | New |
| `agentmux-cef/src/commands/mod.rs` | Add `pub mod palette;` |
| `agentmux-cef/src/ipc.rs` | Add `"run_command"` arm |
