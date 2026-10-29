# The Shortcuts pane: a split opens tiles for every pane type and widget, last used first

**Status:** active. Approved by the operator 2026-10-29 (task #8191); built in task #8192 by Camper-2@narko. A singleton tile (Memory, Connectors...) brings back the existing pane and closes the Shortcuts pane; Shift-click or Shift+Enter opens a new one in place instead, and a singleton that is not open opens in place.
**Author:** Camper@narko, at the operator's request (2026-10-28).
**Scope:** [`frontend/app/view/shortcuts/shortcuts.tsx`](../../frontend/app/view/shortcuts/shortcuts.tsx) (the existing tile view, renamed from `launcher.tsx`), [`frontend/app/store/keymodel-blockcreate.ts`](../../frontend/app/store/keymodel-blockcreate.ts) and [`frontend/app/store/command-registry.ts`](../../frontend/app/store/command-registry.ts) (the split shortcuts and palette entries), [`frontend/app/block/pane-actions.ts`](../../frontend/app/block/pane-actions.ts) (the pane menu's Split), [`frontend/app/block/split-block-def.ts`](../../frontend/app/block/split-block-def.ts).

The operator's words (10-28): *"I think we should have a generic pane that simply has shortcuts to all the other panes. It would show tile icons of the widgets with the last used first. It doesn't have its own widget, instead, it only shows when users use the "Split up/right" feature. In that case, the new pane is the generic."* Their name for it: **the Shortcuts pane**. That is its name in the UI, this spec and the code.

## 1. What exists

Most of this pane is already built, under another name:

- **A tile view exists**: the `launcher` pane (`launcher.tsx`, native, headerless: its search box is its top edge). It shows a grid of tiles for the widgets in the widget configuration, in their `display:order`; typing filters by label; arrow keys move, Enter opens; a group (Messengers) drills down and Escape backs out; choosing a tile **replaces the launcher in place** with that pane (`replaceBlock`), same size and position. Hidden widgets (Settings → Widgets) are left out, like the bar and the "+" picker.
- **Nothing opens it by itself.** It is reachable only by the setting `app:defaultnewblock = launcher` (the "New pane" shortcut and the split shortcuts' fallback). It is in no widget bar and no palette.
- **What a split opens today** differs by path: the pane menu's **Split Down / Split Right** copies the source pane (`copiedSplitBlockDef`: a terminal splits into a terminal in the same directory); an **agent** pane splits into a fresh agent picker (`splitBlockDef`, SPEC_AGENT_PANE_SPLIT_OPENS_PICKER); the **keyboard** splits (`split:right`, `split:down`, ⌘D / Ctrl+Shift+D) and the **palette** split a new terminal, or the configured new block.
- **No last-used order is recorded** for panes or widgets (the agent list has its own recency, `agent-recency.ts`).

## 2. What changes

1. **Every split opens a Shortcuts pane.** The pane menu's Split Down / Split Right, the two split shortcuts and the palette's Split Right / Split Down all create a Shortcuts pane next to the source (down and right are the only directions that exist today; up and left appear when the layout gets them). This replaces the copy of the pane and the agent-picker split.
2. **The tiles are ordered by last use.** The pane type or widget the person opened most recently comes first, then the rest in the widget configuration's order. Last use means opened from anywhere: a tile here, the widget bar, the "+" picker, a shortcut. A new `launcher:recent` list (the last 12 widget keys, newest first) is kept in the user's settings, so it follows the person to every window, and is updated where a widget is opened. A widget that was never used has no entry and keeps its configured place.
3. **Tiles for every pane type and every installed widget.** The grid shows what the "+" picker offers: the built-in pane types (agent, terminal, editor, Hangar/files, browser, Swarm, Memory, Connectors, Warden, Tower, sysinfo, and the rest of the widget configuration) and the panes of installed widgets (`ext:<id>/<pane>`), each with its icon and label. An installed widget the person has not approved is not shown. The existing search, arrow keys, Enter and group drill-down stay.
4. **Choosing a tile replaces the Shortcuts pane in place** (as today). The **Agent** tile opens the agent picker in the same place, as the "+" menu's Agent does. A Terminal or Hangar tile inherits the directory and connection of the pane that was split (the source's block id travels in the new pane's meta as `shortcuts:from`), so splitting a terminal and choosing Terminal still lands in the same directory, as the copy did.
5. **Closing it closes the split.** A Shortcuts pane has no content to lose: its close button and the close shortcut remove it with no prompt, and the neighbour takes its space as for any pane. Escape with an empty search closes it too when it was opened by a split (question 3).
6. **No entry of its own.** It is not in the widget bar, the "+" picker or the palette; only a split opens it. The setting value `app:defaultnewblock = launcher` keeps working (it is read as `shortcuts` too), so New pane can still open it for those who want that.
7. **It saves and restores like any pane.** The pane is an ordinary block: a Shortcuts pane left open comes back as one after a restart (question 4).
8. **The name.** The view is renamed Shortcuts in the UI and the code (`LauncherViewModel` becomes `ShortcutsViewModel`, `launcher.tsx` becomes `shortcuts.tsx`, the strings, tests and the settings label). `launcher` stays as an alias of the view name so layouts and the setting that already say `launcher` keep loading.
9. **Not affected:** an agent opening a pane for itself (`Pane` open, `pane.open`), a drag-created pane, tab presets and floating panes.

Motion follows the app's motion policy (`shouldReduceMotion("transition")`, not the OS setting): the tile hover and the swap to the chosen pane animate unless motion is off.

## 3. Questions for the operator

1. **Should an agent pane's split also open the Shortcuts pane?** Today it opens the agent picker. *Recommend yes:* one behaviour for every pane, and the Agent tile is one keypress from the picker.
2. **Should a terminal's split still keep its directory?** *Recommend yes,* through the Terminal tile inheriting it (§2.4); the alternative is a plain terminal from the home directory.
3. **Should Escape on an empty Shortcuts pane close it?** *Recommend yes* for a pane a split opened (it is a question the person can dismiss), no for one opened through `app:defaultnewblock`.
4. **Should a Shortcuts pane left open survive a restart?** *Recommend yes:* it is an ordinary pane and one rule is simpler than a special case; a restored empty Shortcuts pane is one Escape away from gone.
5. **Should it have a widget-bar or palette entry?** *Recommend no,* as you said: a split is the only way in.

## 4. Build outline (#8192)

1. Rename the view to Shortcuts, with the `launcher` alias.
2. The three split paths create `{ view: "shortcuts", "shortcuts:from": <source block id> }`; remove the copy and the agent-picker special case for splits.
3. The Terminal and Hangar tiles read `shortcuts:from`'s directory and connection.
4. The `launcher:recent` list, written where widgets are opened, read for the tile order.
5. Installed widgets' panes in the grid.
6. Escape to close, the no-prompt close.

## 5. Verification

- **Unit:** the tile order (a used widget first, an unused one in its configured place, a hidden one absent, the list capped at 12); the three split paths all produce a Shortcuts block with `shortcuts:from`; the Terminal tile's meta carries the source's directory and connection; `launcher` as a view name and as the setting value still resolve.
- **Component:** the grid lists built-in and installed panes; a tile replaces the pane in place; Escape closes a split-opened pane and only clears the search otherwise; no widget-bar or palette entry exists.
- **Live** in my own task dev build: split a terminal (down and right) and an agent pane, choose Terminal (same directory) and Agent (the picker), restart and see the restored Shortcuts pane, with screenshots.

## 6. Not in scope

Reordering or pinning tiles by hand (the widget bar's pins and Settings → Widgets already do that); a Shortcuts pane for agents' own `Pane` open; the widget bar itself.
