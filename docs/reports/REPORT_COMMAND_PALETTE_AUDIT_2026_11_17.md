# Command palette audit: what it lists, what it misses (task #47920)

Date: 2026-11-18. Author: Agent2@narko.

**Status:** implemented in #7713 (the entries and the two guards this report asks for).

Measured on a `task dev` build of main at `c0ea72cf47`, in its own isolated home, with made-up agents. Palette entries were read from the open palette (Ctrl+P) with dev mode on (the build's default) and with `app:devmode` set to `false`; the list again with an agent pane focused. Script: `scratch/palette-enum.mjs` (and `palette-enum2.mjs`) in the author's workspace.

## 1. What the palette lists today

70 commands plus one row per agent (72 rows with two agents). Dev mode off removes exactly one row.

| Category | Entries |
|---|---|
| Open (20) | Open Terminal, Agent, System Info, Tower, Kanban, Vault, History, Audit, Jobs, Help, Reviews, Atlas, Swarm, Files, Remotes, Toolchain; New Tower, New Kanban, New Help, New Swarm |
| Split (2) | Split Right, Split Down |
| Window (12) | New Window, Close Window, Minimize Window, Toggle Maximize, Reload Window, Reload Window (Free Memory), Hibernate Hidden Browser Panes, Wake Sleeping Browser Panes, Restart Backend, Open Settings File, Open Settings File in System Editor, Open Widgets File |
| Tab (4) | New Tab, Close Tab, Next Tab, Previous Tab |
| Pane (9) | Close Pane, Toggle Magnify, Back to Previous Pane, Forward to Next Pane, Focus Pane Up, Down, Left, Right, Redock floating pane |
| View (10) | Command Palette, Animations: Off, Quick, Smooth, Slow, Toggle Status Bar, Toggle Dev Mode, Zoom In, Zoom Out, Actual Size |
| App (5) | Connectors, Memory, Settings, Customize keyboard shortcuts, Pause all agent browsing (Resume agent browsing replaces it while paused) |
| Workspace (6) | New, Open, Rename, Delete, Export, Import Workspace… |
| Help (1) | Online Docs |
| Dev (1) | Toggle DevTools |
| Agent | one row per agent: its name, with seat, area and state as secondary text |

**Dev mode on vs off.** Only Toggle DevTools (`dev:devtools`, `when: devModeOn`) disappears. The other four commands registered in `registerDevCommands` have no `when`, so they show with dev mode off: Restart Backend, Open Settings File, Open Settings File in System Editor, Open Widgets File. They are in the Window category, not Dev, so the name is misleading. Question 3 below asks which way to fix this.

**With an agent pane focused** (9 more rows): Agent view…, Agent view: Full, Standard, Replies, Follow default, Find in conversation, Agent: Open messages panel (or Show / Hide, by state). Two rows read doubled ("Agent: Agent view: Full", "Agent: Agent: Hide messages panel") because the category and the label both say "Agent".

**Zoom by part** (`view:zoom:<view>:<part>:*`, "Reset zoom in …") appear only while a pane with parts that zoom apart (the Editor's tree and document) is focused. The Editor was not open in this run, so these were read from `zoom-part-commands.ts`, not seen.

**Widget commands** (`contributes.commands`) register as `ext:<id>/<command>` in the Widgets category when a widget is installed. None was installed in this run.

Where entries come from: `registerDefaultCommands()` in `frontend/app/store/command-registry.ts` (the 70 above), `agent-view-commands.ts` (focused agent pane), `zoom-part-commands.ts`, `widget-contributions.tsx`, and `palette-agents.ts` / `palette-agent-entries.ts` (agents). `editor-commands.ts` and `files-view-commands.ts` do not register palette entries: they are pane key handlers (`registerPaneCommandRunner`) and never reach the palette. The macOS menu bar runs ids from the same registry; every menu id (`tab:*`, `window:*`, `view:zoom:*`, `app:*`, `help:docs`, `dev:devtools`, `view:command-palette`) is registered, so menu and palette agree.

## 2. Every feature reachable by name, and whether the palette has it

### 2.1 Pane views (24 registered in `block-registry.ts`)

| View | Palette | Note |
|---|---|---|
| term, agent, sysinfo, tower, kanban, vault, history, audit, warden (Jobs), help, reviews, atlas, swarm, files | Open entry | |
| connectors, memory, settings, toolchain | App / Open entry | Remotes opens Connectors on its Remotes section |
| **editor** | **missing** | on the widget bar; known gap ("Open Editor") |
| **browser** | **missing** | on the widget bar |
| **drone** (Workflows) | **missing** | on the widget bar |
| identity | none | the agent settings tab, opened from an agent pane; exempt candidate |
| shortcuts | none | the empty-pane launcher, created by a palette split; exempt candidate |
| cpuplot | none | legacy name of sysinfo |

The default widget bar (`crates/srv-core/src/config/widgets.json`) also holds Messengers, Discord, Slack, Telegram, WhatsApp and Teams. None has a palette entry. The last five are Browser panes at a fixed address; Messengers is a launcher.

"New …" (always a fresh pane, even for a singleton) exists for Tower, Kanban, Help and Swarm only. Shift+Enter on an "Open" entry gives the same, so this is a naming gap, not a capability gap.

### 2.2 Keyboard-bound commands with no palette entry

From `keybindings/defaults.ts`, ids with a key and no registry entry (a command that is only meaningful inside one pane is marked P):

- Tabs and windows: `tab:rename`, `tab:moveLeft`, `tab:moveRight`, `tab:goto:last` (and 1 to 8), `browser:newIncognito` (P)
- Panes: `pane:new`, `pane:focus:next`, `pane:focus:prev`, `pane:swap:up/down/left/right`, `pane:resize:up/down/left/right`, `pane:refocus`, `pane:replaceWithLauncher`, `pane:changeConnection`, `pane:find`, `pane:voice`, `agent:focusComposer` (P)
- General: `help:shortcuts` (Keyboard shortcuts), `view:zoom:resetAll`, `term:multiInput`, `term:clear` / `copy` / `paste` (P)
- Dev (key-only today): `dev:perfHud`, `dev:diagnostics`
- Pane-scoped families: `doctab:*` (7), `editor:*` (6), `files:*` (17). These are acted on by the focused pane's own key handler. A palette entry would need the focused pane's runner (`registerPaneCommandRunner`) to be the target.

### 2.3 Menus

- Agent pane right-click menu: it has Agent History, Quick Fork, Seat…, Container limits…, Personal memory, the two "without asking" toggles and Shut down. **"View…" and "Bundles…" were removed on purpose** in #7647 ("the agent pane's View options open from an eye in the composer row … the right-click menu drops View and Bundles"; test `agent-body-menu.test.ts` asserts it). So agent5's note (#47946) is a deliberate move, not a loss: View is reachable by the eye and by the palette ("Agent view…", while an agent pane is focused). Bundles lives in the Memory pane's Bundles tab; the palette's Memory entry carries the keyword "bundles" but there is no entry that opens the Bundles tab directly.
- Pane menu items found only in the pane menu (no palette entry): Backdrop… (pane backdrops), pane colour and rename entries.
- The macOS menu bar items have palette equivalents (see section 1).

### 2.4 Features since 0.60.20 with a named entry point and no palette entry

From `VERSION_HISTORY.md`, 0.60.20 to 0.60.33 (not every line is a user entry point; these are):

- Settings > Keyboard shortcuts (0.60.20, 0.60.21): has "Customize keyboard shortcuts". Covered.
- Agent pane search, Messages panel (0.60.20, 0.60.21): covered, but only while the agent pane is focused.
- Backdrop… in the pane menu (0.60.31): no entry.
- Large text viewer: go to line (Ctrl+G), hex viewer find (0.60.30 to 0.60.32): pane-scoped keys; no entries.
- Swarm broadcast (0.60.31): the Swarm's own panel; no entry (Open Swarm reaches it).
- Vault Share button, Vault batch import, export (0.60.31, 0.60.33): inside the Vault pane; no entry.
- Reviews pane, Audit action rows (0.60.31): Open entries exist.
- Browser pane profile menu and saved sign-ins list (0.60.30, 0.60.33): no entries; Browser itself has none.
- Streamed browser (headless web UI, 0.60.31): no desktop entry point.
- Account menu of the agent pane (API key account, 0.60.31): pane menu only.
- Drone replay (0.60.30): no entry (Drone has none).
- Status bar parts and chips: not commands; Toggle Status Bar exists.

## 3. Gaps, ranked

1. **Open Editor, Open Browser, Open Drone** (and New … where it matters). The three are on the widget bar and are the visible gap; a person who lives in the palette cannot open an Editor or a Browser.
2. **Keyboard shortcuts (`help:shortcuts`)**: it has a key (F1, which `keymodel.ts` turns into opening the Help pane) but no palette entry, in a palette that lists "Customize keyboard shortcuts".
3. **Context-free pane and tab commands with keys**: New Pane (`pane:new`), Rename Tab, Move Tab Left/Right, Next/Previous Pane, Swap with …, Move pane border …, Change connection, Replace pane with Shortcuts, Reset zoom on all panes, Type into all terminals, Voice input. A person who cannot remember the chord has no other way in.
4. **Messengers and the five messenger widgets** (a user who pinned them in the widget bar has no name for them in the palette).
5. **Bundles** (Memory > Bundles) and **Backdrop…** (pane menu): reachable by one named place only.
6. **Dev-labelled commands that show with dev mode off** (section 1): either gate them or relabel them, whichever the operator wants.
7. **Doubled "Agent:" in two agent rows.**

## 4. Proposed entries

Following the existing naming (`Open <View>` in category Open; the id `open:<view>`; Shift+Enter or "New …" for a fresh pane):

| Id | Label | Opens |
|---|---|---|
| `open:editor` | Open Editor | `openPane({meta: {view: "editor"}})` |
| `open:browser` | Open Browser | `openPane({meta: {view: "browser"}})` |
| `open:drone` | Open Drone (Workflows) | `openPane({meta: {view: "drone"}})` |
| `open:messengers` (and one per messenger, if wanted) | Open Messengers / Discord / Slack / Telegram / WhatsApp / Teams | the widget's own blockdef |
| `help:shortcuts` | Keyboard shortcuts | the same action as F1 |
| `pane:new`, `tab:rename`, `tab:moveLeft/Right`, `pane:focus:next/prev`, `pane:swap:*`, `pane:resize:*`, `pane:replaceWithLauncher`, `pane:changeConnection`, `view:zoom:resetAll`, `term:multiInput` | named as in the shortcuts sheet | each already runs from the key handler; the palette entry calls the same function |
| `open:bundles` | Open Bundles | Memory, Bundles tab |
| `pane:backdrop` | Backdrop… | opens the backdrop picker on the focused pane |

Each would use the existing `requires` / `when` fields (for example Rename Tab only when a tab is active; Voice input only where it works).

## 5. Guard (proven)

A test that asserts every registered pane view has a palette command or a stated exemption, so a new view cannot ship without one:

- Draft: `frontend/app/store/command-registry-coverage.test.ts` (untracked in the author's clone). It imports `block-registry` (registers all views), calls `registerDefaultCommands()`, lists `listPaneTabs()` without legacy names, and checks each view against a table `view -> command id | { exempt: reason }`, and that the named id exists.
- **Today it fails for exactly the three known gaps** (browser, editor, drone): `"editor" names open:editor, which is not registered`.
- **Proof by removal:** with `open:vault` deleted from `registerOpenViewCommands`, the same run fails four tests (the three gaps plus `"vault" names open:vault, which is not registered`) and passes again once the entry is restored.
- A second guard, a table of keyboard-bound ids with no entry that must be listed as accepted ("pane-scoped") or have an entry, would cover section 2.2 the same way.

## 6. Questions for the operator

1. Should every widget-bar item have an "Open …" entry (including the five messenger tiles), or only the built-in views?
2. Should the pane-scoped and key-only commands (section 2.2) all go in the palette, or only the context-free ones (tabs, panes, zoom, shortcuts)?
3. The four commands in the Window category that act like developer tools (Restart Backend, Open Settings File, Open Settings File in System Editor, Open Widgets File): hide them with dev mode off, or leave them shown?
4. Backdrop… and Bundles: palette entries, or leave them to the pane menu and the Memory pane?
5. For a view that is a singleton, should "New …" exist for all of them, or is Shift+Enter enough?

## 7. Not covered here

- Scoring of search quality (matching, ordering).
- Commands an installed widget contributes (none installed in the run).
- The Editor-only zoom-part commands (read from code, not seen live).
- The touch of `command-palette.tsx` and its styles: Agent1 owns them (#47899 / #47900). Nothing in this report needs a change there; the proposed entries are all in `command-registry.ts` and the feature files.
