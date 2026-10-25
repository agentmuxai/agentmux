# SPEC: Icons in every right-click (context) menu

**Date:** 2026-10-25
**Author:** Agent1 (UI kit, theme), for Kanban #3376; the build is #3377.
**Status:** active. Approved by the operator on 2026-10-25 ("lets move forward with the fa right-click menus, they look great"): Font Awesome, D1 to D6 as written. A custom icon set was explored (#3478) and shown; the operator chose Font Awesome, and the violet and teal palette tones proposed there are not part of this build. Built in the five PRs of section 7, task #3377.
**Grounded in:** `origin/main` at `39336f738`. Every path and line below was read from that commit.
**Related:** `REPORT_CONTEXT_MENU_GAP_AUDIT_2026_08_07.md` (which menus exist and where they have gaps), `SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md`.

## 0. Summary

**The request (operator, 2026-10-25).** "Let's add icons to the right click menus. This likely is a larger task."

**What the inventory found.**
1. **No right-click menu in the app is a native OS menu.** Every one is drawn in the page, so Windows, macOS and Linux get the same menu from the same code. The CEF host's `show_context_menu` command only logs "handled in JS overlay" (`crates/cef/src/ipc.rs:402`). The only native menu is the system tray's, which is not a right-click menu in the app (section 5).
2. **Four renderers draw them**, and none can show an icon today: the JS overlay (`frontend/app/host/js-context-menu.ts`), `PopoverMenu` (`frontend/app/element/popover-menu.tsx`), the small list in `frontend/app/components/context-menu.tsx`, and the window-tab colour panel (`frontend/app/tab/tab.tsx:69`). `FlyoutMenu` already draws an icon (`flyoutmenu.tsx:430`), and the JS overlay already shares its stylesheet, including a `.menu-item-icon` rule (`flyoutmenu.scss:111`).
3. **About 40 menus are listed in section 1**, and most go through the JS overlay (`ContextMenuModel`). Adding one optional `icon` field there, and drawing it, covers most of them. The other three renderers each need the same small change.

**The design, in five lines.**
1. One optional `icon` on a menu item: a Font Awesome name, the same strings `makeIconClass` takes (`"copy"`, `"regular@window-maximize"`). The icon set is the one the toolbar already uses.
2. A leading icon slot. If any item in a menu has an icon, every row reserves the slot, so labels line up. Checkable rows keep their check in the slot.
3. One shared table of icons by action (`menu-icons.ts`), so Copy is the same icon in every menu.
4. Icons are decoration: `aria-hidden`, and labels are unchanged.
5. Built in five PRs: the plumbing first, then the menus by area (section 7).

### 0.1 Decisions I made that the operator may want to change

| # | Decision | Alternative |
|---|---|---|
| D1 | Font Awesome Solid, 14 px slot, the same as the other menus' icons. | A custom icon set per item (more work, no gain). |
| D2 | A menu where no item has an icon looks as it does today (no empty slot). | Reserve the slot everywhere. |
| D3 | Checkbox and radio rows keep their check in the slot and take no icon. | Icon after the check (wider menus). |
| D4 | The system tray menu is left alone in this build (section 5). | Add icons there as a sixth PR. |
| D5 | Menus that only offer "Copy ..." (Swarm, Drone, Install log) get the Copy icon on each row, not a single header. | Leave single-purpose copy menus bare. |
| D6 | Danger rows (Delete, Remove) take the danger colour on the icon as well as the label. | Plain icon. |

## 1. Inventory

"Overlay" is `ContextMenuModel.showContextMenu` or `showLiveContextMenu` (`frontend/app/store/contextmenu.ts:62,76`), drawn by `showJsContextMenu` (`js-context-menu.ts:76`).

| # | Menu | Built at | Renderer | Items |
|---|---|---|---|---|
| 1 | Pane header and grip, every pane | `block/blockframe.tsx:206`, `block/pane-actions.ts:157` | Overlay | Copy, Paste, Split (4 directions; Up and Left are being removed), Magnify / Un-Magnify, Close Pane, Inspect Element, "Pane Color" submenu (swatches), "Use for all X panes", Remote settings, Pin / Unpin section, per-view settings |
| 2 | Pane body, every pane | `blockframe.tsx:1272` | Overlay | The view's items, then the same pane group as 1 |
| 3 | Browser pane body | `view/browser/browser-model.ts:833` | Overlay | Back, Forward, Reload, Cut, Copy, Paste, Copy Link Address, Print, View Page Source, Inspect Element, then the pane group |
| 3a | Browser Home button | `browser-nav-bar.tsx:525` | Overlay | Set this page as Home |
| 4 | Agent pane body | `view/agent/agent-model.ts:987` | Overlay | Agent History, Quick Fork, then the pane group |
| 4a | Agent transcript selection | `agent-view.tsx:1202` | Overlay | Copy |
| 4b | Agent error row | `virtualization/DocumentRow.tsx:300` | Overlay | Copy error |
| 4c | Agent composer | `components/AgentFooter.tsx:1467` | Overlay | Undo, Cut, Copy, Paste, Open Clipboard URL |
| 4d | Agent shell drawer | `AgentShellSubblock.tsx:431` | Overlay | Copy, Paste, Magnify, Close, Inspect |
| 4e | Agent picker template card | `AgentPicker.tsx:748` | Overlay | Hide template |
| 5 | Sysinfo pane | `sysinfo-model.ts:481` | Overlay | "Plot Type" / Simple / Advanced radio rows |
| 6 | Tower header | `tower-instance.tsx:15` | Overlay | Share processes (checkbox) |
| 6a | Sandboxed widget pane | `sandboxed-widget-host.tsx:235` | Overlay | Items the widget supplies |
| 7 | Editor body | `editor-model.ts:1344` | Overlay | Word Wrap (checkbox), Open from remote |
| 8 | Editor file tree | `editor-view.tsx:842` | `components/context-menu.tsx` | New File, New Folder, Refresh, Open, Open to the Side, Open in New Tab, Copy Path, Copy Relative Path, Open in Terminal, Reveal, Collapse Folder, Rename, Delete |
| 9 | Editor and media doc-tab strip | `doc-tabs/DocTabStrip.tsx:58` | Overlay | Move to New Pane, Close Tab |
| 10 | Files row | `view/files/files-view.tsx:589` | `components/context-menu.tsx` | Open (several forms), Open terminal here, Reveal, Mention in agent, Attach to agent, Cut, Copy, Copy path, Rename, Move to Trash, Delete permanently |
| 10a | Files empty area | `files-view.tsx:712` | `components/context-menu.tsx` | New folder, New file, New tab here, Paste, Open terminal here, Reveal, Copy folder path, Show / Hide hidden files, Refresh, Undo |
| 11 | Remotes row | `view/remotes/remotes-view.tsx:518` | `components/context-menu.tsx` | New terminal, Browse files, Sessions, Connect / Disconnect, Pin / Unpin, Hide / Unhide, Forget, Remove helper, Settings |
| 11a | Remote sessions list | `term/hostSessions.ts:24` | Overlay | One row per session, with Open in a New Pane and End Session |
| 12 | Title bar and empty tab-bar space | `window/titlebar-context-menu.tsx:20` | `PopoverMenu` | New Window, New Tab, Show Icons Only (checkbox), "Pin Widgets" section of checkboxes |
| 13 | Widget bar pinned icon | `window/action-widgets.tsx:452` | Overlay | Open in New Window, Open in Floating Pane, Open in New Tab, Unpin from bar, Hide from widget bar |
| 13a | Widget bar "More" and group flyout | `action-widgets.tsx:368` | `PopoverMenu` | The same set; Pin to bar or Unpin from bar |
| 14 | Window tab pill | `tab/tab.tsx:69,289` | `TabContextPanel` | Colour palette, Rename, Close menu (the two rows use emoji glyphs today) |
| 15 | Empty tab | `tab/tabcontent.tsx:39` | Overlay | One row per widget, submenus for groups |
| 16 | Pane-tab "+" | `element/pane-tab-picker.ts:31` | Overlay | The same widget list |
| 17 | Swarm rows (a to i) | `view/swarm/swarm-view.tsx:507` and others | Overlay | Copy name / ID / command / schedule / todo; the Supervisor mode radios |
| 18 | Drone nodes and runs | `view/drone/drone-view.tsx:503,1031` | Overlay | Copy task / URL / body / expression / template / variables / ID / error / output |
| 19 | Instance panel, closed window | `statusbar/InstancePanel.tsx:719` | Overlay | Remove from workspace |
| 20 | Install log | `element/install/LogView.tsx:194` | Overlay | Copy, Copy All |
| 21 | MCP server card | `view/mcp/mcp-manager.tsx:123` | Overlay | Edit, Bind to agent (submenu), Delete |
| 22 | Account card and row | `view/accounts/accounts-manager.tsx:77` | Overlay | Bind to Agent (submenu), Copy account ID, Delete account |
| 23 | Any text field, and the app root | `store/contextmenu.ts:206`, `app.tsx:448` | Overlay | Cut, Copy, Paste, Open Clipboard URL |

Panes with no menu of their own (all Warden views including Kanban and Vault, the history views, media) fall through to row 2. Click-opened menus (the hamburger, bookmarks, the account picker) are not right-click menus and are out of scope; the first two are `FlyoutMenu`s, which can show icons already.

## 2. The icon set

- **Font Awesome**, vendored in `public/fontawesome/css/` (solid, regular, brands, `pro-icon-aliases.css`, `custom-icons.min.css`), loaded by `index.html`. Names are plain strings; `makeIconClass` (`frontend/util/util.ts:55`) turns `"copy"` into `fa fa-solid fa-copy` and accepts `"regular@name"`, `"brands@name"` and `"custom@name"`. That is also what toolbar buttons use (`IconButtonDecl.icon`, `element/iconbutton.tsx:37`; `Button icon="..."`).
- **Same style and size as the other menus' icons:** `fa-solid fa-fw menu-item-icon`, 14 px wide, `var(--text-xs)`, 70 % opacity, drawn in the row's text colour, so it follows the theme, the hover colour and the disabled opacity with no extra rules. The rule exists in `flyoutmenu.scss:111`.
- **All the icons proposed below resolve in the vendored CSS.** I checked every name against `public/fontawesome/css/*.css`. Names that exist only through `pro-icon-aliases.css` (for example `table-rows`) work because that file maps them to free glyphs; where a free equivalent is clearer I name that instead.

## 3. Data model and rendering

One optional field, `icon?: string`, on each item type, with the same meaning everywhere.

| Renderer | Change |
|---|---|
| Overlay | `ContextMenuItem` and `NativeContextMenuItem` in `frontend/types/custom.d.ts` get `icon?`. `_convertAndRegisterMenu` (`contextmenu.ts:33`) copies only a fixed list of fields, so it must copy `icon` too. `renderItems` in `js-context-menu.ts` (around `:197`) draws `<i class="... menu-item-icon">` in the leading slot, before the label. |
| `PopoverMenu` | `PopoverMenuActionItem` gets `icon?`; `PopoverMenuItemRenderer` (`popover-menu.tsx`, around `:145`) draws it where it draws the check today. |
| `components/context-menu.tsx` | Its own `ContextMenuItem` (a different type with the same name) gets `icon?`; the row gets an icon span before `ctx-menu-label`. |
| `TabContextPanel` | Rename and Close menu get FA icons (`pen`, `xmark`) in place of the emoji glyphs. |
| `FlyoutMenu` | Nothing: it has `icon` already. |

**The slot.** The leading slot is one column, 14 px. A menu where no item has an icon is drawn as today (D2). A menu where any item has one reserves the slot on every non-separator row, so labels line up. A checkbox or radio row shows its check in the slot and takes no icon (D3); a row with a `swatchColor` shows the swatch there. Submenu rows may take an icon and still show their chevron on the right. A `sublabel` or shortcut stays right-aligned as now.

**One table.** `frontend/app/util/menu-icons.ts` exports the icon for each action (`copy`, `paste`, `cut`, `undo`, `close`, `rename`, `delete`, `refresh`, ...). Builders use the constant instead of a literal, so the same action is the same icon in every menu, and a change is one edit. The widget lists (rows 15, 16, 13a) use each widget's own `icon`, which `WidgetConfigType` already has (`srv-types.d.ts:1536`).

## 4. The icons

First the shared vocabulary: an action gets this icon wherever it appears.

| Action | Icon | | Action | Icon |
|---|---|---|---|---|
| Copy (any "Copy ...", Copy All) | `copy` | | Close (pane, tab, menu) | `xmark` |
| Copy Link Address | `link` | | Rename, Edit | `pen` |
| Paste, Paste (limits) | `paste` | | Delete, Move to Trash, Remove, Forget | `trash` |
| Cut | `scissors` | | Delete permanently | `trash-can` |
| Undo | `rotate-left` | | Refresh, Reload | `arrows-rotate` |
| Open (generic), Open in New Tab | `arrow-up-right-from-square` | | Back / Forward | `arrow-left` / `arrow-right` |
| Open folder, Reveal, Browse files | `folder-open` | | New folder | `folder-plus` |
| Open terminal, New terminal, Open in Terminal | `terminal` | | New file | `file-circle-plus` |
| Pin to bar | `thumbtack` | | Unpin | `thumbtack-slash` |
| Hide | `eye-slash` | | Unhide, Show hidden files | `eye` |
| Magnify | `regular@window-maximize` | | Un-Magnify | `regular@window-restore` |
| Split Down | `table-rows` | | Split Right | `table-columns` |
| Inspect Element | `bug` | | View Page Source | `code` |
| Settings | `gear` | | New Tab, New Window | `plus` |

Then the rest, by menu:

| Menu | Item | Icon |
|---|---|---|
| 1 | Pane Color (submenu) | `palette` |
| 1 | Use for all X panes | `clone` |
| 1 | Remote settings | `gear` |
| 3 | Print | `print` |
| 3a | Set this page as Home | `house` |
| 4 | Agent History | `clock-rotate-left` |
| 4 | Quick Fork | `code-branch` |
| 4c | Open Clipboard URL | `arrow-up-right-from-square` |
| 4e | Hide template | `eye-slash` |
| 5 | Plot Type (submenu parent) | `chart-line`; its radio rows keep the check |
| 7 | Open from remote | `network-wired` |
| 9 | Move to New Pane | `arrow-up-right-from-square` |
| 10 | Open in Editor | `pen-to-square` |
| 10 | Open with default app | `arrow-up-right-from-square` |
| 10 | Mention in agent | `at` |
| 10 | Attach to (agent) | `paperclip` |
| 10, 10a | Copy path, Copy folder path | `copy` |
| 10a | New tab here | `plus` |
| 11 | Sessions | `list` |
| 11 | Connect / Disconnect | `plug` / `plug-circle-xmark` |
| 11 | Remove helper | `trash` (danger) |
| 11a | Open in a New Pane | `arrow-up-right-from-square` |
| 11a | End Session | `circle-stop` (danger) |
| 13 | Open in New Window | `arrow-up-right-from-square` |
| 13 | Open in Floating Pane | `window-restore` |
| 13 | Hide from widget bar | `eye-slash` |
| 14 | Colour swatches | none (they are the swatch) |
| 19 | Remove from workspace | `xmark` |
| 21, 22 | Bind to agent | `link` |
| 21, 22 | Delete..., Delete account... | `trash` (danger) |
| 12, 6, 7, 17h | Show Icons Only, Pin Widgets rows, Word Wrap, the Supervisor and view radios | check only, no icon |

Items not named have no sensible icon and stay bare in a menu that has icons (their slot is empty). Rows I expect to stay bare: the widget-supplied items of a sandboxed widget (6a) unless the widget SDK adds an `icon` field, which this build does not.

## 5. Native menus

- **Right-click menus:** nothing native to change on any OS (section 0, point 1). The stale comments that call them "native OS context menus" (`window/action-widgets-menu.ts:7`, `block/blockframe.tsx:604`) get corrected in PR 1.
- **System tray (`crates/launcher/src/tray/`):** built with `muda` and `tray-icon` on Windows and macOS and with `ksni` on Linux, with plain text items (no icon is set on any item today). `muda` has an `IconMenuItem` that takes an image, and the Linux tray protocol lets an item name an icon, but that needs image assets per platform, light and dark variants, and a native test pass on three OSes. It is not a "right-click menu" in the app and has five items. Left out (D4); a follow-up if the operator wants it.
- **Browser panes:** CEF's own menu is suppressed (`crates/cef/src/client/context_menu.rs:38`) and the page-level event feeds the overlay (row 3), so web-page menus get icons through the same field.

## 6. Accessibility and behaviour

- The icon element is `aria-hidden="true"` and has no text; the row's accessible name is its label, unchanged. A row never relies on the icon to be understood.
- Keyboard handling, focus, hover, disabled state and the check semantics are untouched. The icon takes `currentColor` and the existing disabled opacity.
- Reduced-motion and high-contrast need nothing new: the icons are static glyphs in the text colour.
- Nothing changes in the hamburger or the bookmark flyout.

## 7. Build split

| PR | What | Menus |
|---|---|---|
| 1 | The plumbing: `icon?` on the four item types, `_convertAndRegisterMenu` copies it, the three renderers draw the slot, `menu-icons.ts` with the shared table, a test per renderer (slot only when an icon is present; a checkable row keeps its check; aria-hidden; disabled and danger rows), the stale comments corrected. No visible change yet. | none |
| 2 | The pane group and the agent pane: the shared pane menu, browser, agent, shell drawer, doc tabs. Lands after the split removal (Loap), or rebases over it. | 1, 2, 3, 3a, 4 to 4e, 9 |
| 3 | Files and code: Files, the editor file tree, editor, Remotes, remote sessions. | 7, 8, 10, 10a, 11, 11a |
| 4 | Window chrome and widgets: title bar, widget bar, More and flyout, window tab panel (emoji out), empty-tab and "+" widget lists. | 12, 13, 13a, 14, 15, 16 |
| 5 | The rest: Swarm, Drone, status bar, install log, MCP, accounts, text fields. | 5, 6, 17 to 23 |

Each PR has a test that its builder's items carry the icons in the tables (a builder-level test, not a screenshot), so a later menu change that drops an icon fails a test. A PR is small enough to review in one sitting and is independent of the next after PR 1.

## 8. Tests

- **PR 1:** for each renderer, a menu with no icon draws no slot; one icon reserves the slot on every row; a checkbox row's check sits in the slot; a separator has none; the icon is `aria-hidden`; a danger row's icon takes the danger colour; the overlay's conversion keeps `icon` on nested submenu items.
- **PRs 2 to 5:** for each builder, every item with an entry in section 4 carries that icon; the widget lists carry the widget's own icon.
- **By hand** (once per PR, in my own `task dev` build): a screenshot of the menu in light and dark themes, to see that labels align and the icon reads at 14 px.

## 9. Open points

1. **The Magnify glyph.** The header button draws a custom restore glyph (`PaneRestoreGlyph`, a JSX element). The menu takes strings only, so it uses the Font Awesome window glyphs. If the operator wants the identical glyph, `icon` can accept an element, as `IconButtonDecl.icon` does.
2. **Sandboxed widgets (6a)** supply their own items. Their menus stay without icons until the widget SDK has an `icon` field, which would be its own change.
3. **The split removal.** Loap's #3375 (PR 5463) removes Split Up and Split Left from these menus and narrows the split direction to down and right. PR 2 touches the same file (`block/pane-actions.ts`) and rebases over it; Split Down and Split Right get their icons there.

## 10. Evidence

Read at `39336f738`: `frontend/app/host/js-context-menu.ts:76,197-244`, `frontend/app/store/contextmenu.ts:33-62,76,206,307`, `frontend/types/custom.d.ts:674-700,795-815`, `frontend/app/element/popover-menu.tsx`, `frontend/app/components/context-menu.tsx:95-125`, `frontend/app/element/flyoutmenu.tsx:430`, `frontend/app/element/flyoutmenu.scss:105-160`, `frontend/util/util.ts:55`, `crates/cef/src/ipc.rs:402`, `crates/cef/src/client/context_menu.rs:38`, `crates/launcher/src/tray/`, and every builder named in section 1.
