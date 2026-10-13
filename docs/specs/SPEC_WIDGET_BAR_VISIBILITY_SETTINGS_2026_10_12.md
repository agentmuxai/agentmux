# Widget bar visibility in Settings: pin, keep in More, or hide

**Date:** 2026-10-12
**Status:** active — phases 1-3 (hidden entries, the Widget bar section, live "+" picker, the null fix) in PR #4754; §7 items 5-11 remain follow-ups. Verified 2026-10-12.
**Author:** AgentY
**Audited against:** `main` at `052f6f67f`
**Related:** `SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md` (§8.4, Settings → Widgets for packages),
`SPEC_WIDGET_BAR_PARENT_SUBMENUS_2026_08_12.md` (groups), `SPEC_WIDGET_PINBAR_DND_STATE_2026_06_15.md` (drag reorder),
`SPEC_DEFAULT_WIDGETS_REORDER_2026_08_25.md` (fresh-install pins), `SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md` (index entries)

## 1. Goal

Let a user decide, for every entry the widget bar knows about, whether it is **pinned** (an icon on the bar),
**in More** (in the More menu only, today's "unpinned"), or **hidden** (on neither). Hidden is the new state: it
lets someone shape the whole bar, More menu included, to the widgets they actually use.

The control lives in Settings → Widgets. It and the bar stay in sync in real time in both directions:
pinning, unpinning, reordering or hiding on the bar shows in Settings at once, and a change in Settings shows on
the bar at once, with no save button and no reload. A hand edit of `widgets.json` or `settings.json` shows in both
too.

## 2. What exists today

Answers to the questions that started this spec, with the evidence.

### 2.1 Is `widgets.json` still respected? Yes, read-only.

- The built-in entries are `crates/srv/src/config/widgets.json`, compiled in (`wconfig/loader.rs:17-35`).
- A user's `widgets.json` next to `settings.json` (`user_widgets.rs:27-31`, folder from `resolve_settings_dir()`)
  is merged on top. The order is built-ins, then installed widget packages (`ext@<id>/<pane>`), then the user file
  (`user_widgets.rs:117-124`). A user entry with the same key **replaces the whole built-in entry**; fields are not
  merged (`:74-76`).
- Nothing in the app writes `widgets.json`. The only writer is a screenshot script
  (`scripts/ui-screenshots/demo-env.mjs:150`), and `widgets.uninstall` refuses entries that come from it
  (`widget_handlers.rs:173-175`).
- **A user cannot hide a built-in entry today.**
  - A `null` value fails the parse of the whole file, so srv keeps the previous widgets (`user_widgets.rs:42,201`).
  - `display:hidden` exists on the entry type (`types.rs:562`, TS `srv-types.d.ts:1677-1690`), but the widget bar
    ignores it: `getMoreWidgets` filters only on pinned and grouped (`action-widgets-config.ts:174-188`). Only the
    Launcher reads it (`launcher.tsx:73-76`). The built-in file sets it on the five messenger children only, where it
    means "shown inside its group, not at the top level".

### 2.2 Is it reactive to save? Yes, with one exception.

- `widgets.json` is watched (300 ms debounce, deletes ignored, `user_widgets.rs:155-206`). A change re-merges and
  broadcasts the full config (`config_watcher_fs.rs:305-327`). The frontend replaces `fullConfigAtom` on the
  `WpsEvent.Config` event (`store/global.ts:284-290`), and the bar re-derives. No restart.
- `settings.json` is watched the same way (`config_watcher_fs.rs:133+`). A settings change swaps only the settings
  half, so the merged widgets survive (`wconfig/state.rs:64-70`).
- **Exception, a bug:** `setconfig` writes disk with null meaning delete (`merge_settings_to_disk`,
  `config_watcher_fs.rs:223-242`). But it updates memory with `merge_settings_into_current`, which **drops** null
  keys instead of removing them (`config_watcher_fs.rs:211`), and broadcasts that. So `{"widget:pinned": null}`
  broadcasts the old pins first; the file watcher corrects it about 300 ms later. A "Reset" button would flicker.

### 2.3 How the bar decides what to show

- Pinned: `widget:pinned` in `settings.json` (an ordered list of short names) wins when present, after old names are
  mapped (`RENAMED_PINS`, `action-widgets-config.ts:64-72`). Otherwise the fresh-install default is entries with
  `display:pinned`, minus grouped children, by `display:order` (`getPinnedKeys`, `:145-159`).
- More: everything not pinned and not a grouped child (`getMoreWidgets`, `:174-188`). On a narrow bar, pinned icons
  that don't fit are moved to the front of More (`action-widgets.tsx:581`).
- Groups: a parent lists `children`; a child pinned on its own is "promoted" out of the group (`:74-104`).
- Writes: every pin, unpin and reorder sends the whole `widget:pinned` array with `SetConfigCommand`
  (`action-widgets-config.ts:276-291`, `use-widget-drag-reorder.ts:86-104`, `titlebar-context-menu.tsx:28-41`).
  `widget:icononly` is toggled from the title-bar menu (`:43-47`) and Settings → Advanced
  (`advanced-section.tsx:131-142`).

### 2.4 Settings today

- Settings → Widgets (`widgets-section.tsx`) manages **installed widget packages** (install, approve, on/off,
  catalog, publisher keys). It reads none of the bar's config.
- Widget colors live under Appearance (`widget-colors.tsx`) and are per pane type, not per bar entry.
- "Icon-only widget labels" is under Advanced.
- Sections read config reactively (`settingsAtom()`, `getSettingsKeyAtom`) and write through `SetConfigCommand`
  (`settings-controls.tsx:17-18`). Anything that reads the same atoms as the bar is already live both ways.

## 3. Decisions

1. **Hidden is stored in `settings.json` as `widget:hidden`, an array of full entry keys**
   (`["defwidget@drone", "ext@acme.board/main"]`), not in `widgets.json`.
   - It goes through the same write path as `widget:pinned` (`setconfig`), which is already live and supports
     delete-by-null.
   - Writing `display:hidden` into `widgets.json` would copy the entire built-in entry into the user file (whole-entry
     replace, §2.1). That entry would then stop picking up AgentMux's own changes to it (icon, label, view), and
     `widgets.json` has no writer today.
   - Full keys, not short names, so installed package widgets (`ext@…`) can be hidden too.
2. **`display:hidden` keeps its current meaning** (an entry shown only inside its group, and kept out of the
   Launcher). The new user-facing state does not reuse it. §7 proposes renaming it later to make the difference plain.
3. **One state per entry, exclusive.** Hiding an entry also removes it from `widget:pinned`, in the same
   `SetConfigCommand` (`{"widget:hidden": [...], "widget:pinned": [...]}`). Showing it again puts it in More.
   Pinning a hidden entry removes it from `widget:hidden`.
4. **Hidden means gone from every list of widgets to open** (owner, 2026-10-12): the pinned icons, More, a group's
   flyout, the title-bar "Pin Widgets" checklist, the pane "+" picker, the empty-tab right-click menu and the
   Launcher grid. All of them read the same config and update together, including a "+" picker that is open at the
   moment of the change (§4.3). Hiding does not disable the pane type: the command palette, keyboard shortcuts, tab
   presets, an existing pane and agents' `OpenWidget` still open it.
5. **Groups:** hiding a parent hides its group from the bar and More. A child that was promoted (pinned on its own)
   stays pinned. Hiding a child removes it from its parent's flyout. If every child is hidden, the parent shows with
   an empty flyout, which reads as broken, so the parent hides too.
6. **Settings can't be hidden.** `defwidget@settings` is the way back to this section, so its row shows "always
   available" with no Hidden option. Settings is also on the main menu, but the bar is the obvious route.
7. **Unknown keys are kept.** A key in `widget:hidden` that matches no entry (a package removed, a widget renamed)
   stays in the list and is ignored, so reinstalling the package keeps it hidden. Old names are mapped the same way
   as `widget:pinned` (`knowledge` → `memory`, `remotes` → `connectors`).

## 4. The Settings UI

Settings → Widgets gets a new first group, **Widget bar**, above the existing packages list.

```
WIDGET BAR                                                    [Reset to defaults]
Choose what the bar shows. Pinned widgets are icons on the bar, More keeps them in
the More menu, Hidden removes them from both and from a pane's + menu.

[x] Show icons only

     ◆ Agent      ↑ ↓  [ Pinned | More | Hidden ]
     ◆ Swarm      ↑ ↓  [ Pinned | More | Hidden ]
     ◆ Messengers ↑ ↓  [ Pinned | More | Hidden ]
        Discord        [ In group | Pinned | Hidden ]
        ...
     ◆ Drone          [ Pinned | More | Hidden ]
     ◆ Settings       Pinned | More   · always available
     ◆ Acme Board     [ Pinned | More | Hidden ]   from package acme.board
```

- **Rows:** every top-level entry the bar knows (built-ins, package entries, user entries), in bar order: pinned
  first in `widget:pinned` order, then More in `display:order`, then hidden. Each row shows the entry's icon in its
  tint (`widgetIconStyle`), its label, and a three-way segmented control. Package entries say which package they come
  from.
- **Groups:** a parent's children are listed under it, indented. A child's states are In group (the default),
  Pinned (promoted) and Hidden.
- **Reorder:** pinned rows have Move left and Move right buttons (keyboard reachable), writing `widget:pinned`.
  Dragging stays on the bar itself.
- **Show icons only** moves here from Advanced (same `widget:icononly` key, and its search keywords come with it).
- **Installed widgets** (`ext@…`) can be More or Hidden; Pinned is disabled with a tooltip until §7 item 5 lands.
- **Reset to defaults** sends `{"widget:pinned": null, "widget:hidden": null}`. The bar falls back to the
  fresh-install pins (`display:pinned`), and nothing is hidden.
- **Search:** index entries `widgets.bar`, `widgets.bar.icononly` and `widgets.bar.reset`, with keywords "widget
  bar", "pin", "unpin", "hide", "More menu", "toolbar", "icons only".

### 4.1 Real time, both ways

- The group derives its rows from the same `fullConfigAtom().widgets` and `settingsAtom()` the bar uses, through the
  same shared helper (§6.1). There is no local copy of the lists, so it cannot drift. This avoids the drift of the
  title-bar menu, which keeps an optimistic copy that is never reset (`titlebar-context-menu.tsx:28`).
- A click writes with `SetConfigCommand`. srv broadcasts the merged settings immediately, and both the bar and the
  section re-render from that one event. The null fix in §6.2 makes Reset and every delete show at once, not 300 ms
  later.
- If a write fails, the controls keep showing what the config says and the error appears inline.
- Hand edits to `widgets.json` (a new entry, a package installed) or `settings.json` arrive through the same
  `Config` event and appear in the list without reopening Settings.

### 4.2 On the bar

- The right-click menu of a pinned icon, a More item and a flyout child gets **Hide from widget bar**, next to the
  existing Pin and Unpin. It writes the same keys.
- Hiding from the bar raises a short notification, "<Widget> hidden. Show it again in Settings → Widgets.", with an
  Undo button that puts back the exact pins and hidden list from before. Shown each time, since an accidental hide
  from a right-click is the case Undo is for.
- When More would be empty (everything is pinned or hidden), the More button is not shown. The bar can be just the
  pinned icons.

### 4.3 The "+" picker, live while open

The pane "+" picker and the empty-tab menu are drawn by the app's DOM context menu
(`frontend/app/host/js-context-menu.ts`), which used to render a fixed list. They now open with
`ContextMenuModel.showLiveContextMenu(build, ev)`: `build` reads the widget config, a Solid effect re-runs it when
the config changes, and `updateJsContextMenu` replaces the open menu's items in place. The menu keeps its position
(re-placed for the new height), a submenu that was open stays open if it still exists, and the tracking stops when
the menu closes by any path (`onJsContextMenuClosed`). If every item is gone, the menu closes.

## 5. Settings keys and types

| Key | Type | Meaning |
|---|---|---|
| `widget:pinned` | `string[]` short names (existing) | Pinned icons, in order. Absent means the fresh-install default. |
| `widget:hidden` | `string[]` full keys (new) | Entries hidden from the bar's surfaces. Absent means nothing hidden. |
| `widget:icononly` | `boolean` (existing) | Icons without labels. |

- Add typed fields for `widget:pinned` and `widget:hidden` to the Rust `SettingsType` (today `widget:pinned` lands in
  the catch-all `extra`, `types.rs:518-521`), to `schema/settings.json`, to the TS `SettingsType` (which removes the
  `as any` at every write), and to `settings-template.jsonc` with a comment.
- `widget:pinned` stays short names for compatibility with every existing `settings.json`. A later change can move it
  to full keys, which is also what package entries need (§7, item 4).

## 6. Implementation

### 6.1 One model for the bar

Add `getWidgetBarModel(settings, wmap)` in `action-widgets-config.ts`. It returns, in bar order,
`{ key, shortName, widget, state: "pinned" | "more" | "hidden" | "grouped", parent?, children? }` for every entry, with
the promotion, renamed-key and hidden rules applied once. Then rebuild on it:

- `getPinnedWidgets`, `getMoreWidgets`, `getChildWidgets` (the bar, More and flyouts);
- the title-bar "Pin Widgets" checklist (drop its own filter, `titlebar-context-menu.tsx:83-104`);
- the new Settings group.

The pane "+" picker and empty-tab menu (`buildPaneWidgetMenuItems`, `:221-255`) and the Launcher keep their own
inclusion rules under decision 4. They should still share the grouped and promoted logic from the model rather than
their own copies.

Writers: `pinWidget`, `unpinWidget`, `hideWidget`, `showWidget` (to More), `reorderPinned`, all in
`action-widgets-config.ts` and each sending one `SetConfigCommand` with every key it changes.

### 6.2 Backend

- Fix `merge_settings_into_current` (`config_watcher_fs.rs:200-218`) to **remove** keys whose new value is null, as
  `merge_settings_to_disk` does. Without it, the broadcast after a delete carries the old value. Add a unit test.
- Typed `widget:pinned` and `widget:hidden` on `SettingsType` (§5).
- srv itself doesn't filter on `widget:hidden`. The merged widget map stays complete, because the pane picker,
  Launcher, palette and `OpenWidget` still need hidden entries (decision 4). The rule is applied in the frontend
  model only.

### 6.3 Files

| File | Change |
|---|---|
| `frontend/app/window/action-widgets-config.ts` | `getHiddenKeys`, `isWidgetHidden`, `getWidgetBarModel`, `nextWidgetBarConfig`/`setWidgetBarState`, `movePinnedWidget`, `resetWidgetBar`; the bar, More, children and pane-picker lists skip hidden entries |
| `frontend/app/window/action-widgets.tsx`, new `widget-bar-hide.ts` | "Hide from widget bar" (and "Hide group…") in the pinned, More and flyout menus; the Undo notification. The More button already disappears when its list is empty |
| `frontend/app/window/titlebar-context-menu.tsx` | skips hidden entries; drops its optimistic copy once the saved pins catch up |
| `frontend/app/view/launcher/launcher.tsx` | skips hidden entries |
| `frontend/app/host/js-context-menu.ts`, `frontend/app/store/contextmenu.ts` | `updateJsContextMenu`, `onJsContextMenuClosed`, `showLiveContextMenu` (§4.3) |
| `frontend/app/element/pane-tab-picker.ts`, `frontend/app/tab/tabcontent.tsx` | open the "+" picker and the empty-tab menu as live menus |
| `frontend/app/view/settings/sections/widget-bar-settings.tsx` (+ `.scss`), `widgets-section.tsx`, `settings-index.ts` | the Widget bar group and its search entries |
| `frontend/app/view/settings/sections/advanced-section.tsx` | icon-only row removed (moved to Widgets) |
| `frontend/app/notification/usenotification.tsx` | `setNotificationAction`, so an Undo can be registered when raised |
| `crates/srv/src/backend/config_watcher_fs.rs` | null removal in the in-memory merge |
| `crates/srv/src/backend/wconfig/types.rs`, `schema/settings.json`, `frontend/types/srv-types.d.ts` | typed keys, read leniently (a malformed list is ignored, not fatal) |
| agentmux-docs `widgets` page | document Pinned, More and Hidden, and `widget:hidden` |

## 7. Cleanup found by the audit

In this change (small and needed for the above):

1. **Null in-memory merge** (§6.2).
2. **Four separate "which entries are top-level" filters** (bar, pane picker, title-bar menu, Launcher). Collapse
   into the model (§6.1).
3. **The title-bar menu's optimistic pinned copy** never resets and can drift (`titlebar-context-menu.tsx:28`).
4. **Untyped `widget:pinned`** (§5).

Separate follow-ups:

5. **Package widgets can't be pinned.** Pin logic always looks up `defwidget@${short}`
   (`action-widgets-config.ts:152,168`, `action-widgets-menu.ts:37`, `action-widgets.tsx:336`), and the title-bar
   menu lists only `defwidget@` keys (`:85`). So `ext@…` entries only ever appear in More. Fix by moving
   `widget:pinned` to full keys, reading short names as `defwidget@<short>` for old files.
6. **Two rename maps** that must agree: `RENAMED_PINS` in TS (includes `armory` → `connectors` + `memory`) and
   `RENAMED_WIDGETS` in Rust (no `armory`, `user_widgets.rs:54-58`). Keep one in Rust and send the canonical names,
   or at least add `armory` to Rust and a test that the two agree.
7. **Dead or leftover keys and fields:** `widget:showhelp` and `widget:*` (schema and types, no reader),
   `widget:order` (`MetaType`, template tests), the `widget:hidden@defwidget@…` example in a comment
   (`types.rs:518`, it would be confused with the new key), `FullConfigType.defaultwidgets` (never filled,
   `types.rs:846`), `ConfigState::set_config` (`#[allow(dead_code)]`, `state.rs:55`), `let _ = view;`
   (`widget_packages.rs:594`).
8. **Out-of-date schemas:** `schema/widgets.json` lacks `display:pinned` and `module` and sets
   `additionalProperties: false`, so it rejects valid files. `schema/settings.json` lacks `widget:pinned`.
9. **`display:hidden` naming:** it means "only inside its group", and the Launcher also uses it. Rename to
   `display:grouponly` (reading the old name), so it can't be confused with the user's Hidden.
10. **Comment drift:** `user_widgets.rs:116` says the user file can "hide" a package entry, but it can only replace
    it. The settings-key doc block is duplicated in `action-widgets.tsx:11-13` and `action-widgets-config.ts:8-10`.
11. **Palette labels** "Open Remotes" and "Open Hangar (Files)" (`command-registry.ts:175,182`) may predate the
    Connectors rename. Check them.

## 8. Tests

- `action-widgets-config.test.ts`: the model for pinned, More, hidden, grouped, promoted, renamed and unknown keys;
  hiding a pinned entry drops the pin; hiding a parent; hiding every child; Settings can't be hidden; package keys.
- `widget-bar-settings.test.tsx`: rows in bar order; the segmented control writes the right keys; the list re-renders
  when `fullConfigAtom` changes (simulated `Config` event) with no other action; Reset sends both nulls.
- `config_watcher_fs.rs`: `merge_settings_into_current` removes a key set to null.
- Live check in an isolated `task dev`: with Settings and the bar side by side, pin, unpin, drag, hide and reset from
  each side and see the other change at once. Edit `widgets.json` and `settings.json` by hand and see both change.
  Hide everything except Settings: More disappears, and Settings brings everything back.
- New coverage for the parts with none today: drag reorder and the title-bar menu.

## 9. Phases

1. **Model and backend fix:** `getWidgetBarModel`, the null fix, typed keys, the bar, More and the title-bar menu
   read the model. No visible change yet, except fixing the drift.
2. **Hidden:** `widget:hidden`, "Hide from widget bar" on the bar, More hidden when empty, the first-hide note.
3. **Settings → Widgets → Widget bar:** the group, reorder, icons only moved, Reset, search entries, docs.
4. **Follow-ups:** §7 items 5-11.

## 10. Open questions

1. ~~Should a hidden entry also leave the pane "+" picker?~~ Decided 2026-10-12: yes, and every widget list stays
   in step with the others, even with the "+" picker open (decision 4, §4.3).
2. Should the fresh-install default pin fewer widgets now that hiding exists? Today 11 entries are pinned out of the
   box (`config/widgets.json`, `display:pinned`).
3. Should the Widget bar group also set each entry's color, which today lives per pane type under Appearance?
