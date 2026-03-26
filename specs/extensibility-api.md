# AgentMux Extensibility API Spec

**Author:** agent1
**Date:** 2026-03-19
**Status:** Draft

---

## Problem

AgentMux has internal extension surfaces (view registry, widget config, RPC commands, controllers) but no public API for third-party or user-driven extensibility. Every new view, widget, or integration requires modifying core source code and rebuilding. This limits adoption and ecosystem growth.

This spec defines a unified extensibility architecture that exposes all viable surfaces through documented, versioned APIs.

---

## Extensibility Surfaces

After auditing the codebase, there are **seven** distinct extension points worth exposing:

| # | Surface | What It Enables | Complexity |
|---|---------|----------------|------------|
| 1 | Custom Widget API | Third-party pane content (dashboards, tools, monitors) | High |
| 2 | Command Palette Extensions | User-defined commands and actions | Low |
| 3 | Keybinding Extensions | Custom keyboard shortcuts mapped to commands | Low |
| 4 | Theme API | Full visual theming beyond terminal colors | Medium |
| 5 | Shell Integration Hooks | Custom shell events, prompt decorations, status reporting | Medium |
| 6 | Backend Service Plugins | Rust-side services (data sources, protocols, integrations) | High |
| 7 | wsh CLI Extensions | Extend the shell integration binary with subcommands | Medium |

---

## 1. Custom Widget API

### Overview

Allow loading custom views into blocks without modifying core source. Widgets are sandboxed web content (iframe or webview) that communicate with AgentMux through a typed message API.

### Architecture

```
┌─────────────────────────────────────┐
│  AgentMux Block (BlockFull)         │
│  ┌───────────────────────────────┐  │
│  │  Widget iframe/webview        │  │
│  │  ┌─────────────────────────┐  │  │
│  │  │  User's HTML/JS/CSS     │  │  │
│  │  │  + agentmux-widget-sdk  │  │  │
│  │  └─────────────────────────┘  │  │
│  └──────────┬────────────────────┘  │
│             │ postMessage           │
│  ┌──────────▼────────────────────┐  │
│  │  WidgetBridge (host side)     │  │
│  │  - message validation         │  │
│  │  - permission enforcement     │  │
│  │  - RPC proxying               │  │
│  └───────────────────────────────┘  │
└─────────────────────────────────────┘
```

### Widget Manifest

Every widget ships a `widget.json` manifest:

```json
{
  "id": "com.example.my-widget",
  "name": "My Widget",
  "version": "1.0.0",
  "entry": "index.html",
  "icon": "icon.svg",
  "description": "A custom monitoring dashboard",
  "minAgentMuxVersion": "0.33.0",
  "permissions": [
    "meta:read",
    "meta:write",
    "event:subscribe",
    "rpc:BlockService.*",
    "filesystem:read"
  ],
  "defaultSize": { "width": 400, "height": 300 },
  "settings": {
    "refreshInterval": {
      "type": "number",
      "default": 5000,
      "label": "Refresh interval (ms)"
    }
  }
}
```

### Widget SDK (guest side)

```typescript
// @agentmux/widget-sdk

interface AgentMuxWidget {
  // Lifecycle
  onActivate(callback: () => void): void;
  onDeactivate(callback: () => void): void;
  onResize(callback: (width: number, height: number) => void): void;

  // Block metadata
  getMeta(): Promise<Record<string, any>>;
  setMeta(updates: Record<string, any>): Promise<void>;
  onMetaChange(callback: (meta: Record<string, any>) => void): void;

  // Settings (widget-specific, persisted)
  getSetting<T>(key: string): Promise<T>;
  setSetting<T>(key: string, value: T): Promise<void>;

  // Events
  subscribe(event: string, callback: (data: any) => void): Unsubscribe;
  publish(event: string, data: any): void;

  // RPC (gated by permissions)
  rpc<T>(service: string, method: string, ...args: any[]): Promise<T>;

  // UI integration
  setTitle(title: string): void;
  setIcon(icon: string): void;
  showNotification(message: string, level?: "info" | "warn" | "error"): void;
  requestAction(action: "close" | "magnify" | "split"): void;

  // Theme
  getTheme(): Promise<ThemeColors>;
  onThemeChange(callback: (theme: ThemeColors) => void): void;
}
```

### Widget Loading

Widgets are loaded from:
1. **Local directory:** `~/.agentmux/widgets/<widget-id>/`
2. **Built-in:** Bundled with the app in `resources/widgets/`

The `view` type for custom widgets is `"widget"`, with `meta["widget:id"]` specifying which widget to load.

### Widget Registration

A new `WidgetViewModel` handles all custom widgets:

```typescript
// Registered once in BlockRegistry
BlockRegistry.set("widget", WidgetViewModel);

// WidgetViewModel reads meta["widget:id"], loads manifest,
// creates sandboxed iframe pointing to widget's entry HTML
```

### Security Model

- Widgets run in sandboxed iframes (`sandbox="allow-scripts"`)
- All RPC calls proxied through WidgetBridge which checks permissions
- No direct filesystem access; only through gated RPC
- Widget origins are `null` (srcdoc) or local file:// scoped to widget dir
- CSP headers restrict network access to declared domains

---

## 2. Command Palette Extensions

### Overview

Users and widgets can register commands that appear in the launcher (command palette). Commands are actions with an ID, label, and handler.

### User-Defined Commands

Added to `~/.agentmux/config/commands.json`:

```json
{
  "commands": [
    {
      "id": "user:restart-backend",
      "label": "Restart Backend Service",
      "icon": "refresh-cw",
      "keybinding": "Ctrl+Shift+R",
      "action": {
        "type": "shell",
        "cmd": "systemctl restart myservice",
        "interactive": false
      }
    },
    {
      "id": "user:open-logs",
      "label": "Open Application Logs",
      "icon": "file-text",
      "action": {
        "type": "block",
        "blockdef": {
          "meta": {
            "view": "term",
            "controller": "cmd",
            "cmd": "tail -f /var/log/app.log"
          }
        }
      }
    },
    {
      "id": "user:toggle-dark",
      "label": "Toggle Dark Mode",
      "action": {
        "type": "config",
        "toggle": "app:theme",
        "values": ["dark", "light"]
      }
    }
  ]
}
```

### Command Action Types

| Type | Description |
|------|------------|
| `shell` | Run a shell command (in new block or background) |
| `block` | Create a block from a blockdef |
| `config` | Set or toggle a config value |
| `rpc` | Call an RPC service method |
| `url` | Open a URL in external browser |
| `widget` | Launch a custom widget |

### Widget-Registered Commands

Widgets can register commands via the SDK:

```typescript
agentmux.registerCommand({
  id: "mywidget:refresh-data",
  label: "Refresh Dashboard Data",
  icon: "refresh-cw",
  handler: () => fetchAndRender(),
});
```

These appear in the launcher prefixed with the widget name.

### Integration with Launcher

The existing `LauncherViewModel` already handles a command palette. Extension:
- Load `commands.json` at startup alongside widgets.json
- Merge user commands, widget commands, and built-in commands
- Filter/sort by frecency (frequency + recency)

---

## 3. Keybinding Extensions

### Overview

Custom keybindings mapped to command IDs. Layered system: defaults < user overrides < widget bindings.

### Configuration

In `~/.agentmux/config/keybindings.json`:

```json
{
  "keybindings": [
    {
      "key": "Ctrl+Shift+T",
      "command": "builtin:new-terminal",
      "when": "!inputFocused"
    },
    {
      "key": "Ctrl+K Ctrl+L",
      "command": "user:open-logs",
      "when": "always"
    },
    {
      "key": "Ctrl+Shift+D",
      "command": "mywidget:refresh-data"
    }
  ]
}
```

### Key Features

- **Chord support:** Multi-key sequences (`Ctrl+K Ctrl+L`)
- **When clauses:** Context-aware activation (`inputFocused`, `viewType == 'term'`, `blockActive`)
- **Conflict detection:** Warn on duplicate bindings, last-write-wins

### Implementation

- Intercept at the window level before views handle keys
- Resolve command ID from keybinding map
- Dispatch to command registry (same system as palette)

---

## 4. Theme API

### Overview

Extend theming beyond terminal colors to the full UI. CSS custom properties are already used internally; expose them as a stable contract.

### Theme File

In `~/.agentmux/themes/<theme-name>.json`:

```json
{
  "id": "nord-deep",
  "name": "Nord Deep",
  "type": "dark",
  "colors": {
    "bg.primary": "#2e3440",
    "bg.secondary": "#3b4252",
    "bg.tertiary": "#434c5e",
    "fg.primary": "#eceff4",
    "fg.secondary": "#d8dee9",
    "fg.muted": "#4c566a",
    "accent.primary": "#88c0d0",
    "accent.secondary": "#81a1c1",
    "accent.danger": "#bf616a",
    "accent.warning": "#ebcb8b",
    "accent.success": "#a3be8c",
    "border.primary": "#4c566a",
    "border.active": "#88c0d0",
    "header.bg": "#2e3440",
    "header.fg": "#eceff4",
    "tab.active.bg": "#3b4252",
    "tab.inactive.bg": "#2e3440",
    "widget.bg": "#3b4252",
    "widget.hover": "#434c5e",
    "scrollbar.thumb": "#4c566a",
    "scrollbar.track": "transparent"
  },
  "terminal": {
    "theme": "nord",
    "cursorStyle": "bar",
    "cursorBlink": true
  },
  "font": {
    "ui": "Inter, system-ui, sans-serif",
    "mono": "JetBrains Mono, monospace",
    "size": 13
  }
}
```

### Theme Loading

1. Scan `~/.agentmux/themes/` for `.json` files
2. Validate against theme schema
3. Present in Settings UI under Appearance
4. Apply by mapping `colors.*` keys to CSS custom properties
5. Terminal theme applied via `term:theme` setting override

### Live Preview

Themes apply instantly via CSS custom property updates. No restart needed.

---

## 5. Shell Integration Hooks

### Overview

The existing shell integration (`shellintegration.rs`) deploys scripts that inject OSC sequences. Extend this to allow users to define custom hooks that fire on shell events.

### Hook Points

| Hook | Fires When | Data Available |
|------|-----------|----------------|
| `prompt:before` | Before prompt renders | `cwd`, `last_exit_code`, `git_branch` |
| `prompt:after` | After command entered, before execution | `command_line` |
| `command:start` | Command begins executing | `command`, `pid` |
| `command:end` | Command finishes | `command`, `exit_code`, `duration_ms` |
| `directory:change` | Working directory changes | `old_cwd`, `new_cwd` |
| `session:start` | Shell session starts | `shell_type`, `pid` |
| `session:end` | Shell session ends | `exit_code` |

### Hook Configuration

In `~/.agentmux/config/hooks.json`:

```json
{
  "hooks": [
    {
      "event": "command:end",
      "condition": "exit_code != 0",
      "actions": [
        {
          "type": "notification",
          "message": "Command failed: {{command}} (exit {{exit_code}})"
        }
      ]
    },
    {
      "event": "directory:change",
      "actions": [
        {
          "type": "meta",
          "set": { "block:cwd": "{{new_cwd}}" }
        }
      ]
    },
    {
      "event": "command:end",
      "condition": "duration_ms > 30000",
      "actions": [
        {
          "type": "notification",
          "message": "Long command finished: {{command}} ({{duration_ms}}ms)",
          "level": "info"
        }
      ]
    }
  ]
}
```

### Hook Action Types

| Type | Description |
|------|------------|
| `notification` | Show a toast notification |
| `meta` | Update block metadata |
| `command` | Execute a command palette command |
| `event` | Publish an EventBus event |
| `rpc` | Call an RPC method |

### Implementation

Shell integration scripts already emit OSC sequences. Extend the OSC protocol:

```
OSC 16162 ; H ; <hook-name> ; <json-payload> BEL
```

The block controller parses these, evaluates conditions, and dispatches actions. Hooks are evaluated in the backend (Rust) for performance.

---

## 6. Backend Service Plugins

### Overview

Allow loading additional Rust services at runtime via shared libraries (`.so`/`.dll`/`.dylib`). These register as RPC service handlers alongside built-in services.

### Plugin Interface

```rust
// agentmux-plugin-sdk crate

pub trait AgentMuxPlugin: Send + Sync {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    fn on_load(&self, ctx: PluginContext) -> Result<()>;
    fn on_unload(&self) -> Result<()>;
    fn services(&self) -> Vec<ServiceRegistration>;
}

pub struct ServiceRegistration {
    pub service: String,
    pub method: String,
    pub handler: Box<dyn ServiceHandler>,
}

pub trait ServiceHandler: Send + Sync {
    fn handle(&self, ctx: &UIContext, args: Vec<serde_json::Value>)
        -> Result<serde_json::Value>;
}

pub struct PluginContext {
    pub data_dir: PathBuf,         // ~/.agentmux/plugins/<id>/data/
    pub config: serde_json::Value, // plugin-specific config
    pub rpc: RpcClient,            // call other services
    pub events: EventPublisher,    // publish events
}

// Plugin entry point macro
#[macro_export]
macro_rules! declare_plugin {
    ($plugin_type:ty, $constructor:expr) => {
        #[no_mangle]
        pub extern "C" fn _agentmux_plugin_create() -> *mut dyn AgentMuxPlugin {
            let plugin: Box<dyn AgentMuxPlugin> = Box::new($constructor);
            Box::into_raw(plugin)
        }
    };
}
```

### Plugin Loading

1. Scan `~/.agentmux/plugins/` for directories containing `plugin.toml`
2. Load shared library from plugin directory
3. Call `_agentmux_plugin_create()` to get plugin instance
4. Call `on_load()` with plugin context
5. Register services in the RPC dispatcher

### Plugin Manifest

`~/.agentmux/plugins/my-plugin/plugin.toml`:

```toml
[plugin]
id = "com.example.my-plugin"
name = "My Plugin"
version = "1.0.0"
library = "libmyplugin.so"
min_agentmux_version = "0.33.0"

[config]
api_key = ""
endpoint = "https://api.example.com"
```

### Security

- Plugins run in-process (not sandboxed) — this is intentional for performance
- Plugins are installed manually (no auto-download)
- Plugin loading requires explicit user opt-in in settings
- Service names must be prefixed with `plugin:<id>:` to avoid collisions

### Priority

This is the highest-complexity surface. Ship widget API and command extensions first. Backend plugins can follow once the RPC service interface is stabilized.

---

## 7. wsh CLI Extensions

### Overview

`wsh` is the shell integration binary that communicates with the AgentMux backend from the terminal. Allow extending it with custom subcommands.

### Extension Mechanism

wsh already has subcommands (`wsh getmeta`, `wsh setmeta`, `wsh run`, etc.). Extensions add new ones via scripts or binaries in a known path.

### Extension Discovery

`wsh` scans for extensions in:
1. `~/.agentmux/wsh-extensions/`
2. System path with `wsh-` prefix (e.g., `wsh-myext` becomes `wsh myext`)

### Extension Protocol

Extensions receive context via environment variables:

```bash
AGENTMUX_BLOCK_ID=<current-block-oid>
AGENTMUX_TAB_ID=<current-tab-oid>
AGENTMUX_WINDOW_ID=<current-window-oid>
AGENTMUX_SOCKET=<backend-socket-path>
AGENTMUX_AUTH_TOKEN=<auth-token>
```

Extensions can use these to call the RPC API directly (via the socket) or use `wsh rpc` as a helper:

```bash
#!/bin/bash
# ~/.agentmux/wsh-extensions/wsh-deploy
# Usage: wsh deploy <env>

ENV=${1:-dev}
echo "Deploying to $ENV..."
wsh setmeta "deploy:status" "deploying"
deploy run --env "$ENV"
EXIT_CODE=$?
wsh setmeta "deploy:status" "$([[ $EXIT_CODE -eq 0 ]] && echo 'success' || echo 'failed')"
wsh notify "Deploy to $ENV $([ $EXIT_CODE -eq 0 ] && echo 'succeeded' || echo 'failed')"
```

### Built-in Helpers for Extensions

```bash
wsh rpc <service> <method> [args...]   # Call any RPC method
wsh notify <message> [--level info]    # Show notification
wsh getmeta <key>                      # Read block meta
wsh setmeta <key> <value>             # Write block meta
wsh publish <event> [data]             # Publish event
```

---

## Implementation Phases

### Phase 1: Foundation (v0.33)

- [ ] Command Palette Extensions (`commands.json`)
- [ ] Keybinding Extensions (`keybindings.json`)
- [ ] Theme API (`themes/*.json` + CSS custom property contract)

**Rationale:** Low complexity, high user value, no security concerns. Establishes config-file-based extension pattern.

### Phase 2: Widget API (v0.34)

- [ ] Widget manifest and loading system
- [ ] WidgetViewModel + sandboxed iframe host
- [ ] WidgetBridge message protocol
- [ ] `@agentmux/widget-sdk` npm package
- [ ] Permission system for widget RPC access
- [ ] Widget settings UI in block header

**Rationale:** Core extensibility feature. Enables ecosystem growth.

### Phase 3: Shell & CLI (v0.35)

- [ ] Shell integration hooks (`hooks.json`)
- [ ] Extended OSC protocol for hook events
- [ ] wsh CLI extension discovery and dispatch
- [ ] `wsh rpc` helper command

**Rationale:** Terminal-focused users get programmable shell behavior. wsh extensions are low-risk (just exec).

### Phase 4: Backend Plugins (v0.36+)

- [ ] `agentmux-plugin-sdk` crate published
- [ ] Plugin loading infrastructure
- [ ] Plugin config and data directory management
- [ ] Plugin manager UI in settings

**Rationale:** Highest complexity, requires stable RPC interface. Deferred until internal APIs settle.

---

## Configuration Directory Layout

```
~/.agentmux/
├── config/
│   ├── settings.json          # existing
│   ├── widgets.json           # existing
│   ├── connections.json       # existing
│   ├── commands.json          # NEW - custom commands
│   ├── keybindings.json       # NEW - custom keybindings
│   └── hooks.json             # NEW - shell hooks
├── themes/
│   ├── nord-deep.json         # NEW - custom themes
│   └── solarized.json
├── widgets/
│   └── com.example.dashboard/
│       ├── widget.json        # manifest
│       ├── index.html         # entry point
│       └── assets/
├── plugins/
│   └── com.example.myplugin/
│       ├── plugin.toml        # manifest
│       ├── libmyplugin.so     # shared library
│       └── data/              # plugin data dir
└── wsh-extensions/
    ├── wsh-deploy             # custom wsh subcommand
    └── wsh-status
```

---

## Non-Goals

- **Marketplace/registry:** No centralized widget store. Distribution is manual or via git. A marketplace is a separate initiative.
- **Hot-reloading backend plugins:** Plugins load at startup only. Restart required for changes.
- **Cross-widget communication:** Widgets communicate through the EventBus, not directly to each other. No peer-to-peer widget messaging.
- **Modifying core UI chrome:** Extensions cannot replace the tab bar, window frame, or layout system. They operate within blocks.

---

## Open Questions

1. **Widget sandboxing depth:** Should widgets get `allow-same-origin` for local storage? This weakens the sandbox but enables stateful widgets without relying on meta storage.

2. **Plugin ABI stability:** How do we version the plugin SDK to avoid breaking changes? Semantic versioning on the SDK crate, but what's the minimum compatibility window?

3. **Theme inheritance:** Should themes be able to extend a base theme (dark/light) and override specific tokens? Or must every theme be complete?

4. **Command palette discoverability:** How do users find available commands from widgets/plugins? Fuzzy search alone, or also categorized browsing?
