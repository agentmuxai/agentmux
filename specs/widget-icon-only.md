# Spec: Widget Icon-Only Mode

**Status:** Implemented
**PR:** https://github.com/agentmuxai/agentmux/pull/69
**Date:** 2026-03-07

## Problem

The action widgets bar (top-right of the window) shows both icons and text labels for each widget button (e.g., icon + "help", icon + "settings", icon + "devtools", plus any custom widgets). For users who are familiar with the icons, the text labels consume horizontal space without adding value. There is no way to hide them.

## Solution

Add a `widget:icononly` boolean setting that hides text labels on all action widgets, showing only icons. The setting is:

1. **Toggleable via right-click context menu** on the widget bar ("Icon Only" checkbox)
2. **Configurable in settings.json** — `"widget:icononly": true`
3. **Live-reloadable** — changes via settings.json are picked up by the file watcher (merged in PR #66)

When enabled, tooltips continue to show the widget label/description on hover, so discoverability is preserved.

## Current State (Before)

```
[ ? help ] [ cog settings ] [ </> devtools ]
```

- `ActionWidget` component renders icon + label unconditionally (unless label is blank)
- No setting exists to control label visibility
- `widget:showhelp` is the only widget-scoped setting

## After

```
# widget:icononly = false (default, unchanged)
[ ? help ] [ cog settings ] [ </> devtools ]

# widget:icononly = true
[ ? ] [ cog ] [ </> ]
```

## Implementation

### Files Changed

| File | Change |
|------|--------|
| `schema/settings.json` | Add `widget:icononly` (boolean) to schema |
| `frontend/types/gotypes.d.ts` | Add `widget:icononly?: boolean` to `SettingsType` |
| `agentmuxsrv-rs/src/backend/wconfig.rs` | Add `widget_icon_only: Option<bool>` to Rust `SettingsType` |
| `frontend/app/window/action-widgets.tsx` | Read setting, pass to `ActionWidget`, add context menu toggle |

### Frontend Detail

**`ActionWidget` component** — accepts new `iconOnly: boolean` prop:
```tsx
const ActionWidget = memo(({ widget, iconOnly }: { widget: WidgetConfigType; iconOnly: boolean }) => {
    // ...
    {!iconOnly && !isBlank(widget.label) && (
        <div className="text-xs whitespace-nowrap">{widget.label}</div>
    )}
    // ...
});
```

**`ActionWidgets` container** — reads setting from config atom:
```tsx
const iconOnly = fullConfig?.settings?.["widget:icononly"] ?? false;
```

**Context menu** — simple checkbox toggle (no submenu, unlike "Show Help Widgets"):
```tsx
{
    label: "Icon Only",
    type: "checkbox",
    checked: iconOnly,
    click: () => {
        fireAndForget(async () => {
            await RpcApi.SetConfigCommand(TabRpcClient, { "widget:icononly": !iconOnly });
        });
    },
},
```

### Rust Backend Detail

New field on `SettingsType` struct in `wconfig.rs`:
```rust
#[serde(rename = "widget:icononly", default, skip_serializing_if = "Option::is_none")]
pub widget_icon_only: Option<bool>,
```

Follows the same pattern as `widget_show_help`. No additional backend logic — the setting is purely consumed by the frontend.

### Schema

Added to `schema/settings.json` under the `widget:*` group:
```json
"widget:icononly": {
  "type": "boolean"
}
```

## Design Decisions

1. **Default is `false`** — labels shown by default, matching current behavior. No breaking change.
2. **Single boolean, not per-widget** — a per-widget `display:labelHidden` option would be overengineered for this use case. Users either want compact mode or they don't.
3. **Tooltips preserved** — when labels are hidden, hovering still shows the `description` (or `label` as fallback) via the existing `Tooltip` component. No information is lost.
4. **Applies to all widgets** — custom widgets from `widgets.json` and built-in widgets (help, settings, devtools) all respect the setting uniformly.
5. **Context menu is a checkbox** — simpler than the "Show Help Widgets" On/Off submenu pattern. A boolean toggle is more natural for this setting.

## Test Plan

- [ ] Right-click widget bar -> "Icon Only" toggles labels on/off immediately
- [ ] Setting persists across app restart
- [ ] Setting `"widget:icononly": true` in settings.json applies via live reload
- [ ] Tooltips still show label text when hovering icon-only widgets
- [ ] Custom widgets defined in widgets.json also respect the setting
- [ ] Default behavior (no setting) shows labels as before
- [ ] Context menu checkbox state reflects current setting value
