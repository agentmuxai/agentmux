# Theme picker in hamburger menu + midnight agent-pane black background

**Status:** Implemented (PR #791)
**Owner:** AgentA
**Date:** 2026-05-10
**Revised:** 2026-05-10 — first cut targeted the right-click tabbar context menu (`base-menus.ts` `createTabBarMenu`); corrected to the actual hamburger (≡) button in `frontend/app/tab/tabbar.tsx` (`tabBarMenuItems`). Opacity moved alongside Theme. Added `Exit` entry and softened menu-item hover color (`--accent-color` → `--hover-bg-color`).
**Driving observation:** AgentMux ships nine themes (`default`, `midnight`, `high-contrast`, `monokai`, `nord`, `dracula`, `catppuccin`, `tokyo-night`, `gruvbox`) and the `window:theme` setting is fully wired end-to-end, but the only way for a user to switch is to hand-edit `settings.json`. We want a `Theme` entry in the hamburger (window-header) menu that expands to a radio list of the available themes. Additionally, the `midnight` theme's deep-navy background (`rgb(10, 12, 22)`) is too light for the agent pane; midnight specifically should paint the agent pane pure black.

---

## 1. What's already in place (no work needed)

| Piece | Location | Notes |
|---|---|---|
| Theme files | `frontend/app/themes/{default,midnight,high-contrast,monokai,nord,dracula,catppuccin,tokyo-night,gruvbox}.scss` | Each declares a `[data-theme="<name>"]` block of CSS custom properties (`--main-bg-color`, `--main-text-color`, etc.) |
| Theme barrel | `frontend/app/themes/index.scss` | Imports all theme files |
| Theme selector | `frontend/app/app.tsx` lines 150-156 in `AppSettingsUpdater` | Reads `window:theme`, calls `document.documentElement.setAttribute("data-theme", theme)` when not `"default"`; removes the attribute for `"default"` |
| Persistence | `~/.agentmux-<version>/config/settings.json`, key `window:theme` | Already a string in the enum at `schema/settings.json:221-225` |
| RPC write | `RpcApi.SetConfigCommand(TabRpcClient, { "window:theme": value })` | Used by the Opacity submenu today (`menu-builder.ts:114-117`) |
| Menu builder | `frontend/app/menu/{base-menus.ts, menu-builder.ts}` | `MenuBuilder.submenu(label, builder)` + radio items with `checked` is exactly the Opacity pattern we'll copy |
| Hamburger trigger | `frontend/app/window/window-header.tsx:27-30` calls `createTabBarMenu(fullConfig())` | Right-click on window header → `ContextMenuModel.showContextMenu()` |

The implementation is genuinely additive — no new state, no new RPC, no new infrastructure.

---

## 2. UI: hamburger menu `Theme` submenu

The hamburger (≡) button lives in `frontend/app/tab/tabbar.tsx` line 623, wrapped in `<FlyoutMenu items={tabBarMenuItems()}>`. `tabBarMenuItems` is a `createMemo<MenuItem[]>` returning a static list (New Tab / New Window / Settings / Help). Insert `Theme` (and `Opacity`, moved from the right-click context menu) between the New Window divider and Settings. The submenu is a radio list — exactly one theme is active — using a new `checked?: boolean` field on `MenuItem` that `FlyoutMenu` renders as a check icon (true) or blank-width spacer (false).

### Theme list, in menu order

The order matters for muscle memory; once shipped, don't reshuffle.

1. **Default** — `"default"` (light)
2. **Midnight** — `"midnight"` (dark navy, soon pure-black agent pane)
3. **High Contrast** — `"high-contrast"`
4. **Monokai** — `"monokai"`
5. **Nord** — `"nord"`
6. **Dracula** — `"dracula"`
7. **Tokyo Night** — `"tokyo-night"`
8. **Catppuccin** — `"catppuccin"`
9. **Gruvbox** — `"gruvbox"`

### Implementation sketch

`frontend/app/menu/base-menus.ts`:

```ts
interface ThemeOption {
    id: string;     // matches the schema enum value
    label: string;  // user-visible label
}

const THEME_OPTIONS: ReadonlyArray<ThemeOption> = [
    { id: "default", label: "Default" },
    { id: "midnight", label: "Midnight" },
    { id: "high-contrast", label: "High Contrast" },
    { id: "monokai", label: "Monokai" },
    { id: "nord", label: "Nord" },
    { id: "dracula", label: "Dracula" },
    { id: "tokyo-night", label: "Tokyo Night" },
    { id: "catppuccin", label: "Catppuccin" },
    { id: "gruvbox", label: "Gruvbox" },
];

function createThemeMenu(settings: FullConfigType): MenuBuilder {
    const current = (settings["window:theme"] as string) || "default";
    const builder = new MenuBuilder();
    for (const opt of THEME_OPTIONS) {
        builder.add({
            label: opt.label,
            type: "radio",
            checked: current === opt.id,
            click: () => {
                RpcApi.SetConfigCommand(TabRpcClient, {
                    "window:theme": opt.id,
                });
            },
        });
    }
    return builder;
}
```

Wire it into `createTabBarMenu()`:

```ts
export function createTabBarMenu(settings: FullConfigType): ContextMenuItem[] {
    return new MenuBuilder()
        .add({ label: `AgentMux ${appVersion}`, click: copyVersionToClipboard })
        .separator()
        .submenu("Opacity", createOpacityMenu(settings))
        .submenu("Theme", createThemeMenu(settings))         // ← new
        .separator()
        .merge(createWidgetsMenu(settings))
        .build();
}
```

That's the entire UI patch. ~30 LOC.

### Behavior

- Click a theme → `SetConfigCommand` writes to `settings.json` → `fullConfigAtom` updates via WebSocket → `AppSettingsUpdater` flips `data-theme` on `<html>` → CSS variables re-resolve → repaint. **No reload needed** (this already works for the existing manual-edit path).
- The radio `checked` state reflects the currently persisted value; switching is immediate and survives restart.
- If `window:theme` is unset / invalid, the menu shows `Default` checked.

---

## 3. Midnight: pure-black agent pane background

Today midnight's `--main-bg-color` is `rgb(10, 12, 22)` (deep navy). That paints the whole app including the agent pane. The user wants the **agent pane specifically** to be pure black under midnight; other surfaces stay navy.

### Approach: scoped override, not a new variable

Adding a `--agent-pane-bg-color` variable would force every theme to declare it and would break the "agent pane just inherits `--main-bg-color`" simplicity. Easier: scope the override to `[data-theme="midnight"] .agent-view` and use a plain color.

Add to `frontend/app/themes/midnight.scss` at the end of the existing `[data-theme="midnight"]` block:

```scss
[data-theme="midnight"] {
    // ... existing vars ...

    // Agent pane reads pure black under midnight — the global
    // --main-bg-color stays deep navy for other surfaces.
    .agent-view,
    .agent-view .agent-document {
        background: #000;
    }
}
```

Why both `.agent-view` and `.agent-view .agent-document`:
- `.agent-view` is the outer container; some panes (settings/identity tabs inside an agent) sit inside it and would otherwise inherit.
- `.agent-document` is the scrollable list; today it has no explicit background and falls through to whatever ancestor paints. Setting both is belt-and-suspenders.

### Hover-strip background

`NodeHoverStrip` currently uses `background: var(--main-bg-color)` (see `frontend/app/view/agent/styles/_document.scss:53`). Under midnight that would read deep navy and produce a visible color mismatch when the strip floats over a pure-black pane.

Fix at the same scope:

```scss
[data-theme="midnight"] {
    .agent-view .node-strip {
        background: #000;
    }
}
```

### Other panes are unaffected

Terminal, browser, sysinfo, swarm — all read `--main-bg-color` and stay on deep navy under midnight. Only the agent pane goes black.

---

## 4. Edge cases

- **First launch with no `window:theme` key set.** Existing behavior: `AppSettingsUpdater` reads `undefined`, treats it as `"default"`, removes the attribute. The menu shows `Default` checked. No change.
- **User selects a theme not in the menu (manually edited `settings.json`).** The radio group reflects no selection (none of the items have `checked: true`). The theme still applies. Acceptable; menu is a convenience layer, not a constraint.
- **Schema enum drift.** If a new theme is added to `schema/settings.json`, the `THEME_OPTIONS` array above must be updated. Mitigation: a short comment in the schema file pointing to `THEME_OPTIONS` and vice versa. Not worth a code-gen step for a once-a-year change.
- **Pane-zoom interaction.** Per-pane CSS `zoom` doesn't affect the theme; `data-theme` lives on `<html>`. Already works for all other themes; midnight's agent-pane override is plain CSS and will scale identically.
- **Multiple windows.** Theme is global (set on `<html>`). Every window in the same `task dev` / portable instance switches together. Acceptable — matches Opacity behavior today.
- **Existing pane content rendered under the old theme.** Repaint is automatic; CSS variable resolution is per-frame. No DOM remount or virtualizer invalidation needed.

---

## 5. Test plan

- [ ] Right-click window header → `Theme` submenu lists all 9 themes with `Default` (or persisted value) checked.
- [ ] Click each theme in turn → visible repaint within one frame, no reload.
- [ ] Restart app → last-selected theme persists.
- [ ] Under `midnight`, agent pane is pure black; terminal/browser/sysinfo panes are deep navy.
- [ ] Under `midnight`, hover strip in agent pane matches pane background (no navy stripe).
- [ ] Under every other theme, agent pane uses the theme's `--main-bg-color` unchanged.
- [ ] Edit `settings.json` directly → menu reflects the new value on next open.
- [ ] Unit test (`base-menus.test.ts`): `createThemeMenu({ "window:theme": "midnight" })` builds 9 items with midnight checked.

---

## 6. Out of scope

- **Theme preview on hover.** Possible follow-up; would need ephemeral `data-theme` swap + revert on mouse-out. Not worth it for v1.
- **Per-pane theme overrides.** Adds significant cross-cutting state. The midnight agent-pane fix above is a one-off, not a precedent.
- **Custom themes from `settings.json`.** Today themes are baked SCSS. A future "user-defined theme via JSON" feature would need a runtime CSS-var loader; out of scope.
- **Adding new themes.** Mechanical: SCSS file + barrel import + schema enum + menu entry. Documented separately if/when added.

---

## 7. Effort

| Component | LOC | Notes |
|---|---|---|
| `THEME_OPTIONS` + `createThemeMenu` in `base-menus.ts` | ~30 | Copy of Opacity pattern |
| Wire into `createTabBarMenu` | ~1 | One `.submenu(...)` chain call |
| Midnight agent-pane background overrides | ~10 | Single SCSS block in `midnight.scss` |
| Unit test for `createThemeMenu` builder | ~25 | Snapshot of menu shape per current setting |
| Manual smoke (test plan §5) | — | ~15 min |
| **Total** | **~65** | **~0.5 day** |

---

## 8. Files touched

| Path | Change |
|---|---|
| `frontend/app/menu/base-menus.ts` | Add `THEME_OPTIONS`, `createThemeMenu`, wire into `createTabBarMenu` |
| `frontend/app/themes/midnight.scss` | Add scoped `.agent-view` + `.node-strip` `background: #000` rules |
| `frontend/app/menu/base-menus.test.ts` (or sibling) | New unit test for theme menu builder |

No schema changes, no Rust changes, no migrations.

---

## 9. Cross-references

- Existing theme registry: `frontend/app/themes/index.scss`
- Theme selector: `frontend/app/app.tsx` (`AppSettingsUpdater`)
- Menu builder: `frontend/app/menu/menu-builder.ts`
- Opacity submenu (template pattern): `frontend/app/menu/base-menus.ts` (lines 97-128)
- Hamburger trigger: `frontend/app/window/window-header.tsx`
- Settings schema: `schema/settings.json` (`window:theme`)

---

## 10. Driving observation (verbatim)

> "lets work on themes ... we want to be able to select a theme from the hamburger menu ... write a spec that gets everything in place .. also update midnight to have a black background in the agent pane. write a spec to file. the hamburger would have a Theme entry that expands to a list of all the themes"
