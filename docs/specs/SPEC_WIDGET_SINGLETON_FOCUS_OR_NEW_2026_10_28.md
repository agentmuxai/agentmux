# A widget click brings back the pane you have, or opens a new one: one rule, set by a `singleton` flag

**Status:** active. Approved by the operator 2026-10-29 ("those camper ones are good, approve all 3"); built in task #9110: the widget bar, More menu, flyout and palette first, then agents' `pane.open`. The operator said yes to the direction on 10-28; this fixes the details.
**Author:** Camper@narko (2026-10-28).
**Scope:** [`frontend/app/window/action-widgets-config.ts`](../../frontend/app/window/action-widgets-config.ts) (`handleWidgetSelect`, `FOCUS_EXISTING_VIEWS`), [`frontend/app/store/block-component-registry.ts`](../../frontend/app/store/block-component-registry.ts) (`openOrFocusPaneIdByView`), [`frontend/app/store/command-registry.ts`](../../frontend/app/store/command-registry.ts) (the palette's open entries), the widget config (`crates/srv/src/config/widgets.json`, `WidgetConfigType`), `crates/srv/src/backend/widget_packages.rs` (extension panes), and the `pane.open` command agents use.

## 1. What exists

- **A hardcoded set decides.** `handleWidgetSelect` focuses the existing pane for `connectors`, `memory` and `warden` (`FOCUS_EXISTING_VIEWS`), and for the Swarm only when the entry names a section (the Kanban and Vault buttons). Every other widget calls `createBlock` and opens another pane each click.
- **The lookup is narrow.** `openOrFocusPaneIdByView` looks through the panes mounted in the **active tab's layout**; a pane in another tab falls through and a second one is created. Other windows are never searched. (#9107, PR #5752, makes the found pane *appear*: it restores it if minimized, un-magnifies another pane that hid it, and switches to its tab in a stack. It does not change what is searched.)
- **The palette ignores the set.** Its open entries (Help, Swarm, Connectors, Tower, System info and the rest in `command-registry.ts`) call `createBlock` directly, so the palette's "Connectors" opens a second Connectors pane while the bar's focuses the first.
- **A `singleton` field exists, and nothing reads it.** An extension manifest's pane has `singleton` (`ManifestPane`, copied to the package and its generated TS type, default false), but no frontend code consults it: an extension pane marked singleton still opens anew. The widget config (`widgets.json`, `WidgetConfigType`) has no such field.
- **Agents' `pane.open`** always creates a block.

## 2. The rule

> A widget either **brings back the pane you already have** (`singleton: true`) or **opens a new one each time** (the default). The widget config says which. The widget bar, the More menu and flyouts, the command palette and an agent's `pane.open` all follow the same flag.

**Which are singletons** (the flag in `widgets.json`; question 1 for the borderline ones):

| Singleton: management panes, one of each is the point | Opens new: work panes, many of each is the point |
|---|---|
| Warden (and its Vault entry), Memory, Connectors, Settings, Swarm (and its Kanban entry), Toolchain, Help, Drone | Agent, Terminal, Browser (and the messenger entries, which are browsers), Editor, Files (files) |
| Tower (one overview of everything) | System info (one per host is legitimate) |

An **installed widget's panes** follow the `singleton` field their manifest already carries, so the field starts to mean what it says; a widget author who wants one pane sets it.

## 3. What "bring back" does

Same steps for every singleton, in this order, wherever the click came from:

1. **Find it.** Search the active tab first, then the window's other tabs, then other windows (the reveal path already crosses windows: `block:reveal`). The first match wins; with several (two Memory panes from before this change) the one used last wins.
2. **Show it.** `showBlockInPane`: restore it if minimized (a merged minimized group, #8674, expands with all its tabs), un-magnify another pane that hides it, switch its pane to its tab if it is a stack member.
3. **Go to it.** Switch the window tab (and raise the other window) when it is elsewhere, then focus it. Question 2 asks whether the click should instead open one *here*.
4. **Apply the click's section.** A Vault or Kanban entry switches the pane to its section, as now.
5. **Nothing found:** open one, as now.

A **magnified** widget entry (`magnified: true` in the config) opens new, as it does today: the entry asked for a full-size pane.

## 4. Getting a second one on purpose

- **Shift-click** on a bar entry, a More item or a palette entry opens a new pane even for a singleton.
- The palette also lists "New <widget>" for each singleton, so it is findable without the modifier.
- An agent's `pane.open` takes `new: true` for the same effect; without it, it gets the existing pane's block id back (the open is idempotent for singletons) and the pane is shown.
- Dragging a pane out, duplicating a pane and the pane menu's Split are not a widget click and are unchanged.

## 5. Interplay

- **Shortcuts pane (#8191).** A singleton tile in a Shortcuts pane brings back the existing pane and **closes the Shortcuts pane** (it has no content to keep), instead of replacing itself with a duplicate. Shift+Enter opens a new one in place. The last-used order records either.
- **Minimize merging (#8674).** Bringing back a pane that was merged into a minimized group restores the group pane and switches to the tab; nothing new is needed there, since `showBlockInPane` is the same path. A singleton never loses its place in the strip.
- **Pane-tab pickers.** The "+" picker and tab presets are a "new pane" gesture: they open new regardless of the flag (question 3).

## 6. The change

1. Add `singleton?: boolean` to `WidgetConfigType` (Rust source and generated type); set it in `widgets.json` for the table's left column; a user's own `widgets.json` can override it for any entry (the file is already theirs to edit).
2. One function decides, used by every caller: `openWidget(widget, { forceNew })` reads `singleton` (built-in config or the extension manifest's pane) instead of `FOCUS_EXISTING_VIEWS`; the bar, More, the flyout, the launcher and the palette call it.
3. `openOrFocusPaneIdByView` searches across tabs and windows (step 1 above) and reveals through the existing reveal path.
4. The palette's open entries go through `openWidget`; "New <widget>" entries and Shift-click pass `forceNew`.
5. `pane.open` for a singleton view returns the existing block unless `new: true`.
6. Remove `FOCUS_EXISTING_VIEWS`.

## 7. Questions for the operator

1. **The borderline panes.** Singleton: Settings, Help, Toolchain, Tower, Drone. New each time: System info (per host), Files, and every browser-based messenger. *Recommend that.* The alternative for Tower and Help is new each time; tell me which you want flipped.
2. **A singleton that is in another tab.** Go to it, or open one in the current tab? *Recommend go to it,* as you said: no duplicate, and the tab switch shows where it was. Shift-click is the way to have one here.
3. **Does the "+" picker follow the flag?** It is an explicit "new pane" gesture. *Recommend no:* it always opens new. The bar and palette follow the flag.
4. **Existing duplicates.** A tab that already has two Memory panes keeps both; the click brings back the one used last and never closes the other. *Recommend that;* we do not merge or close anything the person opened.
5. **Should the flag be visible in Settings → Widgets?** A "Reuse the open pane" toggle per entry. *Recommend later:* the config-file override is enough for now.

If the Warden split (task #9204, Loap's spec, not yet approved) goes ahead, "Warden" in the table reads "Vault, History, Audit" (each a singleton, not pinned by default); the rule needs no other change, since each is its own widget entry with its own flag.

## 8. Verification

- **Unit:** the decision (singleton vs new vs `forceNew`, magnified opens new); the lookup across tabs and windows with the last-used winning; the minimized and stacked cases; an extension pane's `singleton` honoured; `pane.open` idempotent for a singleton and `new: true` forcing; the palette and bar share the function; `FOCUS_EXISTING_VIEWS` gone.
- **Live** in my own task dev build: minimize Warden, click Warden on the bar (restores); click Memory while it is in another tab (goes there); Shift-click Memory (a second one); a Terminal click twice (two terminals); screenshots.

## 9. Not in scope

A Settings toggle for the flag; closing or merging existing duplicates; changing what a section entry (Vault, Kanban) is; per-window singletons.

## 10. Build notes (task #9110)

- **First PR:** the `singleton` field (Rust `WidgetConfigType`, its TS type, `widgets.json` for the table's left column plus Drone), one `openWidget(widget, { forceNew })` for the bar, More and the flyout (Shift-click), the palette's open entries and "New <widget>" entries, an extension pane's manifest `singleton`, and the lookup reaching the window's other tabs. `FOCUS_EXISTING_VIEWS` is gone.
- **Follow-ups:** agents' `pane.open` (idempotent for a singleton, `new: true` to force), and reaching a singleton in **another window** (the lookup covers this window's tabs today; another window needs srv's block list). Picking the most recently used among several is also left: the pane in the active tab wins, else the most recently mounted.
