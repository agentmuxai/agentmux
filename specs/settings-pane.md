# Settings Pane Specification

**Date:** 2026-03-11
**Version:** 1.0
**Status:** Draft

## Overview

Replace the current "open settings.json in an editor" flow with a dedicated Settings pane that renders structured input fields. Changes to any field immediately update the UI (same reactivity as editing settings.json today). An "Open Settings JSON" link in the top-right corner preserves quick access to the raw file for power users.

Initial settings exposed: **Window Opacity/Transparency** and **Telemetry**. The architecture is designed so adding more settings sections over time is trivial.

## Current State

### How Settings Work Today

1. **No dedicated settings UI** -- users edit `settings.json` on disk (via `wsh edit` or code editor pane)
2. **Backend file watcher** detects changes to `settings.json`, broadcasts a `config` WebSocket event
3. **Frontend Jotai atoms** (`fullConfigAtom` -> `settingsAtom`) update, triggering React re-renders automatically
4. **`RpcApi.SetConfigCommand(TabRpcClient, { "key": value })`** is the programmatic write path -- it patches `settings.json` on disk, which triggers the same file-watcher -> atom update flow
5. **`AppSettingsUpdater`** in `frontend/app/app.tsx:123-149` applies window-level settings (opacity, transparency class, bgcolor) to DOM

### Key Files

| File | Role |
|------|------|
| `schema/settings.json` | JSON Schema -- all valid setting keys and types |
| `agentmuxsrv-rs/src/backend/wconfig.rs` | Rust struct definition with defaults |
| `frontend/app/store/global.ts` | `settingsAtom`, `getSettingsKeyAtom()`, `getSettingsPrefixAtom()`, `SetConfigCommand` usage |
| `frontend/app/app.tsx` | `AppSettingsUpdater` -- applies window:transparent, window:opacity, window:bgcolor to DOM |
| `frontend/app/block/block.tsx:38-46` | `BlockRegistry` -- maps view type strings to ViewModel classes |
| `frontend/app/block/blockutil.tsx` | `blockViewToIcon()`, `blockViewToName()` -- icon/name fallbacks |
| `frontend/app/view/helpview/helpview.tsx` | Simplest existing ViewModel -- good pattern reference |
| `frontend/app/store/wshclientapi.ts:411` | `SetConfigCommand` signature |

### Relevant Settings Keys

**Window (from schema):**
| Key | Type | Description |
|-----|------|-------------|
| `window:transparent` | boolean | Enable transparency |
| `window:blur` | boolean | Enable blur effect |
| `window:opacity` | number (0-1) | Window opacity level |
| `window:bgcolor` | string | Background color |

**Telemetry:**
| Key | Type | Description |
|-----|------|-------------|
| `telemetry:enabled` | boolean | Enable/disable telemetry |

## Design

### Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│ Block Header: [gear icon] Settings              [Open Settings JSON] │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  WINDOW                                                          │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │ Opacity          [━━━━━━━━━●━━] 0.80                      │  │
│  │ Transparent      [  toggle  ]                              │  │
│  │ Blur             [  toggle  ]                              │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  TELEMETRY                                                       │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │ Enable Telemetry [  toggle  ]                              │  │
│  │ (muted) Helps improve AgentMux by sending anonymous usage  │  │
│  │ data.                                                      │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

### Data Flow

```
User moves slider/toggles switch
  -> onChange handler calls RpcApi.SetConfigCommand(TabRpcClient, { "window:opacity": 0.75 })
  -> Backend writes to settings.json
  -> File watcher fires config event via WebSocket
  -> fullConfigAtom updates -> settingsAtom updates
  -> All subscribers re-render (AppSettingsUpdater applies opacity, Settings pane reflects new value)
```

This is identical to the existing flow when a user edits settings.json manually. No new backend work required.

### Input Controls

| Setting | Control Type | Details |
|---------|-------------|---------|
| `window:opacity` | Range slider + numeric display | min=0, max=1, step=0.05. Only editable when `window:transparent` is true (disabled + dimmed otherwise). |
| `window:transparent` | Toggle switch | On/off. Toggling off also removes opacity CSS. |
| `window:blur` | Toggle switch | On/off. Independent of transparent. |
| `telemetry:enabled` | Toggle switch | On/off. Includes helper text below. |

## Implementation

### Step 1: Create SettingsViewModel

**File:** `frontend/app/view/settings/settings.tsx`

```typescript
import { atoms, createBlock, getApi, getSettingsKeyAtom } from "@/app/store/global";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { fireAndForget } from "@/util/util";
import { atom, useAtomValue } from "jotai";

class SettingsViewModel implements ViewModel {
    viewType = "settings";
    viewIcon = atom("gear");
    viewName = atom("Settings");
    viewComponent = SettingsView;

    // "Open Settings JSON" button in the block header (top-right)
    endIconButtons = atom<IconButtonDecl[]>([
        {
            elemtype: "iconbutton",
            icon: "file-code",
            title: "Open Settings JSON",
            click: () => {
                fireAndForget(async () => {
                    const path = `${getApi().getConfigDir()}/settings.json`;
                    const blockDef: BlockDef = {
                        meta: { view: "preview", file: path },
                    };
                    await createBlock(blockDef, false, true);
                });
            },
        },
    ]);

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        // blockId and nodeModel stored if needed for future use
    }
}
```

The ViewModel is minimal -- follows the same pattern as `HelpViewModel`. The `endIconButtons` atom puts the "Open Settings JSON" button in the block header's top-right area (same slot used by other views for action buttons).

### Step 2: SettingsView Component

**File:** `frontend/app/view/settings/settings.tsx` (same file, below the class)

The view component reads settings via `useAtomValue(getSettingsKeyAtom("key"))` and writes via `RpcApi.SetConfigCommand`. Each input is a controlled component whose value comes from the Jotai atom.

**Section component pattern:**

```typescript
function SettingsSection({ title, children }: { title: string; children: React.ReactNode }) {
    return (
        <div className="settings-section">
            <div className="settings-section-title">{title}</div>
            <div className="settings-section-content">{children}</div>
        </div>
    );
}
```

**Toggle component pattern:**

```typescript
function SettingToggle({ label, settingKey, description }: {
    label: string;
    settingKey: keyof SettingsType;
    description?: string;
}) {
    const value = useAtomValue(getSettingsKeyAtom(settingKey)) ?? false;

    const handleChange = () => {
        fireAndForget(async () => {
            await RpcApi.SetConfigCommand(TabRpcClient, { [settingKey]: !value });
        });
    };

    return (
        <div className="setting-row">
            <div className="setting-label-group">
                <div className="setting-label">{label}</div>
                {description && <div className="setting-description">{description}</div>}
            </div>
            <button
                className={clsx("setting-toggle", value && "active")}
                onClick={handleChange}
                role="switch"
                aria-checked={value}
            />
        </div>
    );
}
```

**Slider component pattern:**

```typescript
function SettingSlider({ label, settingKey, min, max, step, disabled }: {
    label: string;
    settingKey: keyof SettingsType;
    min: number;
    max: number;
    step: number;
    disabled?: boolean;
}) {
    const value = useAtomValue(getSettingsKeyAtom(settingKey)) ?? min;

    const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
        const newValue = parseFloat(e.target.value);
        fireAndForget(async () => {
            await RpcApi.SetConfigCommand(TabRpcClient, { [settingKey]: newValue });
        });
    };

    return (
        <div className={clsx("setting-row", disabled && "disabled")}>
            <div className="setting-label">{label}</div>
            <div className="setting-slider-group">
                <input
                    type="range"
                    min={min}
                    max={max}
                    step={step}
                    value={value as number}
                    onChange={handleChange}
                    disabled={disabled}
                />
                <span className="setting-slider-value">{(value as number).toFixed(2)}</span>
            </div>
        </div>
    );
}
```

**Main view composition:**

```typescript
function SettingsView() {
    const isTransparent = useAtomValue(getSettingsKeyAtom("window:transparent")) ?? false;

    return (
        <div className="settings-view">
            <SettingsSection title="Window">
                <SettingToggle label="Transparent" settingKey="window:transparent" />
                <SettingToggle label="Blur" settingKey="window:blur" />
                <SettingSlider
                    label="Opacity"
                    settingKey="window:opacity"
                    min={0}
                    max={1}
                    step={0.05}
                    disabled={!isTransparent}
                />
            </SettingsSection>
            <SettingsSection title="Telemetry">
                <SettingToggle
                    label="Enable Telemetry"
                    settingKey="telemetry:enabled"
                    description="Helps improve AgentMux by sending anonymous usage data."
                />
            </SettingsSection>
        </div>
    );
}
```

### Step 3: Register in BlockRegistry

**File:** `frontend/app/block/block.tsx`

```typescript
import { SettingsViewModel } from "@/app/view/settings/settings";
// ...
BlockRegistry.set("settings", SettingsViewModel);
```

### Step 4: Update blockutil.tsx

**File:** `frontend/app/block/blockutil.tsx`

Add cases to the icon/name fallback functions:

```typescript
// In blockViewToIcon():
if (view == "settings") return "gear";

// In blockViewToName():
if (view == "settings") return "Settings";
```

### Step 5: Add Settings Widget to Default Widgets

**File:** `agentmuxsrv-rs/src/config/widgets.json`

Add a settings widget entry so it appears in the sidebar widget bar:

```json
"defwidget@settings": {
    "display:order": 100,
    "icon": "gear",
    "label": "settings",
    "description": "Application settings",
    "blockdef": {
        "meta": {
            "view": "settings"
        }
    }
}
```

`display:order: 100` places it after all other widgets. It will appear at the bottom of the widget bar (above the help widget, which is hardcoded below all `defwidget@` entries).

### Step 6: Styles

**File:** `frontend/app/view/settings/settings.scss`

Key styling decisions:
- **Layout:** Single scrollable column, max-width ~600px, centered
- **Sections:** Subtle card-like groups with section title headers
- **Toggle switch:** CSS-only toggle (no external dependency), uses `--accent-color`
- **Slider:** Native `<input type="range">` styled with CSS to match app theme
- **Disabled state:** `opacity: 0.4` + `pointer-events: none` on disabled controls
- **Spacing:** Consistent with existing pane padding (`padding: 16px 20px`)

## Opening the Settings Pane

The settings pane can be opened by:

1. **Widget bar** -- clicking the gear icon (Step 5 above)
2. **Programmatically** -- `createBlock({ meta: { view: "settings" } })`
3. **Future:** Keyboard shortcut (e.g., `Ctrl+,`) -- not in this spec

## Adding More Settings Later

To add a new setting to the pane:

1. Ensure the key exists in `schema/settings.json` and `wconfig.rs` (it probably already does)
2. Add a `<SettingToggle>`, `<SettingSlider>`, or new control type inside the appropriate `<SettingsSection>` (or create a new section)
3. No other changes needed -- `SetConfigCommand` handles any valid settings key

**Likely next additions:**
- Terminal: font size, font family, theme, scrollback, copy-on-select
- Editor: minimap, word wrap, font size
- Window: background color, tile gap size, reduced motion
- Auto-update: enabled, channel

## Out of Scope

- Settings search/filter (premature for ~5 controls)
- Per-connection or per-block overrides (existing override system handles this separately)
- Import/export settings
- New backend endpoints (not needed -- `SetConfigCommand` already exists)

## Testing

1. Open settings pane via widget bar gear icon
2. Toggle `window:transparent` on -- verify window becomes transparent
3. Move opacity slider -- verify window opacity changes in real-time
4. Toggle `telemetry:enabled` -- verify value persists across pane close/reopen
5. Click "Open Settings JSON" in header -- verify settings.json opens in code editor pane
6. Edit `window:opacity` in settings.json directly -- verify slider in settings pane updates
7. Verify opacity slider is disabled when transparent toggle is off
