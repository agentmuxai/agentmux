# Per-window theme and opacity: one opacity per window, a theme per window, new windows inherit

**Status:** active: implemented in #4798; the owner took the recommended answer to each question in §8.
**Date:** 2026-10-12.
**Requested by:** repo owner: "currently we have 2 opacity numbers, the one set in the hamburger and the one in the status bar version panel. We want to combine them. Currently each window gets its own opacity in the status bar version panel, but the hamburger appears to change all. lets have the hamburger behave like the version status panel, and they should change reactive to each other. meanwhile, we want the theme in the hamburger to be per-window. Currently changing the theme modifies all windows on the channel, but we want it to only change the actual window. if a new window is created, it inherits the theme from the window that created it".
**Author:** Clamk@narko.
**Related:**
- [instance-panel-floating-panes.md](instance-panel-floating-panes.md) (the status bar panel's per-window opacity rows, keyed by label);
- [SPEC_WINDOW_TRANSPARENCY.md](SPEC_WINDOW_TRANSPARENCY.md) and [SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01.md](SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01.md) (how a window is made see-through on each platform);
- [SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_PERSISTENCE_2026_07_06.md](SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_PERSISTENCE_2026_07_06.md) (srv's `Window.opacity` mirror);
- [SPEC_THEME_PICKER_AND_MIDNIGHT_AGENT_BG.md](SPEC_THEME_PICKER_AND_MIDNIGHT_AGENT_BG.md) and [themes-spec.md](themes-spec.md) (the themes);
- [REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md](REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md) (why a new window's first paint matters).

---

## 1. What's there today

### 1.1 Opacity: three controls, two stores, one native effect

| Control | Writes | Scope |
|---|---|---|
| Hamburger ☰ → Opacity (`window/hamburger-menu.tsx`) | settings `window:opacity` + `window:transparent` via `SetConfigCommand` | every window on the channel |
| Status bar version panel slider (`statusbar/InstancePanel.tsx`) | live: `dispatchWindowOpacity` → `setWindowOpacity(label)`; on release: the window object's meta `window:opacity` + `window:transparent` | one window (or one floating pane, session-only) |
| Settings → Appearance → Opacity (`view/settings/sections/appearance-section.tsx`) | settings `window:opacity` | every window on the channel |

Both IPCs end in the same native effect, uniform whole-window alpha: `WS_EX_LAYERED` + `SetLayeredWindowAttributes` on Windows, `NSWindow.alphaValue` on macOS, `_NET_WM_WINDOW_OPACITY` on X11, a CSS page fade on native Wayland (`crates/cef/src/commands/window/transparency.rs`). They fight over it:

- **Every window applies the global value.** `AppSettingsUpdater` (`app.tsx`) runs in every window and, on every settings tick, calls `setWindowTransparency(transparent, blur, settings opacity)` for its own window. A hamburger choice is a settings change, so it reaches every window. That is the "hamburger changes all" the owner sees.
- **A per-window value doesn't survive a reload.** The panel's per-window value is stored in the window's meta, but nothing reads it back when a window loads. The host's `get_window_opacity` exists for that and has no caller in the frontend. A reloaded or restored window shows the global value while the panel's slider, which reads the meta, shows the per-window one.
- **Any settings change undoes it.** The next settings tick, for any key under `window:`, re-applies the global opacity over a per-window choice.
- **The two controls don't agree.** The hamburger's checkmark reads settings; the panel reads the window's meta and the live store. Neither sees the other's change.
- **The value is persisted twice.** The host also writes each per-window change, debounced, to srv's `Window.opacity` field (Pillar 1 Step 2) beside the meta key the panel writes.

### 1.2 Theme: one value for the channel

`window:theme` is a single setting. The hamburger's Theme submenu and Settings → Appearance both write it with `SetConfigCommand`. `AppSettingsUpdater` in every window sets `data-theme` / `data-theme-polarity` on its document from it, so every window changes together.

Readers that must follow the window's theme rather than the setting:
- `AppSettingsUpdater` (`app.tsx`): the `data-theme` attributes.
- `isLightThemeActive()` (`block/pane-identity.ts`): pane colours. It reads `getSettingsKeyAtom("window:theme")`.
- The hamburger's checkmark.
- Terminals derive their palette from the document's `--term-*` CSS variables (`view/term/termutil.ts`), so they already follow the document. They recompute when settings change, and must recompute when the window's theme changes instead.
- Sandboxed widgets observe `data-theme` on the document (`block/sandboxed-widget-host.tsx`), so they follow per-window themes as is.

### 1.3 How a window is born

Every new window's renderer creates its srv `Window` row with `WindowService.CreateWindow(…, seedView, seedMeta)` (`app-init.ts`, `initCefNewWindow`). The ways in:

| Path | Started from | Has a creator window |
|---|---|---|
| New Window: hamburger, `Ctrl+Shift+N` (`keymodel.ts`), command palette (`command-registry.ts`), title bar menu, status bar panel | a window's renderer → `open_new_window` IPC | yes: the window the command ran in |
| Tab or pane tear-off (`drag/tear-off-pool-helper.ts` → `tear_off_pool_promote`) | the source window's renderer | yes: the window the tab left |
| "Open in New Window" on a widget (`openNewWindowWithView`) | a window's renderer | yes |
| Floating pane (pane pool) | a window's renderer | yes: the owner window |
| Second launch of the exe, login start, a notification click for a workspace no window shows | host or launcher | no |
| Session restore / crash reproject | srv's saved `Window` rows | no: the window has its own saved values |

`open_new_window` already threads an opaque `initial_meta` through both the pool path and the cold path, so the creator's identity can travel the same way.

## 2. Goals

1. **One opacity per window.** The hamburger's Opacity and the panel's slider set the same per-window value. Changing it in one window changes no other window.
2. **The two controls stay in sync.** A hamburger choice moves that window's panel slider and %, in this window's panel and in any other window's panel row for it. A slider drag moves the hamburger's checkmark.
3. **A per-window opacity lasts.** It survives a reload, an app restart and a crash reproject, and a settings change doesn't undo it.
4. **A theme per window.** The hamburger's Theme changes only the window it was opened in. Every reader in §1.2 follows that window's theme.
5. **A new window inherits from its creator.** It opens with its creator's theme, and, per §8 Q2, its opacity, with no flash of the default theme first.
6. **No visible change on upgrade.** A window with no per-window value of its own looks exactly as it does today.

## 3. Non-goals

- **Per-window blur and `window:transparent`.** `window:transparent` decides at process start whether windows are created alpha-capable (`read_window_transparent_setting`, `crates/cef/src/app/window_settings.rs`). It can't vary per window, and stays a channel setting.
- **Per-pane or per-tab themes.** Pane colours and tab colours are unchanged.
- **Floating panes in the hamburger.** A floater has no hamburger. It keeps its session-only slider in the panel and follows its owner's theme (§5.4).
- **Syncing theme across machines or channels.** A window's values live with that window.

## 4. Design: opacity

### 4.1 One value, one resolver

A window's opacity is its own meta key `window:opacity` when set, otherwise the channel default:

```
windowOpacity(win) = win.meta["window:opacity"]
                     ?? (settings["window:transparent"] ? settings["window:opacity"] ?? 0.8 : 1.0)
```

The fallback is exactly what every window shows today, which satisfies Goal 6. `opacityFor(windowMeta, settings)` in `store/window-appearance.ts` computes it, and `currentWindowOpacity()` is its reactive value for the window the code runs in. Combined with `liveWindowOpacity(label)` for the drag in progress, it is what every control displays.

**Source of truth:** the window's meta key. srv's `Window.opacity` mirror keeps being written by the host's debounced write-through, unchanged, since the crash reproject reads it. Retiring the duplicate is a follow-up (§9), not part of this work.

### 4.2 One setter, used by both controls

Extract the panel's two halves into one function in `window-opacity-store.ts`:

```ts
setWindowOpacityFor(target: { label: string; windowId: string | null }, opacity: number, phase: "live" | "commit")
```

- **`live`:** `dispatchWindowOpacity` → native alpha now. The panel calls this on `onInput`.
- **`commit`:** the same, plus a write to the window's meta (`window:opacity`). The panel calls this on `onChange`. The hamburger calls it on click, since a menu item is a single discrete choice.

`windowId: null` keeps today's floater behaviour: live only, never persisted.

100% is stored as `1`, not cleared. A cleared value would fall back to the channel default, which may be see-through, so a window set to 100% would not stay opaque. The panel no longer writes `window:transparent` beside it; the resolver doesn't need it.

The hamburger stops writing settings. Its submenu keeps its 100%–35% steps; the checkmark compares against this window's `currentWindowOpacity()` (or the drag in progress), rounded to 5% as now, so a slider value between steps, such as 72%, checks the nearest (70%).

### 4.3 Applying it: the window owns its alpha

`AppSettingsUpdater` stops pushing the channel opacity to the native window:
- **On load**, once the window's host label is known, it applies `currentWindowOpacity()` via `dispatchWindowOpacity` (`source: "restore"`, a command that already existed for this and was unused).
- **When that value changes**, it applies it again. A window with its own value resolves to it whatever the settings say, so a settings change leaves it alone; a window without one follows Settings → Appearance. The dispatch is untracked, so a slider drag doesn't re-run the effect and snap the window back.
- **The `setWindowTransparency` call is gone.** It only set the native alpha, the same thing `setWindowOpacity` does, and it set every window to the channel's value. The CSS `is-transparent` class and `--window-opacity` are still set from the settings and the window's resolved opacity, as before.

### 4.4 Reactivity

- **Same window:** both controls read the same signal (`liveWindowOpacity`) and the same window object, so a hamburger click moves the slider and a drag moves the checkmark on the next tick.
- **Other windows:** a window's row in another window's panel reads that window's object, which srv broadcasts on every meta write. It moves on commit: a hamburger click, or a slider release. It does not track the other window's drag tick by tick; that matches today.

### 4.5 Settings → Appearance → Opacity

It becomes the channel default: the value windows without one of their own use. Its description says so. As before, the row shows only while **Window transparency** is on, since the default applies only to a see-through channel; a window's own opacity needs no switch. See §8 Q1.

### 4.6 A reload keeps the window's own values

The main window reuses its srv row when it reloads, but any other window (opened by New Window, a tear-off, a widget or a layout) creates a new row each time its page loads, including a reload. Its own theme and opacity would stay on the old row. Found by the Linux test run on #4798.

The renderer remembers its row id in `sessionStorage`, which survives a reload, and passes it to `CreateWindow` as the optional `previousWindowId` (arg 6). srv copies the old row's `window:theme` and `window:opacity` onto the new row before it returns, so the window renders with them. A pool renderer keeps its `sessionStorage` when it is demoted and promoted again for a different window, so the pool's boot path forgets the id (`app/init/reload-window-id.ts`). Reusing the row on reload, as the main window does, would be the fuller fix; it is out of scope here.

## 5. Design: theme

### 5.1 One value, one resolver

```
windowTheme(win) = win.meta["window:theme"] ?? settings["window:theme"] ?? "default"
```

One reactive helper, `currentWindowTheme()` (new, in `store/`), resolves it for the window the code is running in. Each reader in §1.2 switches to it from `settings["window:theme"]` / `getSettingsKeyAtom("window:theme")`:
- `AppSettingsUpdater`: `data-theme` and `data-theme-polarity`;
- `isLightThemeActive()`;
- the hamburger's checkmark;
- the terminal's recompute trigger.

A test greps the frontend for direct reads of `"window:theme"` outside the resolver and the Settings pane, so a new reader can't bypass it. The repo's other grep gates work the same way.

### 5.2 The hamburger writes this window's theme

Theme → a theme writes `window:theme` to this window's meta (`ObjectService.UpdateObjectMeta(makeORef("window", windowId), …)`), not settings. Other windows don't change.

### 5.3 Settings → Appearance → Theme

It becomes the channel default: what a window with no theme of its own shows, and what a window with no creator opens with (§6). See §8 Q1 for whether it should also reset open windows.

### 5.4 Floating panes

A floater is its own renderer, with its own srv `Window` row (`CreateWindow` mints one per renderer). With no value of its own it shows the channel default theme, as today. Making a floater follow its owner window's theme is a follow-up (§9). Its opacity stays its own and session-only, as today.

## 6. Design: a new window inherits from its creator

### 6.1 The creator names the new window to srv

Threading the creator through the host would touch every pool and cold-path window function on three platforms. It isn't needed: every creator path learns the new window's host label as soon as its request returns (`openNewWindow`, `openNewWindowWithView`, `openWorkspaceInNewWindow`, `tearOffPoolPromote`, and the tear-off's cold path all return it).

Host labels are reused, though: a pool window can be demoted and promoted again straight away, and the old window's row can outlive that, because its close runs on another thread. So the label alone can't say which row is the new window's. The creator therefore:
1. takes a mark from srv, `WindowService.MarkWindowOpen()`, before it opens the window;
2. opens the window and gets its label;
3. calls `WindowService.InheritWindowAppearance(sourceWindowId, hostLabel, mark)`.

`openWindowInheriting(open)` and `openNewWindowFromHere()` (`store/window-appearance.ts`) do this for every path in §1.3 that has a creator. A tear-off asks for the mark alongside the open instead of before it, so the drag isn't slowed. If the mark arrives late, that window keeps the defaults; it can never select an older window.

### 6.2 srv copies, whichever call arrives first

Marks and the rows `CreateWindow` records share one counter in srv, so a row recorded after a mark was opened after it, and any older row for the same label belongs to an earlier use of the label. The new window's renderer calls `CreateWindow` at about the same moment as the creator's request, so the two race, and srv handles both orders (`server/service/window_inherit.rs`):
- `InheritWindowAppearance` records a pending entry (source, label, mark). Then, if `CreateWindow` has already recorded a row for the label **after the mark**, it takes the entry and copies.
- `CreateWindow` records its row for the label, then takes the pending entry if its row came after the entry's mark, and copies before it returns, so the renderer renders with the inherited values.

Taking an entry is atomic and each side publishes before it checks, so the copy happens exactly once in either order. An entry expires after 30 seconds.

The copied keys are `window:theme` and `window:opacity`, each only if the source sets it. The copy is a snapshot: changing the creator's theme later doesn't change the new window. If the source window has closed in between, nothing is copied.

### 6.3 No flash of the default theme

`initCefNewWindow` awaits `CreateWindow` before the window's content renders, so when the creator's call wins the race (the usual case: one round trip against the new renderer's two), the row already holds the inherited values at first render. When it loses, the values arrive a moment later as a meta update and the window switches theme once. Verify by eye on each platform, against the startup colour flash report.

### 6.4 Windows with no creator

The second-exe launch, login start and a notification click for an unshown workspace open with the channel defaults (Settings → Appearance). Session restore and crash reproject keep each window's own saved meta. That already happens, since meta is persisted with the `Window` row.

## 7. Plan

Built in three parts and shipped as one PR, verified live in a `task dev` build before review.

**P1: Opacity, combined (§4).**
- `setWindowOpacityFor(…, phase)` in `window-opacity-store.ts` and `opacityFor` / `currentWindowOpacity()` in `window-appearance.ts`; the panel moves onto them.
- The hamburger's Opacity moves onto them and stops writing settings.
- `AppSettingsUpdater` applies the window's resolved opacity on load and on change, as §4.3 describes, in place of `setWindowTransparency`.
- Settings → Appearance's Opacity is relabelled as the default.
- **Tests:**
  - Reducer and store: commit writes meta; live doesn't; a floater never persists.
  - Rendered hamburger: a click writes this window's meta only and no setting; the checkmark follows a dispatched slider value.
  - Rendered panel: a hamburger commit moves the slider and %.
  - `AppSettingsUpdater`: a window with its own value ignores a settings change; one without follows it; a reload re-applies the meta value.

**P2: Theme per window (§5).**
- `currentWindowTheme()` and the reader switch-over, with the grep test.
- The hamburger writes window meta.
- Floaters following their owner is deferred (§5.4, §9).
- Settings → Appearance's Theme relabelled as the default.
- **Tests:**
  - Rendered hamburger: a theme click sets this window's `data-theme` and not the setting.
  - `isLightThemeActive` follows window meta over the setting.
  - A terminal recomputes on a window theme change.

**P3: Inheritance (§6).**
- srv `MarkWindowOpen`, `InheritWindowAppearance` and the pending-entry take in `CreateWindow` (§6.2); `openWindowInheriting()` at every creator path.
- **Tests (srv):**
  - copies when the creator's call comes first, and when the window's row comes first;
  - an entry is used once;
  - a row from an earlier use of the same label, even moments old, doesn't take it;
  - a source with no values of its own copies nothing;
  - empty arguments are refused.
- **Live check:** New Window and tear-off from a window on a non-default theme open on that theme, with no flash, on Windows, macOS and Linux.

## 8. Owner decisions

The owner approved the spec as written (2026-10-12), so each question below takes its recommended answer.

- **Q1. What does Settings → Appearance do once the hamburger is per-window?**
  - *Recommended:* it sets the channel default only. Windows with no value of their own follow it live; windows with their own value don't; it is what a window with no creator opens with. Its rows are relabelled "Default theme for windows" / "Default opacity for windows".
  - *Alternative:* it also clears every open window's own value, so one change resets them all.
- **Q2. Does a new window inherit its creator's opacity too, or only its theme?**
  - *Recommended:* both, so a window from a see-through window is see-through. The request names only the theme.
- **Q3. Should the hamburger offer "Apply to all windows"?**
  - *Recommended:* not now. Q1's alternative or a later menu item covers it if it's missed.

## 9. Follow-ups, not in this work

- **The duplicate per-window opacity.** Retire srv's `Window.opacity` mirror in favour of the meta key, or the reverse, and make the crash reproject and `get_window_opacity` read the one that stays.
- **Floaters follow their owner's theme** (§5.4), live.
- **`liveWindowOpacity` for other windows.** Broadcasting a drag in progress to other windows' panels, if anyone wants it.
