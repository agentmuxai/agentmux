# Spec: Widget Bar Pinning & "More" Dropdown

## Problem

The widget action bar currently shows every non-hidden widget in a flat list.
As the product grows (agent, forge, swarm, terminal, sysinfo, settings, help,
devtools — already 8, more coming), the bar becomes crowded and harder to
scan. Users need a way to keep the widgets they use most accessible without
losing access to the rest.

---

## Goals

- Show only **pinned** widgets directly in the action bar
- Collect remaining (unpinned, non-hidden) widgets in a **More** dropdown
- Ship sensible defaults for new installs: `agent`, `terminal`, `sysinfo` pinned
- Let users pin / unpin / reorder at will — persisted to settings
- Existing installs should not lose anything (clean migration)

---

## Concepts

| Term | Definition |
|------|------------|
| **Pinned** | Appears directly in the action bar. Ordered array. |
| **More** | Not pinned and not hidden — lives in the More dropdown. |
| **Hidden** | Not shown anywhere (existing `widget:hidden@<key>` setting). |

Every widget is in exactly one of these three states.

---

## Settings Schema

### New key: `widget:pinned`

```jsonc
// settings.json
"widget:pinned": ["defwidget@agent", "defwidget@terminal", "defwidget@sysinfo"]
```

- **Type:** `string[]` (ordered array of widget keys)
- **Default on new install:** `["defwidget@agent", "defwidget@terminal", "defwidget@sysinfo"]`
  derived from `display:pinned: true` in `widgets.json`
- **Absent key means:** uninitialized — run migration (see below)

### Existing keys (unchanged)

| Key | Meaning |
|-----|---------|
| `widget:hidden@<key>` | Hide from bar and More dropdown entirely |
| `widget:order` | **Repurposed:** order of widgets within the More dropdown |
| `widget:icononly` | Show icons only (no labels) in the bar — unchanged |

### Priority chain (most → least precedent)

```
widget:hidden@<key>  →  in widget:pinned  →  widget:order (More)  →  display:order (widgets.json)
```

---

## widgets.json Changes

Add `display:pinned` field to each entry (the default state for a **new install**):

```json
"defwidget@agent":    { "display:pinned": true,  "display:order": 1, ... }
"defwidget@terminal": { "display:pinned": true,  "display:order": 4, ... }
"defwidget@sysinfo":  { "display:pinned": true,  "display:order": 5, ... }
"defwidget@forge":    { "display:pinned": false, "display:order": 2, ... }
"defwidget@swarm":    { "display:pinned": false, "display:hidden": true, ... }
"defwidget@settings": { "display:pinned": false, "display:order": 6, ... }
"defwidget@help":     { "display:pinned": false, "display:order": 7, ... }
"defwidget@devtools": { "display:pinned": false, "display:order": 8, ... }
```

`display:pinned` is only consulted when `widget:pinned` is absent (new install).
It has no effect once the user has a real `widget:pinned` setting.

---

## Migration (Existing Installs)

When `widget:pinned` is **absent** from settings.json:

1. If `widget:order` **is** set → the user has already customized their bar.
   Treat all currently visible widgets (not `display:hidden`, not `widget:hidden@<key>`)
   as pinned, preserving their order from `widget:order`.
   Write the result into `widget:pinned` on first config save.

2. If `widget:order` is **also absent** → fresh install.
   Derive pinned list from widgets where `display:pinned: true`.

This ensures existing users see no change on upgrade — everything they had in
the bar stays in the bar. They can then unpin widgets to tidy it up.

---

## UI: Action Bar

```
[ Agent ] [ Terminal ] [ Sysinfo ] [ ··· More ▾ ]
```

- Pinned widgets render exactly as today (icon + optional label, click to create)
- **More button** always appears after the pinned list if any unpinned,
  non-hidden widgets exist
- If all widgets are pinned (or hidden), the More button is omitted
- Existing drag-to-reorder continues to work within the pinned section;
  dropping a widget reorders the `widget:pinned` array

### More button states

| State | Appearance |
|-------|-----------|
| Closed | `··· More ▾` (or icon-only: `···`) |
| Open | `··· More ▲` with dropdown visible |
| Badge | Optionally show count: `··· More ▾ (4)` — useful when many widgets are unpinned |

---

## UI: More Dropdown

Opens below the More button, right-aligned to the button edge.

```
┌─────────────────────────┐
│ 🔨  forge               │
│ ⚙   settings            │
│ ?   help                │
│ <>  devtools            │
│                         │
│ ─────────────────────── │
│ Customize bar…          │
└─────────────────────────┘
```

- Items sorted by `widget:order` setting, then `display:order` from widgets.json
- Click on an item → creates the widget (same as clicking pinned)
- **"Customize bar…"** at the bottom → opens the widget manager (see below)
- Dismiss: click outside, press `Escape`, or click the More button again
- Width: fixed at 200px; labels truncated with ellipsis if needed

---

## UI: Right-Click Context Menus

### On a pinned widget (in the action bar)

```
─────────────────
  Unpin from bar
  ─────────────
  Hide widget
```

"Unpin" → removes from `widget:pinned`, widget moves to More dropdown.
"Hide" → sets `widget:hidden@<key>: true`, removes from everywhere.

### On a widget in the More dropdown

```
─────────────────
  Pin to bar
  ─────────────
  Hide widget
```

"Pin to bar" → appends to `widget:pinned`, widget appears at the end of the bar.
"Hide" → sets `widget:hidden@<key>: true`.

---

## UI: Widget Manager ("Customize bar…")

A lightweight modal or side-panel for power users who want full control.
**Not required for the initial implementation** — the right-click menus cover
the basic use case. This is v2.

When built, it should show:

```
Pinned (drag to reorder)         More / Hidden
──────────────────────           ─────────────────
[≡] Agent           [unpin]      [pin] Forge
[≡] Terminal        [unpin]      [pin] Settings
[≡] Sysinfo         [unpin]      [pin] Help
                                 [pin] Devtools (hidden)
```

- Drag within Pinned to reorder
- Drag from More to Pinned to pin
- Toggle hidden state per-widget
- "Restore defaults" resets `widget:pinned` and all `widget:hidden@*` keys

---

## Backend Changes

### `wconfig.rs` — `WidgetConfigType`

Add field:
```rust
pub display_pinned: bool,  // default false
```

### `wconfig.rs` — `SettingsType`

Add field:
```rust
pub widget_pinned: Option<Vec<String>>,  // None = uninitialized
```

### Config loading / migration

In `build_full_config()` or equivalent:

```
if settings.widget_pinned is None:
    if settings.widget_order is not empty:
        # existing install — pin all currently visible widgets
        pinned = [k for k in widget_order if not is_hidden(k)]
    else:
        # new install — use display:pinned defaults
        pinned = [k for k, w in widgets if w.display_pinned]
    settings.widget_pinned = Some(pinned)
```

Migration runs in-memory only; it is written to disk only when the user
subsequently saves any setting.

---

## Frontend Changes

### `action-widgets.tsx`

Replace `getSortedWidgets()` with two functions:

```typescript
getPinnedWidgets(): WidgetConfigType[]
  // widgets where key is in settings["widget:pinned"], in array order,
  // excluding widget:hidden@<key>

getMoreWidgets(): WidgetConfigType[]
  // widgets NOT in widget:pinned, NOT hidden,
  // sorted by widget:order then display:order
```

Render:
```tsx
<PinnedWidgetList widgets={pinned} />
<Show when={more.length > 0}>
  <MoreDropdownButton widgets={more} />
</Show>
```

### New `MoreDropdownButton` component

- Manages open/close state
- Renders the dropdown overlay (positioned absolutely)
- Calls same `handleWidgetSelect` as pinned widgets on click
- Handles right-click context menu for "Pin to bar" / "Hide"

### Context menu additions (`base-menus.ts`)

```typescript
// For pinned widget context menu
{ label: "Unpin from bar", click: () => unpinWidget(key) },
{ type: "separator" },
{ label: "Hide widget",    click: () => hideWidget(key) },

// For more-dropdown widget context menu
{ label: "Pin to bar",  click: () => pinWidget(key) },
{ type: "separator" },
{ label: "Hide widget", click: () => hideWidget(key) },
```

### Settings helpers

```typescript
function pinWidget(key: string): void
  // append key to widget:pinned, save via SetConfigCommand

function unpinWidget(key: string): void
  // remove key from widget:pinned, save via SetConfigCommand

function hideWidget(key: string): void
  // set widget:hidden@<key> = true, save via SetConfigCommand
```

---

## Settings Reference (complete after this change)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `widget:pinned` | `string[]` | `[agent, terminal, sysinfo]` | Ordered list of pinned widget keys |
| `widget:hidden@<key>` | `boolean` | `false` | Hide widget from bar and More |
| `widget:order` | `string[]` | `[]` | Order of widgets within More dropdown |
| `widget:icononly` | `boolean` | `false` | Show icons only in the action bar |

---

## Open Questions

1. **Badge on More button** — show count of unpinned widgets? Useful when many
   widgets accumulate. Minor visual noise tradeoff.

2. **Pin position** — when right-clicking "Pin to bar", should the widget append
   to the end of pinned, or insert after the last pinned widget of the same
   category? Simple append is fine for v1.

3. **Settings/Devtools special treatment** — these trigger actions rather than
   creating blocks. They should still be pinnable/unpinnable the same way.
   No special-casing needed.

4. **Keyboard navigation in More dropdown** — arrow keys to move, Enter to
   select, Escape to close. Treat as future polish.

5. **Per-window pinned state** — spec assumes pinned is global (shared across
   all windows). If per-window overrides are needed in the future, the key
   could become `widget:pinned@<windowlabel>` with fallback to `widget:pinned`.
