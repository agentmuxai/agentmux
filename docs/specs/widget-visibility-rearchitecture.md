# Widget Visibility Re-Architecture
**Date:** 2026-03-11
**Status:** Proposed

---

## What We're Changing

1. **Remove "Edit widgets.json"** from all right-click menus
2. **Direct 1:1 binding between settings.json and the header buttons** — right-click toggles write to settings.json; settings.json changes update the buttons. No separate state, no priority system, no locks.

---

## Design Principle

> The right-click menu is just a UI for editing widget visibility in settings.json.
> Change one, the other changes. That's it.

No concept of "managed by settings.json" or "user-configurable". Every widget's visibility has exactly one home: `settings.json`. The right-click menu reads it and writes it.

---

## Current State (Problems)

### Two menus, both exposing internal details

**Tab bar right-click** (`base-menus.ts → createTabBarMenu`):
- Widget checkboxes read from `widget["display:hidden"]` — stored in the widget definition, not settings
- "Edit widgets.json" item — exposes internal file, confusing

**Action widgets bar right-click** (`action-widgets.tsx`):
- "Edit widgets.json" item — same problem
- `widget:showhelp` and `widget:icononly` — already in settings.json ✓

### Per-widget visibility is not persisted

`display:hidden` on widget objects lives in the in-memory config derived from the embedded `widgets.json`. Toggling it via `SetConfigCommand` updates memory but **does not write to settings.json**. Visibility resets on every restart.

### `widget:showhelp` is an inconsistent special case

The help widget has its own global key (`widget:showhelp`) rather than following the same pattern as other widgets. This means the help widget behaves differently from every other widget in the menu.

---

## Proposed Architecture

### Single visibility key pattern for all widgets

All per-widget visibility lives in `settings.json` under a consistent key:

```json
{
  "widget:hidden@defwidget@terminal": false,
  "widget:hidden@defwidget@sysinfo": true,
  "widget:hidden@defwidget@agent": false,
  "widget:hidden@defwidget@help": false,
  "widget:icononly": false
}
```

Pattern: `"widget:hidden@<widgetKey>"` → `bool`

**Absence = visible by default** (falls back to `display:hidden` from widget definition, which defaults to `false`).

### Migrate `widget:showhelp` → `widget:hidden@defwidget@help`

The help widget becomes a first-class widget in `widgets.json` with key `defwidget@help`. Its visibility follows the same pattern. `widget:showhelp` is deprecated and removed.

### Right-click menu — one menu, one behaviour

Rebuild `createTabBarMenu` to show:

```
[For each widget in fullConfig.widgets, sorted by display:order]
  ☑/☐  <label>   ← checkbox reads/writes "widget:hidden@<key>" in settings.json

[separator]
☑/☐  Icon only   ← reads/writes "widget:icononly" in settings.json
```

Every checkbox directly calls `SetConfigCommand({ "widget:hidden@key": !currentValue })`.
Backend writes it to `settings.json`. Frontend receives the config broadcast and re-renders.

No lock icons. No priority tiers. No special cases.

### Action widgets bar right-click — consolidate or remove

With the tab bar menu now handling all toggles, the action widgets bar right-click can be:

**Option A (recommended):** Remove it entirely — all widget configuration is in the tab bar menu.

**Option B:** Keep it but strip "Edit widgets.json" and the now-redundant `widget:showhelp` toggle. Only keep `widget:icononly` if that's more discoverable from the icon bar.

### Widget rendering

`action-widgets.tsx` replaces the `widget["display:hidden"]` read with a settings.json lookup:

```typescript
function isWidgetHidden(fullConfig: FullConfigType, widgetKey: string): boolean {
    const key = `widget:hidden@${widgetKey}`;
    if (key in (fullConfig.settings ?? {})) {
        return fullConfig.settings[key] as boolean;
    }
    return fullConfig.widgets?.[widgetKey]?.["display:hidden"] ?? false;
}
```

---

## Data Flow (After)

```
User right-clicks header
  → menu shows checkbox state from fullConfig.settings["widget:hidden@key"]

User toggles a checkbox
  → SetConfigCommand({ "widget:hidden@defwidget@sysinfo": true })
  → backend writes to settings.json
  → fs watcher detects change → broadcasts "config" event
  → frontend atoms.fullConfigAtom updates
  → ActionWidgets re-renders with widget hidden

User edits settings.json directly
  → same fs watcher → same broadcast → same re-render
```

Round-trip time: ~50ms (fs watcher debounce).

---

## File Changes

| File | Change |
|------|--------|
| `frontend/app/menu/base-menus.ts` | Rebuild `createWidgetsMenu`: read visibility from `settings["widget:hidden@key"]`; write same key on toggle; remove "Edit widgets.json" |
| `frontend/app/window/action-widgets.tsx` | Replace `widget["display:hidden"]` with `isWidgetHidden()`; remove "Edit widgets.json" and `widget:showhelp` from context menu |
| `agentmuxsrv-rs/src/config/widgets.json` | Add `defwidget@help` entry (promotes help widget to first-class) |
| `agentmuxsrv-rs/src/backend/wconfig.rs` | Accept `widget:hidden@*` keys in settings schema |
| `frontend/types/gotypes.d.ts` | No change needed (settings is `Record<string, any>`) |

---

## Backend Gap: `SetConfigCommand` is Not Implemented

**Verified:** `SetConfigCommand` (RPC `"setconfig"`) is defined in `rpc_types.rs:187` and called by the frontend, but **no handler is registered** in `websocket.rs → register_handlers()`. The backend returns `"unknown command: setconfig"` and does nothing.

This means:
- The tab bar widget toggles (calling `SetConfigCommand`) silently fail today
- `widget:showhelp` and `widget:icononly` toggles also silently fail
- Widget visibility has never actually been persisted via the UI

**Verified:** The fs watcher (`config_watcher_fs.rs`) correctly detects writes to `settings.json` (any write, including programmatic), debounces 300ms, reloads the file, and broadcasts a `"config"` event to all WebSocket clients. This path works.

---

## Required Backend Work

### Implement the `setconfig` RPC handler

Add to `websocket.rs → register_handlers()`:

```rust
handlers.register(COMMAND_SET_CONFIG, |broker, wstore, event_bus, req| {
    let settings: SettingsType = serde_json::from_value(req.data.clone())?;
    // Merge incoming keys into settings.json on disk
    let settings_path = get_settings_path();
    let mut current = read_settings_from_disk(&settings_path);
    current.extend(settings);
    write_settings_to_disk(&settings_path, &current)?;
    // fs watcher picks up the write and broadcasts within ~300ms
    // No explicit broadcast needed here — the watcher closes the loop
    Ok(serde_json::Value::Null)
});
```

The fs watcher automatically handles the broadcast after the write — no manual event emission needed.

### Key files to modify
- `agentmuxsrv-rs/src/server/websocket.rs` — register the handler
- `agentmuxsrv-rs/src/backend/wconfig.rs` — add `write_settings_to_disk()` helper (reads current file, merges, writes back)

---

## Complete Data Flow (After)

```
User toggles widget checkbox in right-click menu
  → SetConfigCommand({ "widget:hidden@defwidget@sysinfo": true })
  → WS RPC "setconfig" → new handler in websocket.rs
  → merge key into settings.json on disk
  → fs watcher detects write (within ~300ms)
  → reload_and_broadcast() fires
  → "config" event broadcast to all WS clients
  → atoms.fullConfigAtom updates
  → ActionWidgets re-renders, widget hidden
```

Round-trip: ~300ms (fs watcher debounce). Acceptable for a toggle action.

---

## Migration

No user data migration needed. `display:hidden` values were never persisted (all toggles silently failed), so nothing to carry over. On first launch after the change, all widgets appear at their default visibility — same as today.
