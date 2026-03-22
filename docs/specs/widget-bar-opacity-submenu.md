# Spec: Widget Bar Context Menu — Opacity Submenu

## Location in menu

Right-clicking the widget bar calls `createTabBarMenu()` in `base-menus.ts`, which
produces this item sequence today:

```
AgentMux v0.32.71        ← createTabBarBaseMenu()
─────────────────        ← .separator()           ← GAP (item A)
Pinned in bar            ← menu.section(...)       ← GAP (item B)
  [ ] Agent
  [ ] Terminal
  ...
─────────────────
[x] Icon Only
```

Items A and B are **two consecutive separators** (one plain, one labeled).
The Opacity submenu sits **between them** — after item A, before item B.

---

## Menu structure after change

```
AgentMux v0.32.71
─────────────────
Opacity  ▶         ← new submenu (inserted here)
  ● 100%           ← current value shown with radio bullet
  ○ 95%
  ○ 90%
  ...
  ○ 35%
─────────────────  ← section("Pinned in bar")
  [ ] Agent
  [ ] Terminal
  ...
─────────────────
[x] Icon Only
```

---

## Opacity values

14 steps, 100% → 35% inclusive, 5% each (descending):
`100, 95, 90, 85, 80, 75, 70, 65, 60, 55, 50, 45, 40, 35`

Each item is a `type: "radio"` ContextMenuItem. The checked item is the one
that matches the current resolved opacity (see §State mapping below).

---

## State mapping

The opacity submenu reads and writes **`window:opacity`** in settings
(`RpcApi.SetConfigCommand`).

Clicking a step:
- **< 100% (i.e. any value 0.20 – 0.95):** set `window:opacity` to the
  decimal value AND set `window:transparent` to `true` (enables the effect).
- **100%:** set `window:opacity` to `1.0` AND set `window:transparent` to
  `false` (restores fully-opaque window).

Reading current value for the radio check:
```ts
const rawOpacity = settings["window:opacity"] ?? 0.8;      // default 0.8
const isTransparent = settings["window:transparent"] ?? false;
const effectiveOpacity = isTransparent ? rawOpacity : 1.0;  // 1.0 = opaque
```
Round `effectiveOpacity` to the nearest 0.05 when matching the radio item.

Default (`window:opacity` not set and `window:transparent` false): 100% checked.

---

## Implementation plan

### 1. `frontend/app/menu/base-menus.ts`

Add `createOpacityMenu(settings)` helper:

```ts
function createOpacityMenu(settings: Record<string, any>): MenuBuilder {
    const menu = new MenuBuilder();
    const rawOpacity = settings["window:opacity"] ?? 0.8;
    const isTransparent = settings["window:transparent"] ?? false;
    const effective = isTransparent ? rawOpacity : 1.0;
    const currentStep = Math.round(effective * 20) / 20;   // snap to 0.05 grid

    for (let pct = 100; pct >= 35; pct -= 5) {
        const value = pct / 100;
        menu.add({
            label: `${pct}%`,
            type: "radio",
            checked: Math.abs(value - currentStep) < 0.001,
            click: async () => {
                const { RpcApi } = await import("@/app/store/wshclientapi");
                const { TabRpcClient } = await import("@/app/store/wshrpcutil");
                if (value < 1.0) {
                    await RpcApi.SetConfigCommand(TabRpcClient, {
                        "window:opacity": value,
                        "window:transparent": true,
                    } as any);
                } else {
                    await RpcApi.SetConfigCommand(TabRpcClient, {
                        "window:opacity": 1.0,
                        "window:transparent": false,
                    } as any);
                }
            },
        });
    }
    return menu;
}
```

Update `createTabBarMenu` to insert the submenu:

```ts
export function createTabBarMenu(fullConfig: any): MenuBuilder {
    const settings = fullConfig?.settings ?? {};
    return createTabBarBaseMenu()
        .separator()
        .submenu("Opacity", createOpacityMenu(settings))   // ← inserted here
        .merge(createWidgetsMenu(fullConfig));
}
```

### 2. No Rust / backend changes needed

`window:opacity` and `window:transparent` are already:
- Declared in `agentmuxsrv-rs/src/backend/wconfig.rs`
- Reactively applied in `frontend/app/app.tsx` via `AppSettingsUpdater`
- Forwarded to the Tauri window via `setWindowTransparency`

### 3. No new config keys

Re-uses existing `window:opacity` (f64, 0.0–1.0) and `window:transparent` (bool).

---

## Files changed

| File | Change |
|------|--------|
| `frontend/app/menu/base-menus.ts` | Add `createOpacityMenu()`, update `createTabBarMenu()` |

---

## Out of scope

- Per-pane opacity (separate feature)
- Blur toggle from this menu (existing `window:blur` setting, out of scope)
- Opacity values below 35% (impractical; window becomes hard to use)
- Animated opacity transitions
