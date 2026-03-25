# Spec: Settings Widget

**Branch:** `agent2/settings-widget`
**Status:** Draft
**Date:** 2026-03-06

---

## Overview

Add a **Settings** widget button to the widget bar (between Help and DevTools) that opens `~/.agentmux/config/settings.json` in the user's **default external text editor** via the OS. AgentMux no longer has a built-in editor (Monaco was removed), so we use Tauri's `openUrl`/`openNativePath` to hand off to the system.

## Motivation

Currently, editing settings requires either:
1. Manually navigating to `~/.agentmux/config/settings.json` and opening it
2. Using `wsh` CLI commands
3. Right-clicking the widget bar > "Edit widgets.json" (only for widgets, not settings)

There's no single-click way to access the main settings file. Adding a Settings widget provides direct access alongside Help and DevTools.

---

## Design

### Widget Bar Layout (Current vs Proposed)

```
Current:   [custom widgets...] [help] [devtools]
Proposed:  [custom widgets...] [help] [settings] [devtools]
```

### Click Behavior

Clicking the Settings widget opens `settings.json` in the **OS default editor** (e.g., VS Code, Notepad, vim) using Tauri's opener plugin:

```typescript
const path = `${getApi().getConfigDir()}/settings.json`;
getApi().openNativePath(path);
```

This uses the existing `openNativePath` API (`tauri-plugin-opener`) which delegates to the OS file association for `.json` files. The backend's config watcher auto-reloads changes when the file is saved externally.

### Why External Editor (Not In-App)

1. **No built-in editor** -- Monaco was removed; the `preview` view is read-only file preview, not an editor
2. **User's preferred editor** -- Opens in whatever the user has configured for `.json` (VS Code, Sublime, etc.)
3. **Backend auto-reloads** -- `wconfig.rs` watches config files and hot-reloads on save
4. **Consistent with devtools** -- The devtools widget also has special handling (not a block), so settings can follow the same pattern
5. **Zero new dependencies** -- Uses existing `openNativePath` / `tauri-plugin-opener`

### Special Handling in handleWidgetSelect

Like `devtools` which special-cases `getApi().toggleDevtools()` instead of creating a block, `settings` will special-case `getApi().openNativePath()` instead of creating a block:

```typescript
async function handleWidgetSelect(widget: WidgetConfigType) {
    if (widget.blockdef?.meta?.view === "devtools") {
        getApi().toggleDevtools();
        return;
    }
    if (widget.blockdef?.meta?.view === "settings") {
        const path = `${getApi().getConfigDir()}/settings.json`;
        getApi().openNativePath(path);
        return;
    }
    const blockDef = widget.blockdef;
    createBlock(blockDef, widget.magnified);
}
```

---

## Implementation

### Files Changed

| File | Change |
|------|--------|
| `frontend/app/tab/widgetbar.tsx` | Add `settingsWidget`, special-case in `handleWidgetSelect`, render between help and devtools |

That's it -- one file.

### 1. `widgetbar.tsx` -- handleWidgetSelect

Add settings special case after the existing devtools case (line ~27):

```typescript
async function handleWidgetSelect(widget: WidgetConfigType) {
    // Special handling for devtools widget
    if (widget.blockdef?.meta?.view === "devtools") {
        getApi().toggleDevtools();
        return;
    }
    // Special handling for settings widget -- open in external editor
    if (widget.blockdef?.meta?.view === "settings") {
        const path = `${getApi().getConfigDir()}/settings.json`;
        getApi().openNativePath(path);
        return;
    }
    const blockDef = widget.blockdef;
    createBlock(blockDef, widget.magnified);
}
```

### 2. `widgetbar.tsx` -- Widget Definition

Add inside `WidgetBar` component (after `helpWidget`, before `devToolsWidget`):

```typescript
const settingsWidget: WidgetConfigType = {
    icon: "cog",
    label: "settings",
    description: "Open Settings (external editor)",
    blockdef: {
        meta: {
            view: "settings",
        },
    },
};
```

### 3. `widgetbar.tsx` -- Render

Update JSX to include settings between help and devtools:

```tsx
{showHelp && <HorizontalWidget key="help" widget={helpWidget} />}
<HorizontalWidget key="settings" widget={settingsWidget} />
<HorizontalWidget key="devtools" widget={devToolsWidget} />
```

---

## API Reference

### openNativePath

```
Frontend:  getApi().openNativePath(filePath)
Impl:      frontend/util/tauri-api.ts:171  ->  openUrl(filePath)
Plugin:    tauri-plugin-opener (Cargo.toml: tauri-plugin-opener = "=2.5")
Capability: src-tauri/capabilities/default.json includes "opener:default"
```

Opens a file path with the OS default application. On Windows this is `ShellExecuteW`, on macOS `open`, on Linux `xdg-open`.

### Config Directory

```
Function:  getApi().getConfigDir()
Returns:   ~/.agentmux/config  (or AGENTMUX_CONFIG_HOME env override)
Backend:   agentmuxsrv-rs/src/backend/wavebase.rs:get_wave_config_dir()
```

### Config Hot Reload

The backend's `ConfigWatcher` (wconfig.rs) monitors the config directory. When `settings.json` is modified externally, changes are picked up and pushed to the frontend via events. No restart needed.

---

## How Settings Work (Reference)

### Backend Config System

- **Config dir:** `~/.agentmux/config/` (override: `AGENTMUX_CONFIG_HOME`)
- **Settings file:** `~/.agentmux/config/settings.json`
- **Schema:** `schema/settings.json` (strict, `additionalProperties: false`)
- **Backend:** `agentmuxsrv-rs/src/backend/wconfig.rs` -- `SettingsType` struct, file constants, config watcher
- **RPC:** `SetConfigCommand` writes settings, `GetConfigCommand` reads them

### Current Settings Namespaces

| Prefix | Examples |
|--------|----------|
| `app:*` | `globalhotkey`, `defaultnewblock` |
| `ai:*` | `preset`, `model`, `apitoken`, `maxtokens` |
| `term:*` | `fontsize`, `fontfamily`, `theme`, `scrollback` |
| `editor:*` | `minimapenabled`, `wordwrap`, `fontsize` |
| `window:*` | `transparent`, `blur`, `opacity`, `zoom` |
| `widget:*` | `showhelp` |
| `autoupdate:*` | `enabled`, `channel`, `intervalms` |
| `telemetry:*` | `enabled` |
| `conn:*` | `askbeforewshinstall`, `wshenabled` |

### Widget Bar Architecture

- **File:** `frontend/app/tab/widgetbar.tsx`
- **Custom widgets:** Loaded from `fullConfigAtom` (sourced from `~/.agentmux/config/widgets.json`)
- **Built-in widgets:** `help`, `devtools` (and now `settings`) are hardcoded after custom widgets
- **Click handler:** `handleWidgetSelect()` -- special-cases `devtools` and `settings`, everything else calls `createBlock(blockDef)`

---

## Future Enhancements (Out of Scope)

- **Settings GUI view** -- A dedicated `"settings"` view type with forms, toggles, and dropdowns (registered in BlockRegistry)
- **Search/filter** -- Filter settings by namespace or keyword
- **Validation overlay** -- Show schema validation errors inline when file is malformed
- **"Edit settings.json" context menu** -- Add to widget bar right-click menu alongside "Edit widgets.json"

---

## Test Plan

- [ ] Settings widget appears between Help and DevTools in widget bar
- [ ] Clicking Settings opens `settings.json` in OS default editor
- [ ] File exists at `~/.agentmux/config/settings.json` (created if missing by backend)
- [ ] After editing and saving externally, backend picks up changes (e.g., change `term:fontsize`)
- [ ] Icon renders correctly (`cog` / gear icon)
- [ ] Tooltip shows "Open Settings (external editor)"
- [ ] Widget bar layout doesn't break on narrow windows
- [ ] `npx vite build --config vite.config.tauri.ts` succeeds
