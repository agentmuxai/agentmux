# SPEC: a "dev mode" setting that hides developer-only UI

**Author:** Agent2
**Date:** 2026-10-28
**Status:** implemented, 2026-10-28 (task #6931; built in #6932)

The operator's request (2026-10-28, quoted): "lets add a "dev mode" setting where the "Dev Tools" in the hamburger and "Inspect Element" on right click menus get removed, they only show when dev mode is enabled, and it is disabled by default. also find any other candidates that should go out of normal and into dev mode".

All file references are to current `origin/main` (a06f7bfd0).

---

## 1. Problem

Every window has a hamburger entry "DevTools", and every pane's right-click menu ends with "Inspect Element". They open Chromium's DevTools on the running app. An operator who has never opened DevTools sees two developer-facing entries in the menus they use all day, and a stray click opens a debugger window on the app's own UI.

## 2. Goal

- One boolean setting, **off by default**, that gates DevTools and Inspect Element everywhere they are reachable: the hamburger menu, every right-click menu, the command palette, the macOS menu bar and the host commands behind them.
- Turning it on or off takes effect at once, in every window, with no restart.
- Task dev and debug builds keep DevTools without anyone touching the setting.
- An audit (§6) of the other developer-facing items in the app, each with a recommendation.

Non-goals: no change to what agents can do through MCP / MuxAPI (§7); no new DevTools features.

## 3. The setting

| | |
|---|---|
| Key | `app:devmode` (boolean) |
| Declared in | `schema/settings.json` (no `default`: absence means "the build's default", below), the typed settings struct in `crates/srv/src/backend/wconfig/types.rs`, and the generated `frontend/types/srv-types.d.ts` (regenerate, don't hand-edit); pinned in `settings-defaults.test.ts` like its neighbours |
| Effective value | `settings["app:devmode"]` if set, otherwise **true in a task dev / debug build, false in a release build** |
| Where it appears | Settings › Advanced, a new "Developer" header at the bottom with one toggle "Dev mode"; the settings search finds it by "devtools", "inspect element", "developer", "app:devmode" |
| Palette | "Toggle Dev Mode" (id `view:devmode:toggle`, category View), modelled on `view:statusbar:toggle` in `command-registry.ts`. Always listed, so a user can turn it on when support asks |

"Task dev / debug build" is decided by the same test the app already uses for the DEV badge: `isDev()` in the frontend (`getApi().getIsDev()`), and `agentmux_common::is_dev_self()` in the host. This means agents testing a task dev build keep DevTools with no setup. (Operator decision D1: yes, recommended.)

### 3.1 Frontend

New `frontend/app/store/dev-mode.ts` exports `devModeOn()` (reactive: reads `settingsAtom()` and falls back to `isDev()`) and `setDevMode(on)`, which writes `app:devmode` through the same `set()` the settings controls use. Every gate below calls `devModeOn()`; nothing reads the key directly.

Settings reach every window already: `settingsAtom` is `fullConfigAtom().settings`, fed by srv's config event, so a toggle in one window changes all of them, including torn-off windows and browser-pane clients. No new channel is needed.

### 3.2 Host (CEF)

The native paths are `toggle_devtools`, `inspect_element_at` and `browser_pane_inspect_element` (`crates/cef/src/ipc.rs`, `commands/window/meta.rs`, `browser_panes/navigation.rs`). Each refuses with an error ("DevTools is off: turn on Dev mode in Settings › Advanced") unless dev mode is on. This is the backstop: a stale window, a missed menu or a script cannot open DevTools while the setting says off.

The host reads the effective value on demand from the live `settings.json` that srv loads and the settings UI edits (`crates/cef/src/dev_mode.rs`), with the default `is_dev_self()`. The rule itself is `agentmux_common::settings_jsonc::dev_mode_from`, in the CEF-free crate so its tests run in the PR lane. Reading a small file on each of these rare commands is cheap and is always current, so the host needs no push channel and no cached copy to keep in step.

### 3.3 macOS menu bar

`macos_menu.rs` builds a static menu once at launch, with View › "Toggle DevTools" (`dev:devtools`) and Settings…. There is no update path for an existing item. Recommendation: build the item only if dev mode is on at launch; the command itself is gated live (§3.2), so a toggle made after launch takes effect for the item at the next start and for everything else at once. Not verifiable on narko (Windows): needs Clare to check a build. (Decision D4.)

### 3.4 Already-open DevTools windows

Turning dev mode off does not close a DevTools window that is open. It only stops new ones. (Simplest; closing it would need the host to track the window.)

## 4. The two named items

### 4.1 "DevTools" in the hamburger menu

`frontend/app/window/hamburger-menu.tsx:213`. The items are built in a `createMemo` that deliberately reads no reactive state (a rebuild would remount an open Theme or Opacity submenu under the cursor). The gate is a spread, `...(devModeOn() ? [item] : [])`, which makes the memo depend on dev mode. A toggle happens in Settings or the palette, never with the hamburger open, so the one rebuild is harmless; the test must pin that Theme/Opacity do not rebuild on any other change.

### 4.2 "Inspect Element" in every right-click menu

There are two builders, and both need the gate:

1. **`buildPaneContextMenu`** (`frontend/app/block/pane-actions.ts:152`). Its `inspect` section is the last group of the menu of every pane that goes through `blockframe.tsx` (call sites at `blockframe.tsx:220` and `:1289`, each passing `inspectAt`): agent, terminal, editor, files, Swarm, Warden, settings, widgets. One change here covers them all: drop the section when `devModeOn()` is false (a `PaneMenuSection` already exists, `"inspect"`, and `omit` already removes it, so the gate is `omit` plus `inspectAt` unset). The shell drawer in an agent pane (`AgentShellSubblock.tsx`) keeps magnify/close/inspect by test; that test changes to expect `inspect` only with dev mode on.
2. **The browser pane page menu**, `getBodyContextMenuItems` in `frontend/app/view/browser/browser-model.ts:921`. It has its own "Inspect Element" (`browserPanes.inspectElement`, the host command `browser_pane_inspect_element`). Gate it the same way. Its neighbour "View Page Source" is the same kind of item; see D3.

The menus the app builds itself in the main client are DOM-level (`showTextInputContextMenu` on the root, `app.tsx:448`, which calls `preventDefault`), so Chromium's own menu, which has an "Inspect" item, is not shown there. The browser panes suppress CEF's native menu in the host (`client/context_menu.rs`) and show the frontend's. The one path that falls back to CEF's native menu is `run_context_menu` returning 0 when a pane's block or window can't be resolved; that menu has a native "Inspect". The build closes that path: while dev mode is off, `run_context_menu` cancels the callback and returns 1 in that case instead of returning 0, so the native menu never shows (the user gets no menu for that one right-click, which is better than a menu with Inspect). The task dev run (§8) confirms by forcing the fallback.

Tab bar, doc tabs, Swarm and Warden menus have no Inspect item today (grep of `Inspect`/`inspectElement` finds only the two builders above).

### 4.3 Keyboard shortcuts

Grep finds no F12 / Ctrl+Shift+I / Ctrl+Shift+J handler in the host (`client/handlers.rs` `handle_pre_key_event`) or in the shortcut table (`keybindings/defaults.ts`); the only DevTools rows are the Ctrl+Alt+Shift+P Performance HUD and Ctrl+Alt+Shift+F12 Diagnostics panel, both `devOnly` (§6). Whether CEF's own accelerators open DevTools in this window style is not known from reading, so the build starts by checking it in the task dev instance (send F12 and Ctrl+Shift+I by CDP). If they do open it, `handle_pre_key_event` consumes F12, Ctrl+Shift+I, Ctrl+Shift+J and Ctrl+Shift+C (Cmd+Option+I/J/C on macOS) while dev mode is off, using the same read as §3.2. If they don't, nothing to do and the spec's answer is "no shortcut exists".

### 4.4 Command palette

"Toggle DevTools" (`dev:devtools`, `command-registry.ts:447`, category Dev) is hidden while dev mode is off, and `run("dev:devtools")` declines it (so a keybinding or the macOS menu can't run it either). `CommandEntry` already has `requires?: keyof HostCaps`, which `isCommandAvailable` checks both for the palette and for `run`; add a sibling `when?: () => boolean` that `isCommandAvailable` also checks, and give `dev:devtools` `when: devModeOn`. `window:recycle`, `browser:hibernate-hidden` and the rest are not gated.

## 5. How a user is told to turn it on

Support or an agent says: "Settings › Advanced › Developer › Dev mode, or the command palette: Toggle Dev Mode." The palette entry is always present and always searchable by "dev". The settings row's description says what it turns on and where it differs: "Shows DevTools in the menu, Inspect Element in right-click menus, and other developer tools. Off by default in the installed app; on by default in a task dev or debug build." The search keywords include "task dev" and "debug build", so nobody is confused when their own build shows DevTools and the installed one doesn't.

## 6. Audit: other developer-facing UI

Method: every menu builder (`label:` items in hamburger, pane/tab/Swarm/Warden/editor/browser menus), the command registry and keybinding table, Settings sections and their search index, the status bar and its popovers, the host's IPC command list and the macOS menu. "Who" is who uses it today: O operator day to day, D developer, A agents (through MuxAPI / MCP, not the UI).

| # | Item | Where | Who | Recommendation | Why |
|---|---|---|---|---|---|
| 1 | DevTools | hamburger menu, `hamburger-menu.tsx:213` | D | **Dev mode** | Named by the operator |
| 2 | Inspect Element | every pane's right-click menu, `pane-actions.ts:215` | D | **Dev mode** | Named |
| 3 | Inspect Element | browser pane page menu, `browser-model.ts:921` | D | **Dev mode** | Named ("every right-click menu") |
| 4 | Toggle DevTools | command palette `dev:devtools` | D | **Dev mode** | Same action as 1 |
| 5 | Toggle DevTools | macOS menu bar View, `macos_menu.rs:354` | D | **Dev mode** (at launch, §3.3) | Same action |
| 6 | View Page Source | browser pane page menu, `browser-model.ts:915` | D | **Dev mode** (D3) | Opens a source view of the page; nothing an operator needs next to Back/Reload/Print |
| 7 | Restart Backend | palette `dev:restart_backend` | O, D | **Keep** (rename its category from "Dev" to "Window") | A recovery action for a stuck backend; the operator needs it exactly when something is wrong, and dev mode may be off |
| 8 | Open Settings File | palette `dev:open_settings` | O, D | **Keep** (category "Window") | The documented way to hand-edit settings; the macOS menu already lists "Settings…" outside any dev gate |
| 9 | Show block IDs | Settings › Window, `window-panes-section.tsx:14` (`blockheader:showblockids`) | D | **Dev mode**: hide the row while off, and stop drawing IDs in headers while off even if the value is true | A "debugging aid" by its own description; a stored `true` would otherwise keep IDs on with no way to see why |
| 10 | DevTools port | status bar host popover, `HostPopover.tsx:355` | D | **Dev mode** (the one row) | Meaningless without DevTools; the IPC / Backend / WS port rows stay: support asks for them |
| 11 | Performance HUD (Ctrl+Alt+Shift+P) | `perf/hud.tsx`, row `dev:perfHud` | D | **Keep as is** (build-gated `isDev()`) | Its probes are compiled out of release builds (`import.meta.env.DEV`); under dev mode in a release build it would open an empty HUD |
| 12 | Diagnostics panel (Ctrl+Alt+Shift+F12) | `devtools/diag-panel.tsx`, row `dev:diagnostics` | D | **Keep as is** (build-gated) | Same |
| 13 | "DEV" badge | status bar, instance panel | D | **Keep** | Identifies the build, not a tool |
| 14 | Web Speech engine | Settings › Recording, `recording-section.tsx:323` | D | **Keep as is** (build-gated) | Already hidden in release |
| 15 | GPU trace commands | host IPC, `commands/window/gpu_trace.rs` | D | **Keep as is** (`AGENTMUX_DEV` env gate) | No UI; already refused unless the env var is set |
| 16 | Copy block / agent / shell / cron / workflow / node / run / account ID | Swarm, drone and identity right-click menus | O, A | **Keep** | Operators paste these into messages and task cards; agents ask for them by ID. Moving them behind dev mode would hurt the people they serve |
| 17 | Reload (page), Reload Window (Free Memory), Hibernate Hidden Browser Panes | browser menu, palette | O | **Keep** | Memory and browsing tools the operator uses |
| 18 | CDP port and the browser DOM API | `crates/cef/src/cdp_port.rs`, `browser_api/` | A | **Keep; never gated** | The Browser MCP tool and the test harness drive panes over it; it is not UI |
| 19 | `getPaneDebugState`, `__agentmuxOneWay`, `atom-cache-diagnostic` | window-global debug handles | D | **Keep as is** | No menu or setting; reached only from a console |
| 20 | Tray menu | none | | n/a | No tray menu exists in the host (grep of `crates/cef`) |

Not found: a raw JSON/block viewer, a log-viewer menu item, a feature-flag screen or a seed/test command in any menu or palette. If the operator knows of one, add it to this table.

## 7. What dev mode must NOT gate

- Anything an agent does through MuxAPI or MCP: the Browser tool's snapshot / click / evaluate, `UI` screenshots and queries, `Pane`, the CDP port, test-harness key injection. None of them calls `toggle_devtools`, `inspect_element_at` or `browser_pane_inspect_element` (grep of `crates/srv`, `crates/mcp`, `crates/common`: no hits). Agents never read the setting.
- The status of the build (DEV badge, version) and recovery tools (rows 7, 8).
- Agent panes' own debugging: transcripts, tool-call details and the agent-pane "copy" menus are part of the product, not developer chrome, and stay.

## 8. Tests and checks

- `dev-mode.test.ts`: effective value (set true / set false / absent in a dev build / absent in a release build).
- `pane-actions.test.ts`: the menu has no Inspect Element with dev mode off and has it with dev mode on; the other groups and separators are unchanged either way (the `joinMenuGroups` shape cases).
- `hamburger-menu.test.tsx`: DevTools is absent / present; Theme and Opacity do not remount on a dev-mode toggle beyond the one rebuild.
- `browser-model.test.ts`: Inspect Element and View Page Source follow the setting.
- Command palette test: "Toggle DevTools" is absent with dev mode off, "Toggle Dev Mode" always present.
- Settings: `settings-defaults.test.ts` pins `app:devmode` (no default); a search for "devtools" finds the row; the Show block IDs row is absent while off.
- Rust: the three host commands return the "turn on Dev mode" error with the setting false or absent in a non-dev build, and succeed when true or when the build is dev; the helper reads a JSONC `settings.json` (comments, trailing commas) like `bool_setting_in`'s tests.
- Isolated `task dev` check, with screenshots attached to #6932: hamburger menu, a terminal pane menu, an agent pane menu and a browser pane menu, each with dev mode off and on; the toggle from the palette with two windows open (both change); F12 / Ctrl+Shift+I by CDP (§4.3); the native-menu fallback (§4.2). Because a task dev build defaults dev mode ON (D1), the "off" shots set `app:devmode: false` explicitly.

Changelog line: "Added a Dev mode setting (off by default): DevTools and Inspect Element now show only when it is on."

## 9. Operator decisions, with a recommendation each

- **D1.** Dev mode defaults on in task dev and debug builds, off in release builds. *Recommend yes*: agents testing builds keep DevTools; the operator's installed build is off.
- **D2.** Setting name `app:devmode`, in Settings › Advanced under a "Developer" header. *Recommend yes*: Advanced already holds the power-user rows (WebGL fallback, sub-agent models, environment variables), and `app:` is the prefix of the other app-wide keys.
- **D3.** Also gate "View Page Source" in the browser pane menu. *Recommend yes*: same class as Inspect; Back, Reload, Print, copy and split stay.
- **D4.** macOS menu item decided at launch, not live. *Recommend yes*: the command is gated live regardless; a live native menu update is a separate piece of work.
- **D5.** Show block IDs (row 9) moves behind dev mode. *Recommend yes*, and a stored `true` stops drawing IDs while dev mode is off. The "Copy block ID" / "Copy agent ID" menu items take their IDs from the model (Swarm's `node().blockId`, `sub.agent_id`), not from the header text, so they work with the header IDs hidden; a test pins that.
- **D6.** Restart Backend and Open Settings File stay in the normal palette, moved out of the "Dev" category. *Recommend yes*: recovery and settings editing are not developer-only.
- **D7.** Perf HUD and Diagnostics panel stay build-gated rather than following dev mode. *Recommend yes* (row 11).

## 10. Build plan (#6932)

One PR, one changeset. Order: schema + types + `dev-mode.ts` + the settings row and palette command; the frontend gates (pane-actions, hamburger, browser-model, palette `when`, show-block-ids, HostPopover); the host gate and the macOS item; the tests; then the isolated dev check. The shortcut work (§4.3) is added only if the check shows DevTools opens from a key. Files touched stay under the file-size gate: `command-registry.ts` and `advanced-section.tsx` are the large ones, so the new code goes in `dev-mode.ts` and a small section component.
