# Spec: Settings → Widget Pane with UI Form

**Date:** 2026-06-25
**Status:** Draft
**Scope:** Replace the hamburger "Settings" entry (opens raw `settings.json` in
external editor) with a first-class widget-bar pane that renders all settings as
labeled form controls, while retaining the raw-JSON escape hatch for power users.

---

## Problem

The current Settings flow:
1. User clicks hamburger → Settings
2. Backend ensures `settings.json` exists on disk
3. File opens in whatever the user's OS default editor is (VS Code, Notepad, vim…)
4. User edits raw JSON — no labels, no descriptions, no validation, no autocomplete
5. On save, the backend fs-watcher detects the change and propagates to the app

This is a developer-grade experience. It:
- Leaks file paths and internal key names to users who don't need them
- Provides no guidance on valid values, ranges, or enums
- Offers no grouping or search
- Is disconnected from the app's reactive state (user can't preview a theme change)
- Cannot live side-by-side with an agent pane

---

## Goal

A **Settings pane** widget — same scaffold as Toolchain / Armory — that:
1. Presents all user-facing settings as form controls with labels and descriptions
2. Applies changes immediately and reactively via `SetConfigCommand` (no Save button)
3. Groups settings into logical sections with a left-rail navigator
4. Retains "Open raw JSON" as a footer link for advanced use
5. Runs as a proper block pane, openable from the widget bar and the hamburger

---

## Settings Catalog & Grouping

The full settings schema (`schema/settings.json`) defines ~70 keys. The pane exposes
the user-facing subset grouped into 7 sections. Internal/debug keys, platform-only
keys, and secret keys (API tokens) are either hidden or in an Advanced section.

### Section 1 — Appearance

| Key | Label | Control | Notes |
|---|---|---|---|
| `window:theme` | Theme | Select | "default", "midnight", "high-contrast", "monokai", "nord", "dracula", "catppuccin", "tokyo-night", "gruvbox" |
| `window:opacity` | Window opacity | Slider 35–100% | `window:transparent` must be true; show note if not |
| `window:transparent` | Window transparency | Toggle | Enables opacity + blur |
| `window:blur` | Background blur | Toggle | Requires `window:transparent` |
| `window:tilegapsize` | Pane gap size (px) | Number 0–20 | Gap between tiled panes |
| `window:reducedmotion` | Reduce motion | Toggle | Disables CSS transitions |

### Section 2 — Terminal

| Key | Label | Control | Notes |
|---|---|---|---|
| `term:fontsize` | Font size | Number 8–32 | px |
| `term:fontfamily` | Font family | Text | Comma-separated fallback list |
| `term:theme` | Terminal color theme | Select | Populated from `fullConfigAtom().termthemes` |
| `term:scrollback` | Scrollback lines | Number 1000–100000 | Default 10000 |
| `term:copyonselect` | Copy on select | Toggle | |
| `term:shiftenternewline` | Shift+Enter → new line | Toggle | |
| `term:allowbracketedpaste` | Bracketed paste | Toggle | |
| `term:transparency` | Terminal background opacity | Slider 0–1 | |

### Section 3 — Agent

| Key | Label | Control | Notes |
|---|---|---|---|
| `term:agentmaxruntimehours` | Max runtime (hours) | Number ≥0 | 0 = unlimited |
| `term:agentidletimeoutmins` | Idle timeout (minutes) | Number ≥0 | 0 = unlimited |
| `voice:enabled` | Voice input button | Toggle | Shows mic button per pane |
| `voice:engine` | STT engine | Select | "whisper", "webspeech", "groq", "whisper-local" |
| `voice:whisperModel` | Whisper model | Text | e.g. "base.en" — shown if engine = whisper |
| `voice:whisperCliPath` | whisper.cpp path | Text | Path override |

### Section 4 — Sounds & Notifications

| Key | Label | Control | Notes |
|---|---|---|---|
| `notify:sounds:enabled` | Notification sounds | Toggle | Master enable |
| `notify:sounds:volume` | Volume | Slider 0–1 | Shown only if enabled |
| `notify:sounds:suppresswhenfocused` | Suppress when focused | Toggle | Mute if pane is in foreground |
| `notify:sound:agent.turn.complete` | Sound: turn complete | Toggle | |
| `notify:sound:agent.turn.error` | Sound: turn error | Toggle | |
| `notify:sound:agent.turn.interrupted` | Sound: interrupted | Toggle | |
| `notify:tooltones:enabled` | Tool-call tones | Toggle | Subliminal per-tool audio |
| `notify:tooltones:volume` | Tool-tone volume | Slider 0–1 | |
| `notify:tooltones:scope` | Tool-tone scope | Select | "all" \| "focused" |

### Section 5 — Network

| Key | Label | Control | Notes |
|---|---|---|---|
| `network:lan_discovery` | LAN discovery | Toggle | mDNS peer discovery; may prompt Windows Firewall |

(This duplicates the HostPopover toggle — both stay; HostPopover is for quick access,
Settings pane is the canonical place. Both write the same key.)

### Section 6 — Files & Drag-Drop

| Key | Label | Control | Notes |
|---|---|---|---|
| `dnd:enabled` | File drag-and-drop | Toggle | |
| `dnd:maxfilesizemb` | Max file size (MB) | Number | |
| `dnd:agentinserttoken` | Insert placeholder tokens | Toggle | For dropped files in agent panes |
| `preview:showhiddenfiles` | Show hidden files | Toggle | File picker |

### Section 7 — Advanced

Collapsed by default. Contains developer/power-user keys and secret-adjacent values.

| Key | Label | Control | Notes |
|---|---|---|---|
| `app:globalhotkey` | Global hotkey | Text | Key binding string |
| `term:localshellpath` | Shell executable | Text | Default: system shell |
| `term:disablewebgl` | Disable WebGL (terminal) | Toggle | Fallback to canvas2D |
| `window:disablehardwareacceleration` | Disable GPU acceleration | Toggle | Requires restart |
| `telemetry:enabled` | Usage telemetry | Toggle | |
| `cmd:env` | Environment variables | Key-value editor | `key=value` pairs injected into all shells |
| `messaging:discord:enabled` | Discord bridge | Toggle | |
| `messaging:discord:channel` | Discord channel ID | Text | |
| `messaging:discord:target` | Discord → agent ID | Text | |

**Secret keys** (`messaging:discord:token`, `voice:groqApiKey`) — shown as masked
password fields with copy/clear buttons only. Never log or display in plaintext.

---

## Component Architecture

### SettingsViewModel

**File:** `frontend/app/view/settings/settings-model.ts`

```typescript
export class SettingsViewModel implements ViewModel {
    viewType = "settings";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon = () => "cog";
    viewName = () => "Settings";
    viewComponent = SettingsView;

    // Active section — persisted in wave object meta ("settings:section")
    activeSection: () => SettingsSection;
    setSection: (s: SettingsSection) => void;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        // read "settings:section" from block meta, default "appearance"
    }
}

type SettingsSection =
    | "appearance"
    | "terminal"
    | "agent"
    | "sounds"
    | "network"
    | "files"
    | "advanced";
```

### SettingsView

**File:** `frontend/app/view/settings/settings.tsx`

```tsx
function SettingsView(props: ViewComponentProps<SettingsViewModel>) {
    const settings = settingsAtom;    // reactive
    const section = () => props.model.activeSection();

    return (
        <div class="settings-view">
            <aside class="settings-rail">
                <SettingsRailItem ... />  {/* one per section */}
            </aside>
            <main class="settings-body">
                <Switch>
                    <Match when={section() === "appearance"}>
                        <AppearanceSection settings={settings()} />
                    </Match>
                    ...
                </Switch>
                <footer class="settings-footer">
                    <button onClick={openRawJson}>
                        <i class="fa-solid fa-file-code" /> Open raw settings.json
                    </button>
                </footer>
            </main>
        </div>
    );
}
```

### Setting Control Pattern

Every setting row uses the same `SettingRow` primitive:

```tsx
<SettingRow
    label="Theme"
    description="UI color theme for all windows"
    control={
        <select
            value={settings()["window:theme"] ?? "default"}
            onChange={(e) =>
                fireAndForget(() =>
                    RpcApi.SetConfigCommand(TabRpcClient, {
                        "window:theme": e.target.value,
                    } as SettingsType)
                )
            }
        >
            {THEME_OPTIONS.map((t) => <option value={t.id}>{t.label}</option>)}
        </select>
    }
/>
```

**Key design rules:**
- **No Save button.** Every control change fires `SetConfigCommand` immediately.
  The backend merges the single key; the fs-watcher propagates the update; the
  WPS `fullconfig` event causes `fullConfigAtom` to update; all controls re-render.
- **Optimistic UI.** Controls read from `settingsAtom()` (the server-confirmed value),
  NOT from local controlled state. The round-trip is fast (~10ms local); no flicker.
- **Debounce for sliders/numbers.** Inputs like opacity, font-size, and volume use a
  200ms debounce before calling `SetConfigCommand` to avoid saturating the config
  writer during drag.
- **Secret fields.** Password inputs use `type="password"` + a toggle-reveal button.
  Clear is a separate destructive button that calls `SetConfigCommand({ key: "" })`.

---

## Widget Registration

### `widgets.json`

```json
"defwidget@settings": {
    "display:order": 99,
    "display:pinned": false,
    "icon": "cog",
    "label": "Settings",
    "blockdef": {
        "meta": { "view": "settings" }
    }
}
```

`display:order: 99` places it last in the widget list (behind Toolchain/Trust).
`pinned: false` — in More dropdown by default.

### Block registry

```typescript
// block-registry.ts
import { SettingsViewModel } from "@/app/view/settings/settings-model";
blockViewRegistry.set("settings", SettingsViewModel as any);
```

---

## Hamburger Menu Update

Replace the external-editor open with `openOrFocusPaneByView`:

```typescript
// hamburger-menu.tsx (was lines 110–117)
{
    label: "Settings",
    icon: "cog",
    onClick: () => fireAndForget(openOrFocusPaneByView("settings")),
},
```

The theme and opacity submenus in the hamburger are **removed** — they become
redundant once the Settings pane is available. This simplifies the hamburger.

---

## "Open raw JSON" Escape Hatch

At the bottom of every section is a footer link:

```
⌘ Open raw settings.json
```

Clicking it calls the existing IPC path:

```typescript
const openRawJson = async () => {
    const path = await invokeCommand<string>("ensure_settings_file");
    await invokeCommand("open_in_editor", { path });
};
```

This preserves full power-user access for bulk edits, copy-paste from docs, or
settings not yet surfaced in the UI.

---

## `configerrors` Surface

`fullConfigAtom().configerrors` contains any parse/validation errors in the current
`settings.json`. The Settings pane renders these at the top of every section as a
dismissible error banner:

```tsx
<Show when={fullConfigAtom()?.configerrors?.length > 0}>
    <div class="settings-config-errors">
        {fullConfigAtom().configerrors.map((e) => (
            <div class="settings-config-error">
                <i class="fa-solid fa-circle-exclamation" />
                {e.err} {e.filename && <span class="mono">{e.filename}:{e.linenum}</span>}
            </div>
        ))}
        <button onClick={openRawJson}>Fix in editor</button>
    </div>
</Show>
```

---

## Live Preview for Theme / Opacity

For theme and opacity controls, the change takes effect instantly app-wide (the
backend propagates via `fullconfig` WS event → `fullConfigAtom` → CSS vars update).
No special preview mechanism needed — the pane itself re-themes as the user selects.

A small preview swatch next to the theme selector shows a color sample derived from
the theme's CSS variable values without requiring a full page re-render.

---

## Implementation Sequence

### Phase 1 — Scaffold (no design work)
1. Add `defwidget@settings` to `widgets.json`
2. Create `settings-model.ts` (ViewModel stub)
3. Create `settings.tsx` with 7 section stubs (headings only, no controls)
4. Register in `block-registry.ts`
5. Update hamburger to `openOrFocusPaneByView("settings")`
6. Remove theme/opacity submenus from hamburger

### Phase 2 — Appearance + Terminal sections
- Wire all Appearance and Terminal controls
- `SettingRow` primitive component
- Slider + debounce utility

### Phase 3 — Agent + Sounds + Network + Files sections

### Phase 4 — Advanced section
- `cmd:env` key-value editor (add/remove rows)
- Secret field with mask/reveal/clear
- Discord bridge settings

### Phase 5 — Polish
- `configerrors` banner
- Theme preview swatch
- Search/filter across all settings keys
- `settings:section` persistence in wave meta

---

## Files Affected

| File | Change |
|---|---|
| `agentmux-srv/src/config/widgets.json` | Add `defwidget@settings` |
| `frontend/app/block/block-registry.ts` | Register `"settings"` view type |
| `frontend/app/view/settings/settings-model.ts` | **New** — ViewModel |
| `frontend/app/view/settings/settings.tsx` | **New** — ViewComponent |
| `frontend/app/view/settings/settings.scss` | **New** — styles |
| `frontend/app/window/hamburger-menu.tsx` | Replace Settings action; remove theme/opacity submenus |
| `frontend/app/store/global.ts` | `openOrFocusPaneByView()` (shared with toolchain/trust spec) |

---

## Out of Scope

- **Tab-level settings** (`tab:preset`, `tab:color`) — these are per-tab, not
  global; they belong in tab context menus, not the global Settings pane.
- **Connection settings** (`conn:*`) — the Connections panel (WSH) is a separate
  surface; don't merge here.
- **Widget pinning UI** — managed by right-click on the widget bar, not the Settings pane.
- **Agent-level overrides** — per-agent model/effort/provider config lives in the agent
  Identity/Preset system (Armory), not global settings.
- **Import/export** — bulk settings portability is out of scope; the raw JSON escape
  hatch covers this use case adequately.
