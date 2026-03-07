# Spec: JSONC Settings with Live Reload

**Status:** Draft
**Date:** 2026-03-06

## Problem

1. `settings.json` is plain JSON — users cannot add comments to document their config
2. When users edit `settings.json` in an external editor (VS Code, etc.), the app does not detect changes — requires full restart
3. The default `settings.json` is an empty `{}` — users have no idea what options exist

## Current State

### Settings File
- **Location:** `~/.agentmux/settings.json`
- **Created by:** Tauri command `ensure_settings_file` (`src-tauri/src/commands/platform.rs:59-75`)
- **Default content:** `{}\n` (empty object)
- **Parser:** `serde_json::from_str()` in `wconfig.rs:735` — plain JSON, no comment support

### Settings Button (UI)
- **Widget:** `action-widgets.tsx:37-45` — calls `ensure_settings_file`, then `openNativePath(path)`
- Opens the file in the OS default editor (e.g. VS Code, Notepad)

### Settings Loading
- **Startup:** `build_default_config()` reads file, merges with defaults
- **Delivery:** Backend sends `config` event over WebSocket on connect (`websocket.rs:75-96`)
- **Frontend:** `fullConfigAtom` / `settingsAtom` in `global.ts:105-108`, updated via `config` event handler at line 229-234

### File Watching
- **Not implemented.** `ConfigWatcher` (`wconfig.rs:640-682`) holds config in `RwLock<Arc<FullConfigType>>` but has no filesystem watcher. Comment says "deferred until integrated with Tauri event loop."

### Settings Schema
- **Rust struct:** `SettingsType` in `wconfig.rs:34-267` (all fields `Option<T>`)
- **TypeScript:** `SettingsType` in `frontend/types/gotypes.d.ts:784-859`
- **JSON Schema:** `schema/settings.json:1-238`

### Available Setting Keys (from schema)
| Prefix | Keys |
|--------|------|
| `app:*` | `globalhotkey`, `dismissarchitecturewarning`, `defaultnewblock`, `showoverlayblocknums` |
| `ai:*` | `preset`, `apitype`, `baseurl`, `apitoken`, `name`, `model`, `orgid`, `apiversion`, `maxtokens`, `timeoutms`, `proxyurl`, `fontsize`, `fixedfontsize` |
| `term:*` | `fontsize`, `fontfamily`, `theme`, `disablewebgl`, `localshellpath`, `localshellopts`, `scrollback`, `copyonselect`, `transparency`, `allowbracketedpaste`, `shiftenternewline` |
| `editor:*` | `fontsize`, `minimapenabled`, `wordwrap` |
| `window:*` | `transparent`, `zoom`, `opacity`, `tilegapsize` |
| `widget:*` | `showhelp` |
| `conn:*` | `wshenabled`, `askbeforewshinstall` |
| `autoupdate:*` | `enabled`, `installonquit`, `channel` |
| `telemetry:*` | `enabled` |
| `blockheader:*` | `showblockids` |
| `markdown:*` | `fontsize` |
| `preview:*` | `fontsize` |
| `tab:*` | `preset` |
| `cmd:*` | `env` (object) |

---

## Implementation Plan

### Task 1: JSONC Parsing (Rust)

**Goal:** Allow `//` and `/* */` comments in `settings.json`.

**Approach:** Strip comments before parsing, so the file remains `.json` (not `.jsonc`) for editor compatibility.

**Changes:**
- **`Cargo.toml`** (agentmuxsrv-rs): Add `json_comments` crate (lightweight comment stripper) or implement a simple `strip_json_comments()` function (~30 lines)
- **`wconfig.rs:read_config_file()`** (~line 735): Pipe file content through comment stripper before `serde_json::from_str()`
- **Write helper function:** `fn strip_json_comments(input: &str) -> String` — handles `//`, `/* */`, preserves strings

**Crate option:** `json_comments = "0.2"` — well-maintained, zero-dep, does exactly this.

### Task 2: Commented Default Settings Template

**Goal:** When `settings.json` is first created, populate it with a commented template showing all available options.

**Changes:**
- **`platform.rs:ensure_settings_file()`**: Instead of writing `{}`, write a template string
- **Template content:** Embed at compile time from `src-tauri/src/config/settings-template.jsonc` or inline

**Template format:**
```jsonc
// AgentMux Settings
// Edit this file and save — changes apply immediately.
// Uncomment a line to override the default value.
//
// Full reference: https://docs.agentmux.ai/settings
{
  // -- Terminal --
  // "term:fontsize":     12,
  // "term:fontfamily":   "JetBrains Mono",
  // "term:theme":        "default-dark",
  // "term:scrollback":   1000,
  // "term:copyonselect": true,
  // "term:transparency": 0.5,

  // -- AI / Claude --
  // "ai:model":      "claude-sonnet-4-6",
  // "ai:maxtokens":  4096,
  // "ai:timeoutms":  60000,
  // "ai:fontsize":   14,

  // -- Window --
  // "window:transparent":  false,
  // "window:opacity":      1.0,
  // "window:zoom":         1.0,
  // "window:tilegapsize":  3,

  // -- Editor --
  // "editor:fontsize":       14,
  // "editor:minimapenabled": false,
  // "editor:wordwrap":       true,

  // -- App --
  // "app:globalhotkey": "Ctrl+Shift+A",

  // -- Auto Update --
  // "autoupdate:enabled":      true,
  // "autoupdate:installonquit": true,
  // "autoupdate:channel":      "latest"
}
```

**Migration:** Existing users with `{}` keep their file. Only new installs get the template.

### Task 3: File Watcher (Rust Backend)

**Goal:** Detect saves to `settings.json` and push updated config to frontend in real time.

**Approach:** Use `notify` crate (already common in Rust ecosystem) to watch the config directory.

**Changes:**
- **`Cargo.toml`** (agentmuxsrv-rs): Add `notify = "7"` (cross-platform file watcher)
- **New module:** `agentmuxsrv-rs/src/backend/config_watcher_fs.rs`
  - Watches `~/.agentmux/` directory for `settings.json` modifications
  - On change: debounce 300ms, re-read file, strip comments, parse, update `ConfigWatcher`
  - Emit `config` event to all connected WebSocket clients (same event frontend already handles)
- **`main.rs`**: Spawn watcher thread after `ConfigWatcher` initialization
- **`ConfigWatcher`**: Add method `reload_from_disk(&self) -> Result<()>` that re-reads, parses, and broadcasts

**Event flow:**
```
User saves settings.json in VS Code
  -> notify crate detects modification
  -> debounce 300ms (coalesce rapid saves)
  -> strip_json_comments() + serde_json::from_str()
  -> ConfigWatcher.set_config(new_config)
  -> broadcast "config" event via WebSocket
  -> frontend fullConfigAtom updates
  -> UI re-renders with new settings
```

**Error handling:**
- If JSONC is invalid after save, log warning + keep previous config
- Optionally: send a `config_error` event so frontend can show a toast

### Task 4: Frontend — No Changes Needed (Verify Only)

The frontend already handles `config` events and updates atoms reactively:
- `global.ts:229-234` — config event handler updates `fullConfigAtom`
- Components that read `settingsAtom` will re-render automatically via Jotai

**Verify:** Settings-dependent UI (terminal font size, theme, transparency, etc.) actually reads from the atom reactively, not just at mount time.

---

## Dependency Summary

| Crate | Version | Purpose |
|-------|---------|---------|
| `json_comments` | 0.2 | Strip `//` and `/* */` from JSON before parsing |
| `notify` | 7.x | Cross-platform filesystem event watcher |

## File Change Summary

| File | Change |
|------|--------|
| `agentmuxsrv-rs/Cargo.toml` | Add `json_comments`, `notify` |
| `agentmuxsrv-rs/src/backend/wconfig.rs` | Strip comments in `read_config_file()` |
| `agentmuxsrv-rs/src/backend/config_watcher_fs.rs` | New — filesystem watcher module |
| `agentmuxsrv-rs/src/backend/mod.rs` | Register new module |
| `agentmuxsrv-rs/src/main.rs` | Spawn watcher thread |
| `src-tauri/src/commands/platform.rs` | Write commented template for new installs |
| `src-tauri/src/config/settings-template.jsonc` | New — default settings template (embedded) |

## Risks & Notes

- **`notify` on Windows:** Uses `ReadDirectoryChangesW` — reliable but may double-fire on some editors (hence debounce)
- **VS Code atomic saves:** VS Code writes to a temp file then renames — `notify` handles this via rename events
- **Large settings files:** Not a concern — settings.json will always be small
- **Backward compat:** Existing `{}` files continue to work. Comments are optional.
- **No `.jsonc` extension needed:** We strip comments ourselves, file stays `.json` for maximum editor compatibility. VS Code auto-detects `//` comments in `.json` files and handles them gracefully.
