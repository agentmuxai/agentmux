# SPEC: a default color for every widget type, set per widget in Settings

**Status:** active. PR 1 (§7) is built: the color lookup, the built-in colors and the header-tail rule. PR 2 (Settings, menu) and PR 3 (top bar) are next. §8 records the decisions taken.
**Date:** 2026-10-05
**Author:** Agent3

## 1. The ask

Each pane-tab type (widget: Terminal, Agent, Browser, Files, ...) gets a default color. Today a pane has a color only if the user picks one ("Pane Color" in the header menu) or the pane is an agent with an identity color. Wanted:

1. A starting set of default colors, chosen at random (§3.3).
2. A default color acts exactly like a picked one: border highlight, pane-tab pill, active-tab underline, header tint, Swarm rows, everything a picked color drives today.
3. **The widget at the top** carries a subtle hue of its widget's color and updates live when that color changes (§3.6).
4. **Settings** gets an area where the user sets the color for each widget (§3.5).
5. The design is DRY. Other DRY opportunities found along the way are listed in §4.2.

**How this spec reads "the widget at the top" (§8 D1):** the widget's button in the top bar (`ActionWidget`, `frontend/app/window/action-widgets.tsx`). The Terminal button gets a subtle tint of the Terminal color. It changes the moment that color changes, whether from Settings or from a pane's color menu ("Use for all Terminal panes", §3.7). The pane's own header already tints with its color today (`headerTint`) and keeps doing so. With this change it also tints for panes that only have a default color.

## 2. How colors work today

The facts this design builds on. Paths are under `frontend/app/` unless noted.

- **One color model already exists.** Every pane color comes from one *identity hue*, normalised in OKLCH: `identityOklchHue` and `paneRoleColor(hue, hex, isLight, role)` in `block/pane-color-scheme.ts`. `PANE_COLOR_TOKENS` holds fixed lightness and chroma per role (`identity`, `border`, `pill`, `pillActive`, `headerTint`) and per theme. This is the result of `SPEC_PANE_COLOR_SYSTEM_CONSOLIDATION_2026_09_20.md` and the OKLCH rules in `REPORT_PANE_TAB_COLOR_BEST_PRACTICES_2026_10_02.md` §6 P7. It has two inputs:
  - block meta `frame:hue` (0–360), written by the Pane Color menu (`buildPaneColorSubmenu` in `block/blockframe.tsx`, palette `PANE_HUE_OPTIONS` in `block/pane-color-menu.ts`, 12 hues 30° apart);
  - block meta `frame:activebordercolor` (a hex), the agent identity color seeded at launch.
- **Every reader goes through role wrappers that read those two meta keys.** `computeBlockColorBg`, `computeBlockTabPillBg`, `computeBlockTabPillActiveBg`, `computeBlockIdentityColor`, `computeBlockActiveBorderColor` and `computeFocusRingBorderColor` in `block/blockframe.tsx` each repeat the same two reads. They feed the header, the old focus ring (`BlockMask`), the terminal border (`--block-agent-color`), the pane ring (`--pane-ring-color`, `element/PaneChrome.tsx`), the pills (`--pane-tab-*`, `element/PaneTabStrip.tsx`), the header tail and the Swarm rows (`view/swarm/swarm-row-colors.ts`).
- **With no color**, every wrapper returns `undefined` and its caller falls back to a neutral value (`computeNonAgentHeaderBg`, `--accent-color` and `--border-color` for the ring, `computeBlockTabPillNeutralBg` for pills).
- **The widget-type registry** is `PaneTabManifest` (`block/pane-tab-registry.ts`): `view`, `aliases`, `label`, `icon`, `capabilities`, ... Built-in views register in `block/block-registry.ts`; third-party widgets register through `block/widget-loader.ts`. It is the one table keyed by widget type.
- **Top-bar buttons** come from the `widgets` config (`crates/srv/src/config/widgets.json`). Each entry opens `blockdef.meta.view`. Some entries carry a `color` hex (agent `#cc785c`, swarm `#f59e0b`, ...). The top bar does not render it: icons are monochrome via `--widget-icon-color`, per `SPEC_WIDGET_ICON_COLORS_2026-05-26.md`. Only the Launcher grid uses it.
- **Settings** rows are declared per section (`view/settings/sections/*.tsx`, search index `settings-index.ts`). Values are written with `set(key, value)` (`SetConfigCommand`) and read reactively with `getSettingsKeyAtom(key)` (`store/block-atom-cache.ts`). A map-valued setting already exists (`cmd:env`). Backend: `SettingsType` in `crates/srv/src/backend/wconfig/types.rs`; schema `schema/settings.json`.

## 3. Design

### 3.1 Principle: a default is the lowest tier of the same identity hue

A widget default is another source for the **same** identity hue the resolver already uses. Nothing downstream changes: every role (border, pill, underline, header tint, ring) is still `paneRoleColor(hue, ...)`. "Acts exactly like a picked color" (§1 item 2) then holds by construction, not by updating each surface.

Precedence, highest first:

| Tier | Source | Stored where |
|---|---|---|
| 1 | Explicit pick on this pane | block meta `frame:hue` (as today) |
| 2 | Agent identity | block meta `frame:activebordercolor` (as today) |
| 3 | User's color for this widget type | setting `pane:colors[view]` (new, §3.4) |
| 4 | Built-in color for this widget type | `PaneTabManifest.defaultHue` (new, §3.3) |
| — | none | neutral fallbacks as today |

Tiers 3 and 4 are **never written into block meta**. They are computed at read time, so changing a widget's color in Settings recolors every open pane of that type at once, and "Default" in a pane's menu goes back to following the widget's color.

An agent pane keeps its own identity (tier 2), so different agents still look different. The Agent widget's color (tiers 3–4) only shows on an agent pane that has no identity color, and on the Agent button in the top bar.

### 3.2 One resolver, used everywhere

Built in `block/pane-identity.ts` (`pane-color-scheme.ts` stays pure color math). As shipped, the identity carries the source's own value, a hue or the agent's hex, and the role colors stay `paneRoleColor`'s job:

```ts
/** Where a pane's color comes from, for its tab dot, Swarm and tests. */
export type PaneHueSource = "pick" | "agent" | "widget-setting" | "widget-default";

export interface PaneIdentity {
    hslHue?: number;        // a pick or a widget color, on the Pane Color scale
    hex?: string;           // the agent's identity color
    source: PaneHueSource;  // "pick" | "agent" | "widget"
}

export function widgetHueFor(view): number | undefined;          // tiers 3–4
export function resolvePaneIdentity(meta, opts?: { widget?: boolean }): PaneIdentity | undefined;
export function blockRoleColor(meta, isLightTheme, role, opts?): string | undefined;
export function isLightThemeActive(): boolean;
```

- `widgetHueFor(view)` is the only place tiers 3 and 4 are combined: the `pane:colors` entry for the view **after alias resolution** (`workflows` → `drone`), else the manifest's `defaultHue`. It reads the setting through `getSettingsKeyAtom("pane:colors")`, so every memo that colors a pane re-runs when the setting changes. (`cpuplot` is registered as its own view by the same factory as `sysinfo`, so it has the same color.) `{ widget: false }` skips it, for the tail rule (§3.8).
- The six `computeBlock*` wrappers in `blockframe.tsx` become one-liners over `blockRoleColor`, or are replaced by it. Their two duplicated meta reads go away (§4.1 item 1).
- `isLightThemeActive()` replaces the seven copies of the light-theme check (§4.1 item 2).
- `agentColorOf` in `view/agent/useAgentStream.ts` (the activity flash) switches to the resolver. Today it reads `frame:activebordercolor` raw and ignores a `frame:hue` pick (a small bug, §4.1 item 3).

### 3.3 Built-in defaults: the random starting set

`PaneTabManifest` gets an optional `defaultHue?: number`, next to `label` and `icon`. The type's look is then described in one place, and a third-party widget can declare its own. The starting set uses the 12 `PANE_HUE_OPTIONS` hues, so every default is also a swatch the user can pick.

The hues were drawn with a seeded shuffle. The first twelve views (the most used) get all twelve hues once; the other eight reuse a second shuffle:

| View | Label | Default | Hue |
|---|---|---|---|
| `term` | Terminal | Blue | 240 |
| `agent` | Agent | Coral | 30 |
| `browser` | Browser | Amber | 60 |
| `editor` | Editor | Violet | 270 |
| `files` | Hangar | Teal | 180 |
| `sysinfo` | Sysinfo | Fuchsia | 300 |
| `swarm` | Swarm | Crimson | 0 |
| `media` | Media | Green | 120 |
| `help` | Help | Chartreuse | 90 |
| `launcher` | Launcher | Pink | 330 |
| `remotes` | Remotes | Sky | 210 |
| `memory` | Memory | Emerald | 150 |
| `identity` | Identity | Amber | 60 |
| `drone` | Drone | Blue | 240 |
| `warden` | Warden | Violet | 270 |
| `toolchain` | Toolchain | Sky | 210 |
| `connectors` | Connectors | Chartreuse | 90 |
| `knowledge` | Knowledge | Emerald | 150 |
| `armory` | Armory | Pink | 330 |
| `settings` | Settings | Crimson | 0 |

The draw gave Agent Crimson, which can read as "error", and Swarm Coral. They were swapped (§8 D4): Agent is Coral, close to today's agent `#cc785c`, and the twelve most-used widgets keep twelve distinct colors. Settings drew Coral alongside Swarm and moved with it to Crimson. `block-registry.test.ts` checks that every built-in has a color from the palette and that the twelve are distinct.

A manifest without `defaultHue` (a third-party widget that declares none) gets **no** default and stays neutral, as today. Hashing the view name to a hue was considered and rejected: a color nobody chose, which changes if the widget is renamed.

### 3.4 The setting

`pane:colors`: an object from view to hue, e.g. `{ "term": 120, "agent": null }`.

- A number (0–360) overrides the built-in default (tier 3).
- `null` means "no color for this widget": the user switched the default off, and the pane falls back to neutral. This is different from "not set", which falls through to tier 4.
- Removing a key returns that widget to its built-in default.
- Backend: no new field. `SettingsType`'s flattened `extra` map already carries keys it doesn't name (as `window:theme` is), and `merge_settings_to_disk` writes them. Typed in `frontend/types/srv-types.d.ts` and `schema/settings.json` (an object of `number | null` in 0–360).
- Hue, not hex, on purpose: it is what `frame:hue` already stores and what the OKLCH tokens consume, so a custom color gets the same per-theme normalisation as a preset (best-practices report P7).

### 3.5 Settings: "Widget colors"

A new subsection in **Appearance** (`view/settings/sections/appearance-section.tsx`), indexed in `settings-index.ts` with keywords *color, colour, widget, pane, tab, border*.

- **One row per registered widget**, generated from the registry, so a new or third-party widget appears without editing Settings. Each row shows the widget's icon and label, a swatch row (the 12 hues plus **None**), and **Reset** when the value differs from the built-in default.
- **A live preview chip** per row, a pill painted with `blockRoleColor(..., "pill")` and its underline, so the user sees the real result in the current theme.
- **Reset all** at the top deletes `pane:colors`.
- **Writes** use `set("pane:colors", next)`, the same path `cmd:env` uses.
- **Shared picker.** The swatch row is a component, `HueSwatchRow` (§4.1 item 4). The Pane Color menu builds its entries from the same `PANE_HUE_OPTIONS` and swatch function, so the two pickers can't drift.

### 3.6 The top-bar widget's subtle hue

- `ActionWidget` resolves its view from `widget.blockdef.meta.view` and asks `widgetHueFor()` for the hue. It never reads a pane, so many Terminal panes in different colors don't make the Terminal button flicker.
- The tint uses a new token role, `widgetTint`, in `PANE_COLOR_TOKENS`:
  - It colors the **icon** only, at low chroma (around `c: 0.06`), with lightness close to `--widget-icon-color` in each theme.
  - The button background stays neutral, and hover and active states keep their current tokens.
  - This keeps the top bar calm, as `SPEC_WIDGET_ICON_COLORS_2026-05-26.md` intended, while tying each button to its panes. "Subtle" is a single token to tune.
- It is applied as an inline `--widget-tint` on `.widget-icon`, falling back to `var(--widget-icon-color)` in `action-widgets.scss`.
- Entries with no view stay monochrome: messenger children, group parents, "More".
- **Reactive by construction:** `widgetHueFor()` depends on the setting, so a change in Settings or through §3.7 repaints the button immediately.
- **The widget's `color` field in `widgets.json`:** built-in entries stop carrying a hex. Their color now comes from the manifest, and keeping both would be two sources for one fact (§4.1 item 5). The Launcher grid switches to `widgetHueFor()` too. A user-defined widget's `color` is honoured as before where it is used; no migration is needed.

### 3.7 Pane Color menu: "Use for all <Label> panes"

The header's Pane Color submenu gets one entry: **Use for all Terminal panes**, with the label taken from the manifest.

- It writes the pane's current hue to `pane:colors[view]` and clears this pane's `frame:hue`, so the pane now follows the widget color.
- This is how "the user changes the pane tab color" reaches the top-bar widget (§1 item 3) without making every pick global: a plain pick still recolors only that pane.
- **Default** in the same menu clears `frame:hue` as today, which now means "follow this widget's color".
- On agent panes the entry is hidden: a pick there also saves the agent's identity color (`ui:color`), and widget colors don't apply to agents with an identity.

### 3.8 The header tail must not regress

`SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md` (amended 2026-10-02) colors a multi-tab pane's header tail by counting **identities**: an uncolored utility tab (Accounts, CPU) does not compete with an agent's tint. If every tab now has a default color, a pane with one agent plus an Accounts tab has two colors, and its tail turns neutral. That would undo the owner's own 2026-10-02 request.

Rule: **only tiers 1 and 2 count as identities in the tail rule.**

- A tab colored only by a widget default (tiers 3–4) counts as uncolored there.
- A pane whose tabs have no identity uses the rule's existing no-identity path. If all its tabs share one widget color, the tail shows that color, the same "one color, it is about that color" reasoning as the spec's §2.1. Otherwise it stays neutral.

`PaneHueSource` (§3.2) is what makes this a one-line check in `headerTailBg` (`PaneChrome.tsx`). Every pill still paints its own effective color, defaults included.

### 3.9 What does not change

- What `frame:hue` and `frame:activebordercolor` mean and store; agent identity seeding (`launchAgentDefinition`, `register_agent_open`); the agent's save-back to `ui:color`.
- The OKLCH tokens for existing roles; contrast stays as the 2026-10-02 work measured, since defaults are just hues through the same tokens.
- Window-tab colors (`tab:color`, `TAB_COLORS`): a different object, out of scope.

## 4. DRY

### 4.1 Consolidated by this design

1. **Six wrappers read the same two meta keys.** (Done in PR 1; the five `…ForEffectiveColor` aliases in `pane-color-menu.ts`, each a one-line call of `paneRoleColor`, were removed with them.) `computeBlockColorBg`, `computeBlockTabPillBg`, `computeBlockTabPillActiveBg`, `computeBlockIdentityColor`, `computeBlockActiveBorderColor` and `computeFocusRingBorderColor` (all `blockframe.tsx`), plus the Pane Color menu's own read, become `resolvePaneIdentity` + `blockRoleColor`. Adding tiers 3–4 then touches one function instead of seven.
2. **The light-theme check was copied seven times** (`blockframe.tsx` ×3, `PaneChrome.tsx` ×3, a variant in `swarm-view.tsx`). It is `isLightThemeActive()` now. **Done in PR 1**, as are items 1 and 3.
3. **The activity flash keeps its own color read** (`agentColorOf`, `useAgentStream.ts`): it ignores `frame:hue` and would ignore the new tiers. It switches to the resolver.
4. **Two hue pickers.** The Pane Color menu and the new Settings rows share `PANE_HUE_OPTIONS` and one swatch function and component.
5. **A widget's look lives in two places.** For built-ins, `widgets.json` `color` and the manifest become one: the manifest's `defaultHue`.
6. **No widget-type → color map.** `widgetHueFor()` is the single combination of built-in and user colors, used by panes, the top bar, the Launcher and Settings.

### 4.2 Other DRY opportunities (documented, not part of this work)

| # | Duplication | Where | Suggested fix |
|---|---|---|---|
| A | Five palettes for "pick a color" | `PANE_HUE_OPTIONS` (pane-color-menu.ts), `AGENT_COLOR_PALETTE` (view/agent/agent-color.ts), `TAB_COLORS` (tab/tab.tsx), `REMOTE_COLORS` (view/remotes/remotes-sections.ts), widget `color` (widgets.json) | One `color-palettes.ts` naming each palette and its purpose. Remote and agent colors could adopt the 12-hue set and the OKLCH normalisation; window tabs stay desaturated by design (`SPEC_TAB_COLOR_DESATURATION_2026_08_13.md`). |
| B | Agent palette kept by hand in two languages | `agent-color.ts` and `crates/srv/src/backend/agent_color.rs` | Generate one from the other, or a test that compares them. |
| C | Agent color assigned at launch in two places, each with its own pick/dim | frontend `launchAgentDefinition` (view/agent/agent-model.ts), backend `register_agent_open` (crates/srv/src/server/app_api/agent_open.rs) | Backend only; the frontend asks or reads it. |
| D | Three copies of color math | `hslToHex` (pane-color-menu.ts), `hslToRgb`/`hexToRgb` (pane-color-scheme.ts), `parseCssColor`/`hslToRgb` (block/autotitle.ts) | One `color-math.ts`. |
| E | Five hex validators | `isValidAgentColor` (agent-color.ts), `validColor` (store/remote-display.ts), `ICON_COLOR_RE` (element/pane-tab-model.tsx), `colorRegex` (block/blockutil.tsx), the `hexToRgb` regex | One `isHexColor` in `color-math.ts`. |
| F | View → label and icon defined in several places, and they disagree | manifests; `widgets.json` (term `square-terminal` vs `terminal`, editor `file-code` vs `file-lines`, swarm `bee` vs `diagram-project`); `store/command-registry.ts` "open:*" commands hard-code icon and `iconColor`; per-model `viewIcon` (agent-model.ts, sysinfo-model.ts, media-pane.tsx); `widget-loader.ts` fallbacks | The manifest is the source; widgets.json, commands and models read it. Decide each disagreeing icon once. |
| G | Two border paths for one color | `BlockMask` + `--block-agent-color` (blockframe.tsx, block.scss) and `PaneChrome` `--pane-ring-color` | Remove the old `BlockMask` ring where hoisted chrome always draws the ring. |
| H | `frame:hue` missing from `MetaType` | `frontend/types/srv-types.d.ts` | Add it; drop the casts. Small enough to do in this work's first PR. |
| I | Settings defaults scattered | every read site supplies its own `?? default`; the Rust side uses `Option`; the template has comments | A frontend `SETTINGS_DEFAULTS` table read by `getSettingsKeyAtom`. `widgetHueFor()` follows that pattern for its own key. |

## 5. Files

| Area | Files |
|---|---|
| Resolver and tokens | `block/pane-identity.ts` (`resolvePaneIdentity`, `blockRoleColor`), `block/pane-color-scheme.ts` (`widgetTint` token, PR 3), `block/pane-color-menu.ts` (aliases removed) |
| Registry | `block/pane-tab-registry.ts` (`defaultHue`), each built-in manifest (the §3.3 values), `block/pane-identity.ts` (`widgetHueFor()`) |
| Readers | `block/blockframe.tsx`, `element/PaneChrome.tsx` (tail rule §3.8), `element/PaneTabStrip.tsx`, `view/swarm/swarm-row-colors.ts`, `view/agent/useAgentStream.ts` |
| Menu | `block/blockframe.tsx` `buildPaneColorSubmenu` (§3.7) |
| Top bar | `window/action-widgets.tsx`, `window/action-widgets.scss`, `view/launcher/launcher.tsx` |
| Settings | `view/settings/sections/appearance-section.tsx`, `settings-index.ts`, `settings-controls.tsx` (`HueSwatchRow`) |
| Config | `crates/srv/src/backend/wconfig/types.rs`, `frontend/types/srv-types.d.ts`, `schema/settings.json`, `settings-template.jsonc`, `crates/srv/src/config/widgets.json` (built-in `color` removed) |
| Theme check | `isLightThemeActive()` in `block/pane-identity.ts` |

## 6. Tests

- **Precedence.** `resolvePaneIdentity` for each tier and every combination: pick beats agent beats setting beats built-in; `null` in the setting means none; an alias resolves to its view; an unknown view has none.
- **Reactivity.** Change `pane:colors` and assert that an open pane's ring, pill and header, plus the top-bar button, recolor without remount (vitest with the settings atom).
- **Tail rule.** Agent + Accounts keeps the agent tint; two terminals with the same default show it; terminal + browser with different defaults is neutral; two agents is neutral.
- **Contrast.** Extend the existing per-hue contrast test to `widgetTint` against the top-bar background in both themes.
- **Settings.** Rows come from the registry (a registered test widget appears); Reset deletes the key; None writes `null`.
- **Menu.** "Use for all" writes the setting and clears `frame:hue`; it is hidden on agent panes.
- **Backend.** `pane:colors` round-trips through `SettingsType`, and the schema accepts `number | null`.
- **Screenshots** in a dark and a light theme of a mixed layout before and after, for the PR (best-practices report §7 asks for these on any change to the look of every pane).

## 7. Delivery

1. **PR 1, resolver and defaults (built):** §3.1–3.3 and §3.8, `frame:hue` and `pane:colors` typed (§4.2 H), `isLightThemeActive`. Every pane gets its default color; the setting can be edited in `settings.json` but has no UI yet.
2. **PR 2, Settings and menu:** §3.4, §3.5, §3.7.
3. **PR 3, top bar:** §3.6, the `widgetTint` token, Launcher, `widgets.json` cleanup.

PR 1 alone changes the look of every uncolored pane, so it could also ship behind PR 2's setting with "None" as each widget's starting value. §8 D3 decides.

## 8. Decisions

Taken 2026-10-05 on the recommendations below (the owner asked to proceed with the best recommendations): D1 the top-bar button, D2 agents keep their own color, D3 on for everyone, D4 Agent swapped to Coral (§3.3), D5 icon-only tint.

- **D1. "The widget at the top".** This spec reads it as the top-bar button (§1, §3.6). If it meant the pane's own header, that already tints and follows the pane's color, and §3.6 is not needed.
- **D2. Agent panes.** Agents keep their own identity color over the Agent widget color (§3.1). Alternative: the Agent widget color wins unless the agent has an explicit pick. Agents would then all look the same, which loses the per-agent identity the 2026-08-08 agent color work added.
- **D3. Rollout.** Turn the defaults on for everyone in PR 1, or ship them as "None" until PR 2 so users opt in.
- **D4. The palette.** Keep the random draw (§3.3), or swap Agent's Crimson for something that doesn't read as an error.
- **D5. Top-bar strength.** Icon-only tint (proposed), or a tinted background chip as well.
