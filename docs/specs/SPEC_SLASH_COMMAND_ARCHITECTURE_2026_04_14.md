# SPEC — Slash Command Architecture

**Date:** 2026-04-14
**Status:** Draft
**Owner:** AgentA
**Scope:** `frontend/app/view/agent/` — the composer's slash command surface
**Related:** PR #378 (minimal `/model` /`/effort` /`/permission-mode` dispatcher — the mechanism proof this spec grows into a real architecture)

---

## 1. Why a spec, not just more case statements

PR #378 shipped a ~80-line switch in `useAgentCommands.sendMessage` that intercepts `/model`, `/effort`, `/permission-mode`, `/bypass`, `/plan`, `/runtime` and maps them to the existing runtime-config path. It works, but four observations make the switch a dead end:

1. **Five commands today, ten more tomorrow.** Claude Code has ~20 slash commands (`/model`, `/memory`, `/hooks`, `/mcp`, `/compact`, `/cost`, `/status`, `/doctor`, `/config`, `/bug`, `/release-notes`, `/help`, `/clear`, `/login`, ...). Codex has its own vocabulary. Gemini CLI ships a third. Some commands map to the same action (`/clear` = reset conversation), some don't exist in one CLI and do in another.
2. **Args want structure.** `/model <name>` needs a picker when invoked bare. `/memory` wants autocomplete over recent memory files. `/cost` needs no arg at all. A string dispatcher doesn't know any of that.
3. **Users want discovery.** `/help` should list *all* commands available in the current pane, not a hardcoded subset. Pressing `/` should show completions as the user types.
4. **The CLI isn't the only command source.** Built-in AgentMux commands (`/clear`, `/runtime`, `/login`) are orthogonal to the CLI's own vocabulary. Forge-defined agents might want their own commands. A command registry lets all three sources coexist.

The right abstraction is: **commands are data, not code**. The dispatcher, picker, autocomplete, and help panel all consume the same registry.

## 2. What doesn't exist yet

To ground the design, here's what AgentMux currently has and lacks:

| Feature | Today (0.33.148) | After this spec |
|---|---|---|
| Client-side intercepting dispatcher | ✓ PR #378 switch | Registry lookup |
| Runtime-config commands (`/model`, `/effort`, `/permission-mode`) | ✓ PR #378 | Same commands, registered via registry |
| Arg picker when `/model` is typed bare | ✗ logs a warning | Inline picker overlay |
| Autocomplete while typing `/mo...` | ✗ | Dropdown above composer |
| `/help` command listing all available commands | ✗ falls through to CLI | Help panel renders from registry |
| Provider-scoped commands (e.g. Claude vs Codex) | ✗ everything is Claude | Each `ProviderDefinition` contributes commands |
| Agent-specific commands (forge-defined) | ✗ | Registry accepts per-agent additions (deferred to a later spec) |
| Error for commands that exist in interactive CLI but not stream-json | ✗ silently becomes a user message | Registry entry with "not-in-stream-json" handler |

Nothing below creates a new RPC, a new data store, or a new backend concept. Everything is a frontend refactor of the dispatch path plus three new presentation surfaces.

## 3. Non-goals

- **Backend slash command handling.** The sidecar never sees a slash command; all dispatch is in the frontend.
- **Custom commands in block meta.** A block storing its own command list is an interesting future feature but out of scope. Current providers + static global commands cover everything we need.
- **Rewriting `/login` or `/clear`.** Those work today. They just get registered into the new registry and keep their current behavior.
- **Command aliasing at the user-settings level.** Could come later via a settings panel; not part of this architecture spec.
- **Slash command history / up-arrow replay.** Separate feature.
- **Support for slash commands inside tool arguments.** Only applies to composer input.

## 4. Design

### 4.1 The `SlashCommand` shape

```ts
// frontend/app/view/agent/commands/types.ts — new file
export type SlashCommandCategory = "runtime" | "session" | "auth" | "query" | "system" | "help";

export interface SlashCommandContext {
    /** Block id this command runs against. */
    blockId: string;
    /** Current provider definition, if the pane is in presentation mode. */
    provider: () => ProviderDefinition | undefined;
    /** Block meta accessor. */
    block: () => Block | undefined;
    /** Document atom pair for commands that mutate the conversation. */
    documentAtom: SignalPair<DocumentNode[]>;
    /** Log a system message to the launch-log sink. */
    log: LogFn;
    /** Set the OAuth URL for /login. */
    setAuthUrl: (url: string | null) => void;
    /** Open the inline arg picker. Returns a promise that resolves with the selected value or rejects if dismissed. */
    openPicker: (spec: SlashPickerSpec) => Promise<string>;
    /** Access the registry so commands like /help can iterate. */
    registry: SlashCommandRegistry;
}

export type SlashArg =
    | { kind: "none" }
    | { kind: "enum"; choices: SlashChoice[]; required: boolean; defaultLabel?: string }
    | { kind: "freeform"; placeholder: string; required: boolean }
    | { kind: "dynamic"; placeholder: string; completions: (ctx: SlashCommandContext) => Promise<SlashChoice[]> };

export interface SlashChoice {
    value: string;
    label: string;
    description?: string;
    current?: boolean; // if true, render as the currently-active option
}

export interface SlashCommand {
    name: string;
    aliases?: string[];
    category: SlashCommandCategory;
    description: string;
    /** Longer help text shown in /help panel or on hover. */
    longDescription?: string;
    arg: SlashArg;
    /**
     * Called after arg validation. If `arg.required` and no arg provided,
     * the dispatcher opens a picker first and invokes handler with the
     * selected value.
     */
    handler: (ctx: SlashCommandContext, arg: string) => Promise<SlashResult>;
    /**
     * Optional: where the command is available. Undefined = global. A
     * provider match scopes to that CLI; `"any-agent"` scopes to
     * "pane has an agentId set"; `"picker-only"` scopes to the agent
     * picker screen.
     */
    availability?: "global" | "any-agent" | "picker-only" | { provider: string };
}

export type SlashResult =
    | { kind: "ok"; message?: string }
    | { kind: "error"; message: string }
    | { kind: "passthrough" }; // fall through to AgentInputCommand
```

### 4.2 The registry

```ts
// frontend/app/view/agent/commands/registry.ts — new file
export class SlashCommandRegistry {
    private commands = new Map<string, SlashCommand>();

    register(cmd: SlashCommand): void {
        this.commands.set(cmd.name, cmd);
        for (const alias of cmd.aliases ?? []) {
            this.commands.set(alias, cmd);
        }
    }

    lookup(name: string): SlashCommand | undefined {
        return this.commands.get(name.toLowerCase());
    }

    /** List commands available in the given context (applies availability filter). */
    list(ctx: SlashCommandContext): SlashCommand[] { ... }

    /** Autocomplete — returns commands whose name or aliases start with the prefix. */
    completions(prefix: string, ctx: SlashCommandContext): SlashCommand[] { ... }
}

/**
 * Build the registry with all commands for the current pane. Called from
 * useAgentCommands, memoized. Re-run only when the provider changes.
 */
export function buildRegistry(provider: ProviderDefinition | undefined): SlashCommandRegistry {
    const registry = new SlashCommandRegistry();

    // Global commands (always available)
    registerGlobalCommands(registry);

    // Provider-scoped commands
    if (provider) {
        const providerCommands = SLASH_COMMANDS_BY_PROVIDER[provider.id] ?? [];
        for (const cmd of providerCommands) {
            registry.register(cmd);
        }
    }

    return registry;
}
```

### 4.3 Command files layout

```
frontend/app/view/agent/commands/
├── types.ts                # SlashCommand, SlashArg, SlashContext, SlashResult
├── registry.ts             # SlashCommandRegistry + buildRegistry
├── global/
│   ├── clear.ts            # /clear
│   ├── help.ts             # /help
│   ├── runtime.ts          # /runtime, /model, /effort, /permission-mode, /bypass, /plan
│   ├── login.ts            # /login
│   └── index.ts            # registerGlobalCommands
├── providers/
│   ├── claude.ts           # /cost, /status, /doctor, /memory, /hooks, /mcp, /compact, /config
│   ├── codex.ts            # (placeholder for when we wire Codex commands)
│   ├── gemini.ts           # (placeholder)
│   └── index.ts            # SLASH_COMMANDS_BY_PROVIDER
├── pickers.ts              # SlashPickerSpec, picker resolver (called by dispatcher)
└── dispatch.ts             # dispatchSlashCommand — the main entry point
```

### 4.4 The dispatcher

```ts
// frontend/app/view/agent/commands/dispatch.ts — new file
export async function dispatchSlashCommand(
    input: string,
    ctx: SlashCommandContext,
): Promise<SlashResult> {
    const [name, arg] = parseSlashCommand(input);
    const cmd = ctx.registry.lookup(name);
    if (!cmd) {
        // Unknown slash command — fall through to AgentInputCommand (current behavior).
        return { kind: "passthrough" };
    }

    // Validate arg
    if (cmd.arg.kind === "enum") {
        if (arg === "" && cmd.arg.required) {
            // Bare command → open picker
            try {
                const picked = await ctx.openPicker({
                    title: `Select ${cmd.name}`,
                    choices: cmd.arg.choices,
                });
                return await cmd.handler(ctx, picked);
            } catch {
                return { kind: "ok", message: "cancelled" };
            }
        }
        const match = cmd.arg.choices.find(
            (c) => c.value.toLowerCase() === arg.toLowerCase()
        );
        if (!match && cmd.arg.required) {
            return {
                kind: "error",
                message: `/${cmd.name}: unknown value '${arg}'. Try: ${cmd.arg.choices.map((c) => c.value).join(" | ")}`,
            };
        }
        return await cmd.handler(ctx, match?.value ?? arg);
    }

    if (cmd.arg.kind === "dynamic") {
        if (arg === "" && cmd.arg.required) {
            const choices = await cmd.arg.completions(ctx);
            try {
                const picked = await ctx.openPicker({
                    title: `Select ${cmd.name}`,
                    choices,
                });
                return await cmd.handler(ctx, picked);
            } catch {
                return { kind: "ok", message: "cancelled" };
            }
        }
        return await cmd.handler(ctx, arg);
    }

    if (cmd.arg.kind === "freeform") {
        if (arg === "" && cmd.arg.required) {
            return {
                kind: "error",
                message: `/${cmd.name} requires an argument: ${cmd.arg.placeholder}`,
            };
        }
        return await cmd.handler(ctx, arg);
    }

    // kind === "none"
    return await cmd.handler(ctx, "");
}
```

### 4.5 The inline picker UI

`frontend/app/view/agent/components/SlashCommandPicker.tsx` — a new component mounted above `AgentFooter` in `AgentPresentationView`. Visible only when the picker signal is non-null.

Visual:

```
┌─ Select model ────────────────────────────────┐
│  ◉  Opus          Claude Opus 4.6           │  ← current model highlighted
│     Sonnet        Claude Sonnet              │
│     Haiku         Claude Haiku               │
│     Default       Provider default           │
│  ─────────────                                │
│  Esc to cancel · ↵ to select                  │
└───────────────────────────────────────────────┘
```

- Mouse click or keyboard (↑/↓ + Enter) picks an option.
- Esc dismisses → dispatcher's `openPicker` Promise rejects → command handler returns `cancelled`.
- Single-keystroke filter: typing a letter jumps to the first matching option (same pattern as the existing AgentSearchBar).
- Uses the existing picker state signal pattern from `useScrollToNode` — a signal owned by the hook, set via `openPicker`, consumed by the component via Accessor prop.

### 4.6 Autocomplete dropdown

When the user types `/` in `AgentFooter`, a dropdown appears showing commands whose name or alias starts with the typed prefix. Filters as the user types.

Implementation:
- New signal in the composer hook (`useSlashAutocomplete`) tracks the current composer value.
- If value starts with `/` and has no space, look up completions in the registry.
- Render as a floating list above the textarea using the same positioning pattern as the ToolBlock portal (PR #367).
- Tab / Enter accepts the top completion; keeps typing to narrow.
- Esc dismisses.

The autocomplete consumes the **same** registry as the dispatcher, so adding a command automatically shows up in autocomplete.

### 4.7 The `/help` panel

Registered as a global command in `global/help.ts`. When invoked:
1. Calls `ctx.registry.list(ctx)` to get all currently-available commands
2. Groups by `category` (runtime, session, auth, query, system, help)
3. Renders a new `SlashHelpPanel` component as an overlay
4. Each command entry shows: name, aliases, description, arg hint
5. Clicking a row either runs the command (for arg-less ones) or pre-fills the composer with `/<name> `

Keyboard: `?` or `/help` opens; Esc closes.

## 5. Migration path

Bridging PR #378 (current) to this architecture without a flag-day rewrite:

### Step 1 — Wire the registry, keep the switch

- Create `commands/` directory with `types.ts`, `registry.ts`, empty stubs for `global/` and `providers/`.
- Register exactly the six commands PR #378 handles into `global/runtime.ts`.
- Replace the switch in `useAgentCommands.sendMessage` with `dispatchSlashCommand(input, ctx)`.
- Behavior: identical to PR #378. The switch is just hidden behind a registry lookup.

### Step 2 — Inline picker

- Add `SlashCommandPicker.tsx` component.
- Wire the picker signal through `UseAgentCommands` options (same pattern as `onSent`).
- Change `/model`, `/effort`, `/permission-mode` commands to `arg: { kind: "enum", required: true, ... }`.
- Bare `/model` now opens the picker instead of logging a warning.

### Step 3 — Autocomplete

- Add `useSlashAutocomplete` hook.
- Wire into `AgentFooter` via a new `onAutocomplete` prop.
- Render the dropdown component above the composer.
- Tab / Enter accepts the top match.

### Step 4 — `/help` panel

- Add `global/help.ts` registering `/help`.
- Add `SlashHelpPanel` component.
- Wire the overlay into `AgentPresentationView` controlled by a signal from `useAgentCommands`.

### Step 5 — Claude provider commands

- Populate `providers/claude.ts` with:
  - `/cost` — shells out to `claude -p --output-format json /cost` via a new host-level RPC, parses the result, logs it
  - `/status` — same pattern
  - `/doctor` — same pattern
  - `/memory` — opens `~/.claude/CLAUDE.md` in the system editor via the existing CEF host API
  - `/hooks`, `/mcp`, `/config` — same (open the relevant files)
  - `/compact` — **not supported in stream-json mode**; registered with a handler that explains why and suggests alternatives
  - `/bug`, `/release-notes`, `/help` (CLI-level) — open external URLs
- Each is a separate commit so they land incrementally.

### Step 6 — Codex / Gemini

- When someone wires Codex or Gemini as a first-class provider, they populate `providers/codex.ts` / `providers/gemini.ts`.
- The dispatcher, picker, autocomplete, and help all work automatically.
- No changes to the core.

## 6. Example: `/model` command registration

```ts
// frontend/app/view/agent/commands/global/runtime.ts
import { SlashCommand } from "../types";
import { getRuntimeConfig } from "../../buildRuntimeArgs";

const MODEL_CHOICES = [
    { value: "opus", label: "Opus", description: "Claude Opus — highest quality" },
    { value: "sonnet", label: "Sonnet", description: "Claude Sonnet — balanced" },
    { value: "haiku", label: "Haiku", description: "Claude Haiku — fastest" },
    { value: "default", label: "Default", description: "Provider default" },
];

export const modelCommand: SlashCommand = {
    name: "model",
    category: "runtime",
    description: "Change the active model",
    longDescription:
        "Sets the model for the next turn. Does not restart the session or affect " +
        "history. Maps to --model <choice> in the next CLI invocation.",
    arg: {
        kind: "enum",
        required: true,
        choices: MODEL_CHOICES,
    },
    availability: "any-agent",
    handler: async (ctx, arg) => {
        const current = getRuntimeConfig(ctx.block()?.meta);
        const model = arg === "default" ? null : arg as ModelChoice;
        const updated = { ...current, model };
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", ctx.blockId),
                meta: { "agent:runtime": updated },
            });
            return { kind: "ok", message: `model set to ${arg} (applies to next turn)` };
        } catch (err: any) {
            return { kind: "error", message: `failed to update runtime: ${err?.message ?? err}` };
        }
    },
};
```

Notice:
- The **picker choices are part of the command definition**, so the picker UI has everything it needs without touching the dispatcher.
- The **handler is pure async** and returns a `SlashResult`. The dispatcher logs the message.
- Error handling is centralized — the handler never logs directly, just returns `{ kind: "error", message }`.
- **Availability** scopes it to panes with an agent loaded, so the agent picker screen doesn't offer `/model`.

## 7. Estimated cost per step

| Step | Time | Risk |
|---|---:|---|
| 1. Registry + migrate PR #378 | 1h | Low — behavior identical, just refactored |
| 2. Inline picker + enum arg support | 1.5h | Low — new component, localized |
| 3. Autocomplete dropdown | 2h | Medium — composer focus/keyboard conflicts |
| 4. `/help` panel | 1h | Low |
| 5. Claude provider commands (6 commands, one PR each) | 4–6h | Medium — some need new RPCs for CLI shelling |
| 6. Codex / Gemini registration | 30 min each when those providers land | Low |

**Total to steps 1–4:** ~5.5 hours. Gets the whole frontend architecture in place with no behavior regressions. Step 5 can be spread over days as useful.

## 8. Open questions

1. **Where does the shell-out to `claude -p /cost` run?** Options: (a) new CEF host RPC that spawns the CLI and returns the JSON result; (b) frontend-only via `fetch` to the sidecar which then spawns. (a) is simpler because the host already has `runCliLogin` and can grow a sibling `runCliCommand`.
2. **Does `/compact` have *any* meaningful representation in stream-json mode?** The CLI's `/compact` is an interactive session action. Probably needs to be "not supported, use a new session via /clear + reseed" as a helpful error.
3. **How do we handle commands that need to re-spawn the CLI?** `/model` takes effect on the next turn without restart — that's the happy case. `/config` that changes the base args would require a resync. For now, all restart-requiring commands should just log "restart the pane to apply" and not auto-restart.
4. **Autocomplete conflict with path completion?** If the user types `/home/foo`, autocomplete shouldn't fire. Fix: only trigger if the value starts with `/` **and** doesn't contain a space AND there's no trailing slash in the first token. Keep it a fast regex check.
5. **Command help as Markdown?** `longDescription` could be Markdown if we want to reuse `MarkdownBlock`. Open for a later call — text is fine for now.

## 9. Success criteria

After steps 1–4 land:

- Typing `/model` alone shows a picker with Opus/Sonnet/Haiku/Default, marks the current one, Enter/Esc works.
- Typing `/m` shows autocomplete with `/memory`, `/model`, `/mcp` (or whatever is registered).
- Tab on autocomplete completes to the top match.
- `/help` opens a grouped list of every currently-available command.
- All six commands from PR #378 still behave exactly the same when given an explicit arg.
- Unknown slash commands still fall through to `AgentInputCommand` (no behavior regression).
- Adding a new command is one file create in `commands/global/` or `commands/providers/`, no touches to the dispatcher or picker or autocomplete.

After step 5 lands (full Claude provider coverage):

- Every Claude Code CLI slash command has a corresponding client-side handler or a helpful error explaining why stream-json mode doesn't support it.
- Multi-CLI support is one directory add (`commands/providers/codex.ts`) away, not a dispatcher change.

## 10. Out of scope

- Backend awareness of slash commands (they never cross the wire)
- Custom user-defined commands in settings
- Command history / up-arrow replay
- Inline markdown or JSX in command output (log lines stay plain text)
- Command aliases edited at runtime
- Slash commands in subagent panes (same dispatcher reuses transparently)
- Supporting commands in tool arguments or inside agent-generated messages (composer-only)

## 11. Appendix: Claude CLI slash command inventory

For reference when filling in `providers/claude.ts`. Based on Claude Code 2.x documentation; marked ✗ for anything that's interactive-only (won't work via our shell-out approach).

| Command | Category | Arg | Stream-json viable? | Notes |
|---|---|---|---|---|
| `/model` | runtime | enum | ✓ (via runtime config) | PR #378 mechanism |
| `/effort` | runtime | enum | ✓ (via runtime config) | PR #378 |
| `/permission-mode` | runtime | enum | ✓ (via runtime config) | PR #378 |
| `/bypass` | runtime | none | ✓ (via runtime config) | PR #378 |
| `/plan` | runtime | none | ✓ (via runtime config) | PR #378 |
| `/clear` | session | none | ✓ | Already handled; reset frontend doc |
| `/login` | auth | none | ✓ | Already handled |
| `/cost` | query | none | ✓ via shell-out | Spawns `claude /cost --output-format json` |
| `/status` | query | none | ✓ via shell-out | Same |
| `/doctor` | query | none | ✓ via shell-out | Same |
| `/help` | help | none | ✓ client-side | Reads registry |
| `/memory` | system | none | ✓ open file in editor | Points at `~/.claude/CLAUDE.md` |
| `/hooks` | system | none | ✓ open file in editor | Points at hooks file |
| `/mcp` | system | none | ✓ open file in editor | Points at MCP config |
| `/config` | system | none | ✓ open file in editor | Points at settings.json |
| `/compact` | session | none | ✗ | Requires live CLI session; offer "start new pane" instead |
| `/bug` | help | none | ✓ | Open issue URL |
| `/release-notes` | help | none | ✓ | Open GitHub releases URL |
| `/logout` | auth | none | ✓ via shell-out | `claude auth logout` |
| `/compact` | session | none | ✗ | Same as above |

Codex and Gemini inventories get their own appendix when those providers land.
