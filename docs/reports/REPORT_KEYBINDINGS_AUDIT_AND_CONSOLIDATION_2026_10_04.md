# REPORT — Keyboard shortcuts: what works, what the help pane and docs promise, and one framework for all of it

**Status:** analysis — owner decisions in §10 (2026-10-04), best practices in §11. Implementation progress in §13.
**Trigger:** owner request, 2026-10-04: make sure every shortcut in the help pane works as described; propose new shortcuts; align help pane, code and docs; consolidate every keybinding system into one framework and remove legacy ones.
**Verified against:** agentmux `main` @ `7a2d019ae`, agentmux-docs `main` @ `2d6758d`, 2026-10-04.
**How verified:** by reading code. The double-firing mechanism (§3.1) and the terminal routing (§3.2) were confirmed in the source by hand; the rest comes from a full code trace. **Nothing here was run in the app**, so every "probable bug" in §3 needs a live check before it is fixed.

---

## 1. Summary

- **The help pane's 13 shortcuts mostly work as described.** Two are conditional (`Cmd+G` only in terminal/sysinfo panes; `Cmd+N` follows `app:defaultnewblock`). One UI hint is wrong: the terminal's mic tooltip promises Ctrl+Shift+V for voice, but in a terminal that key pastes.
- **The help pane leaves out the most important shortcut,** the command palette (`Ctrl+P`), and about 25 more that work (§2.2).
- **The real problem is conflicts, not missing handlers.** Global shortcuts fire even while the user types, and a pane cannot stop them:
  - In the **terminal**, every key goes through the global map first. On Windows/Linux, `Cmd` means **Alt**, so readline's Meta keys are taken: **Alt+W closes the pane**, Alt+D splits, Alt+T opens a tab, Alt+F opens find. `Ctrl+P` (shell history) opens the palette, and `Ctrl+[` (Escape in vim) moves focus to another pane.
  - In the **editor**, `Ctrl+Shift+K` deletes the line **and replaces the editor pane with the launcher**. `Ctrl+[`/`]` indent **and** switch panes.
  - In the **Files pane**, `Ctrl+Shift+N` creates a folder **and** opens a new window.
- **There are 9 separate keybinding mechanisms** (§4), with four different meanings of "handled" and no shared registry. The only setting that looks like a binding setting (`app:globalhotkey`) is never read.
- **The docs site disagrees with itself.** `keybindings.md` matches the code, but other pages list shortcuts that don't exist (`Ctrl+,`, `Cmd+Shift+A`, `Ctrl+H`), give wrong platform keys (`main-menu.md`), and describe the dead `app:globalhotkey` as working.

**Recommendation:** one command-and-binding registry with context ("when") rules and a single dispatcher (§6), built in phases (§8). The first phase fixes the destructive conflicts without waiting for the new framework.

---

## 2. Help pane vs. code

The help pane is `frontend/app/view/helpview/helpview.tsx`, which renders `QuickTips` (`frontend/app/element/quicktips.tsx`). Its key labels are hand-written. "All Keybindings" links to docs.agentmux.ai/keybindings.

`Cmd` in a binding means ⌘ on macOS and **Alt** on Windows/Linux (`frontend/util/keyutil.ts:50-56`). The help pane draws it correctly as "Alt" there.

### 2.1 Every shortcut the help pane lists

| Help pane (quicktips.tsx) | Win/Linux | macOS | Handler | Verdict |
|---|---|---|---|---|
| Maximize a Pane `Cmd:m` (:113) | Alt+M | ⌘M | keymodel.ts:90 | Works |
| Connect to a remote server `Cmd:g` (:122) | Alt+G | ⌘G | keymodel.ts:138 | **Conditional:** only panes that manage a connection (terminal, sysinfo); does nothing elsewhere |
| Close Pane `Cmd:w` (:137) | Alt+W | ⌘W | keymodel.ts:82 | Works (closes the tab if it has no panes) |
| New Tab `Cmd:t` (:156) | Alt+T | ⌘T | keymodel.ts:78 | Works |
| New Terminal Pane `Cmd:n` (:160) | Alt+N | ⌘N | keymodel.ts:56 | **Conditional:** follows `app:defaultnewblock`, so it can open a launcher instead |
| Switch to Nth Tab `Cmd:Digit` (:170) | Alt+1…9 | ⌘1…9 | keymodel.ts:181 | Works |
| Previous / Next Tab `Cmd:[` / `Cmd:]` (:174, :178) | Alt+[ / ] | ⌘[ / ] | keymodel.ts:40-55 | Works (wraps around) |
| Navigate Between Panes `Ctrl:Shift:Arrows` (:188) | Ctrl+Shift+arrows | ⌃⇧arrows | keymodel.ts:98 | Works, but takes word/line selection away from every text field (§3) |
| Focus Nth Pane `Ctrl:Shift:Digit` (:192) | Ctrl+Shift+1…9 | ⌃⇧1…9 | keymodel.ts:186 | Works. The numbered overlay that should appear never does (§5, item 1) |
| Split Right `Cmd:d` (:202) | Alt+D | ⌘D | keymodel.ts:66 | Works (an agent pane splits into an agent picker) |
| Split Below `Shift:Cmd:d` (:206) | Alt+Shift+D | ⇧⌘D | keymodel.ts:70 | Works |
| Split in Direction `Ctrl:Shift:s` + arrow (:210) | same | same | chord, keymodel.ts:264; 2 s timeout | Works. Any other second key is swallowed |
| Resize Single Border `Shift + Drag` (:214) | — | — | TileLayout.*.tsx | Works |

**Other key hints shown in the UI:**

| Hint | Verdict |
|---|---|
| Terminal mic tooltip "Speak into this terminal (Ctrl+Shift+V)" (term.tsx:498) | **Wrong.** In a terminal, Ctrl+Shift+V pastes (termViewModel.ts:413-421) before the voice binding runs. |
| Agent "Speak into this agent (Ctrl+Shift+V)" (AgentFooter.tsx:1350) | Works. In the agent's shell drawer it pastes instead. |
| Editor empty state "Mod+S" (editor-view.tsx:98-103) | **Misleading:** Save is a CodeMirror key, so it does nothing on the empty screen. |
| "Rendered preview (Mod+Shift+V)" (editor-view.tsx:918) | Markdown files only. |
| Hamburger menu labels (New Tab, New Window, Command Palette) | Correct by construction: generated from the binding constants (`formatKeyDescription`). |
| Document tabs "New tab (Ctrl+T)" | Works. Literal Ctrl on every platform. |
| Files context-menu labels (files-view.tsx:522-660) | Handlers exist, but some keys also fire a global shortcut (§3.1). |

### 2.2 Works but isn't in the help pane

- **Window and tabs:**
  - `Ctrl+P` command palette.
  - `Ctrl+Shift+N` new window.
  - `Cmd+Shift+W` close tab, with confirmation.
  - `Shift+Cmd+[`/`]`, which duplicate tab switching.
- **Panes:**
  - `Cmd+I` refocus the pane.
  - `Ctrl+[`/`]` cycle pane focus.
  - `Ctrl+Shift+K` replace the pane with the launcher. It's destructive and asks for no confirmation.
  - `Ctrl+Shift+M` terminal multi-input.
  - `Ctrl+Shift+V` voice.
  - `Cmd+F` find. That is **Alt+F** on Windows.
  - `Esc` closes the top modal, then find, then un-maximizes.
- **Zoom:**
  - `Cmd`/`Ctrl` + `=`/`-`/`0` zoom the focused pane.
  - Ctrl+wheel zooms the pane under the cursor.
  - Ctrl+Shift+wheel zooms all panes.
- **Terminal:** `Ctrl+Shift+C`/`V` copy and paste, `Cmd+K` clear, `Enter` restarts a finished shell, `Shift+Enter` newline (a setting).
- **Browser pane:** `Ctrl+L` address bar, `Ctrl+R` reload, `Alt+←/→` back and forward. These are handled in the host app (Rust), not the frontend.
- **Document tabs (editor and media panes):** `Ctrl+T`, `Ctrl+W`, `Ctrl+Tab`/`Ctrl+Shift+Tab`, `Ctrl+PgUp/PgDn`, `Ctrl+Shift+PgUp/PgDn` (move a tab), `Ctrl+Shift+T` (reopen).
- **Editor:** `Ctrl+S`, `Ctrl+Shift+S` (Save As, scratch tabs only).
- **Files pane:** about 30 keys: Alt+arrows, Backspace, Ctrl+A, Ctrl+L, Ctrl+F or `/`, Space, Ctrl+Space, type-ahead, F2, Delete…
- **Agent composer:** Enter sends, Shift+Enter adds a newline, Esc clears (or interrupts when empty), Ctrl/⌘+Z undoes the clear, ↑/↓ history, Tab/→ accept a suggestion, Ctrl+F search.
- **Dev builds only:** `Ctrl+Shift+D` diagnostics panel, `Ctrl+Shift+P` perf HUD. `F5`/`Ctrl+R` reload, during startup only.

---

## 3. Conflicts: one key, two actions

### 3.1 Why a pane can't stop a global shortcut

The global handler is a plain `document` keydown listener (`frontend/app/app.tsx:193-194`). Solid's `onKeyDown` handlers are *also* delegated to `document` (solid-js `delegateEvents`). A pane's `stopPropagation()` stops only Solid's own walk, not the second `document` listener, and `keydownWrapper` never checks `defaultPrevented`. Separately, the global map is checked **before** any "is the user typing?" test (`keymodel-dispatch.ts:128`); only dispatch to the focused pane is gated.

### 3.2 The terminal hands every key to the global map first

`termViewModel.ts:439` calls `appHandleKeyDown` from xterm's key hook, so the shell only sees a key that no global binding claims. The guard meant to protect shell keys (`keymodel.ts:199`, `event.control && shellKeys`) can never match `Cmd:f`, which is Alt on Windows, so it protects nothing.

### 3.3 Conflict list (from the code; to confirm live)

| Where | Key | Intended | Also happens | Severity |
|---|---|---|---|---|
| Terminal (Win/Linux) | Alt+W | readline: kill region | **closes the pane** | High, loses work |
| Terminal (Win/Linux) | Alt+D / Alt+T / Alt+F / Alt+K / Alt+1…9 | readline Meta keys | split / new tab / find / clear / switch tab | High |
| Terminal | Ctrl+P | shell history | command palette | High |
| Terminal | Ctrl+[ | Escape (vim) | cycles pane focus | High |
| Terminal, editor, agent | Ctrl+Shift+K | (editor: delete line) | **pane replaced by the launcher** | High, destructive |
| Editor (Win/Linux) | Ctrl+[ / Ctrl+] | indent | switch pane | High |
| Editor, composer, any text field | Ctrl+Shift+arrows | extend selection by word or line | switch pane | Medium |
| Editor (scratch tab) | Ctrl+Shift+S | Save As | arms the split chord; the next key within 2 s is eaten | Medium |
| Editor (mac) | ⌘D | select next match | split pane | Medium |
| Files (Win/Linux) | Ctrl+Shift+N | new folder | new window | Medium |
| Files (mac) | ⌘T / ⌘W / ⌘F | files tab / close files tab / filter | workspace tab / **close the pane** / pane find | High (⌘W) |
| Composer, text fields | Ctrl+Shift+V | paste as plain text | voice toggle, or swallowed on panes without voice | Low |
| Swarm pane | Ctrl+= | swarm zoom | also focused-pane zoom (capture listener without `stopPropagation`) | Low |
| Diagnostics (mac, dev) | ⌘⇧D | diag panel | never fires: `Shift:Cmd:d` (split) wins | Low |
| Browser pane focused | any app shortcut | — | **none fire**: the page gets the key, and the host forwards only Ctrl+L/R and Alt+arrows | Gap |

---

## 4. The keybinding systems today

| # | Mechanism | Where | Owns | State |
|---|---|---|---|---|
| M1 | Global key map (Wave heritage) | `store/keymodel*.ts`, `util/keyutil.ts` | 62 key strings, about 32 actions, one chord | Live, the main one |
| M2 | Per-pane `keyDownHandler` | `pane-tab-host.tsx:74` | the launcher only; the terminal's always returns false | Live, barely used |
| M3 | xterm `attachCustomKeyEventHandler` | `termViewModel.ts:395-446`, `shell-drawer-keys.ts` | copy, paste, clear, restart; the terminal then calls M1 | Live |
| M4 | CodeMirror keymaps + editor capture listener | `editor-view.tsx:141-190, 371-421` | Save, Save As, find, preview, document tabs | Live |
| M5 | Document-tab keys | `doc-tabs-controller.ts:214-255` | 7 literal-Ctrl keys | Live |
| M6 | Ad-hoc `window`/`document` listeners | about 30 sites | perf HUD, diag panel, startup reload, many Escape-to-close, pane zoom | Live |
| M7 | Component `onKeyDown` | 77 uses in 55 files | Files pane (about 30 keys), composer, modals, palette, F2 rename | Live |
| M8 | Host `OnPreKeyEvent` (Rust) | `crates/cef/src/client/handlers.rs:218-451` | browser pane Ctrl+L/R, Alt+arrows; lets Ctrl+P/G through to JS | Live |
| M9 | Native keys | `macos_menu.rs`, tear-off drag hooks | macOS Edit/App menu; Escape during a tab drag | Live, macOS mostly |

There are also four different meanings of "handled": return `true` (M1), return `false` (xterm), `preventDefault` + `stopPropagation` (DOM) and return `1` (host).

The command registry (`command-registry.ts`, about 33 ids) feeds the palette, the macOS menu and `run_command`. Its entries carry no keys, and M1 calls the same actions directly rather than through those ids.

**Duplicated helpers:**
- Platform detection: 4 copies.
- "Primary modifier": about 9 copies. Some treat it as ⌘/Alt, some as ⌘/Ctrl, some as literal Ctrl.
- "Is the user typing?": about 8 copies.
- Shortcut-label renderers: 4. `formatKeyDescription`, quicktips' own, hand-written labels in `files-view.tsx` and in `editor-view.tsx`.

**Two platform conventions coexist.** The old code uses "`Cmd` = Alt on Windows/Linux". Newer code (document tabs, Files pane, memory editor) uses Ctrl. `SPEC_DOCUMENT_TABS_2026_10_02.md` §4.3 frames it as "`Cmd:` = window level, `Ctrl:` = pane level".

---

## 5. Legacy and dead code to remove

1. **Ctrl+Shift "layout mode" and the numbered pane overlay:**
   - Files: `keymodel-dispatch.ts:12,38-56,151-159`, `global.ts:64`, `blockframe.tsx:895,1072`.
   - It waits for a `control-shift-state-update` event that nothing in `crates/` emits, so it is never on. Settings still advertises "Show numbered overlays for quick pane-jump shortcuts".
   - **Remove it, or rebuild it in the frontend:** show the overlay while Ctrl+Shift is held.
2. **Chord-mode IPC:** `setKeyboardChordMode` → `set_keyboard_chord_mode` is a host stub (`crates/cef/src/commands/stubs.rs:40`). The chord already works entirely in JS.
3. **`onReinjectKey` and `onMenuItemAbout`** (`cef-api.ts:617`, `global.ts:220`): no emitter, no caller.
4. **Unused helpers:** `keyutil.ts` `keyMap`, `keymodel.ts` `getAllGlobalKeyBindings`. Also `isInputEvent`, which returns `undefined` on its last path.
5. **`TermViewModel.keyDownHandler`:** always returns false.
6. **Debug leftovers on the key path:**
   - `keymodel-debuglog.ts` writes to a hard-coded `C:/Systems/agentmux-debug.log` over RPC on every `Cmd+W`.
   - The `lastHandledEvent` de-dupe with `console.log` (`keymodel-dispatch.ts:103-109`).
   - `console.log` in `switchTab`.
7. **`app:globalhotkey`:** declared in the schema, the settings template, the Rust config type and the TS types; never read. Remove it, or replace it with the `keybindings` setting (§6.6).
8. **Ctrl+G passed through by the host** (`handlers.rs:239,360`): no JS binding uses Ctrl+G.
9. **The `shellKeys` Ctrl+F guard** (`keymodel.ts:199`): cannot match, so it protects nothing.
10. **`quicktips.tsx`'s own label renderer:** replace it with the generated cheat sheet (§7).

---

## 6. One framework

Extend the existing command registry. It already feeds the palette, the macOS menu and `run_command`. Make it the single table of actions, and add a single table of bindings on top.

### 6.1 Commands
`CommandEntry` gains `when?` and `args?`. Every keymodel action becomes a command id: `tab:next`, `tab:close`, `pane:split:right`, `pane:focus:left`, `pane:close`, `pane:replaceWithLauncher`, `term:copy`, `term:paste`, `term:clear`, `doctab:reopen`, `editor:save`, `palette:open`, `view:zoom:in`, … The palette, keyboard, macOS menu, `run_command` and MCP all run the same ids.

### 6.2 Default bindings
One file, `frontend/app/keybindings/defaults.ts`, with rows of `{ key, command, when?, platform? }`:
- `key` uses one string syntax: `mod+shift+t`, `ctrl+[`, `code:Digit1`, chords as `ctrl+shift+s ArrowUp`.
- Two platform tokens, both explicit (§11.2):
  - `mod`: ⌘ on macOS, Ctrl on Windows/Linux. For actions that are editing conventions: save, find, zoom, select all.
  - `appmod`: ⌘ on macOS, **Ctrl+Shift** on Windows/Linux. For window, tab and pane actions. This is the GNOME Terminal / Konsole / Windows Terminal convention: a terminal never needs Ctrl+Shift+letter, so these keys never reach the shell.
- Match letters by `event.key`, falling back to `event.code` when a layout or AltGr changes the character (tinykeys' approach). Match digits and punctuation by `event.code`, so `appmod+1` works on AZERTY and German layouts (§11.4).
- Section 12 is the proposed default table.

### 6.3 Context ("when")
A small context service, read synchronously on each key:
- **Context keys:** `paneFocus`, `viewType`, `textInputFocus`, `terminalFocus`, `editorFocus`, `browserPaneFocus`, `modalOpen`, `paletteOpen`, `docTabsHost`, `dragActive`.
- **Replaces:** `shouldDispatchToBlock`, the `shellKeys` capability, `eventBelongsToBlock` gating and `disableGlobalKeybindings`.
- **Default rule:** a binding without `when` does **not** fire while `textInputFocus || terminalFocus || editorFocus`, unless it is in an allow-list (palette, tab switching, zoom). That one rule removes most of §3.3.

### 6.4 One dispatcher
- One `window` **capture-phase** listener resolves key + context to a command, runs it, then calls `preventDefault` + `stopImmediatePropagation`. Capture on `window` runs before Solid's delegated `document` listeners, so the pane-vs-global race in §3.1 goes away.
- **Adapters, not separate systems:**
  - xterm's hook asks the resolver "is this an app binding in this context?" and otherwise lets the shell have the key.
  - CodeMirror's `keymap.of` is generated from the same table for `when: editorFocus` rows.
  - Document tabs and the Files pane become table rows with `when: docTabsHost` / `viewType == files`.
- **Not bindings:** keys inside a widget (arrows in a list, Enter/Esc in one text field, the composer's history arrows) stay local, documented in the table as reserved so the conflict test knows about them.

### 6.5 Host layer
- The host's `OnPreKeyEvent` list and the macOS menu accelerators are **generated from the table**, for example a JSON file written at build time and read by `crates/cef`.
- In a browser pane, the host forwards app-level shortcuts (tab switch, palette, close pane) to the app, the same way it already forwards Ctrl+L as `browser-pane-shortcut`.

### 6.6 User overrides
- **New setting:** `keybindings: [{ key, command, when? }]`, replacing the dead `app:globalhotkey`. A row with `command: "-tab:new"` unbinds a default.
- **Merge order:** defaults, then the user's rows, then validation. Unknown commands and conflicts become a warning in Settings.
- **Live:** reloads through the existing settings watcher.

### 6.7 One set of helpers
- One `platform.ts` (`isMac`, `primaryMod`).
- One `isTypingTarget(el)`.
- One `formatBinding(key)` for every label: help pane, hamburger menu, palette, context menus, tooltips.
- Delete the duplicates listed in §4.

### 6.8 Conflict tests
- A unit test loads the merged table per platform. It fails when two rows share a key with overlapping `when`, or when a row shadows a reserved terminal, editor or text-field key (Ctrl+[, Ctrl+P in a terminal, Mod-D, Ctrl+Shift+arrows…).
- A snapshot of the expanded table per platform makes any change to a shortcut visible in review.
- A precedence test drives real DOM events through the dispatcher.

---

## 7. Help pane, docs and code from one source

- **The help pane renders the table.** Group by category, show the platform's keys through `formatBinding`, and mark conditional ones ("terminal only"). Hand-written labels go away, so the pane cannot drift.
- **The docs page is generated from the table.** A script emits the shortcuts table for agentmux-docs `keybindings.md`, and CI in the app repo fails when the table changes without regenerating it. Other docs pages link to that table instead of repeating keys.
- **Fix the docs now:**
  - `main-menu.md:3,9,17-18,22,37,39`: the palette is Ctrl+P on every platform; New Tab is Alt+T on Windows/Linux, not Ctrl+T; New Window is Ctrl+Shift+N on macOS too, not ⌘⇧N.
  - **Shortcuts that don't exist:** `pane-types.md:474` (`Cmd+Shift+A` new agent pane), `pane-types.md:178` (`Ctrl+H` replace), `config.md:9` (`Ctrl+,` Settings). Either build them (§9) or remove them.
  - `pane-types.md:452`: Ctrl+Shift+V does toggle the Markdown preview in the Editor.
  - `settings.md:87`: `app:globalhotkey` does nothing.
  - **`keybindings.json`:** `keybindings.md:209` (no effect) contradicts `config.md:89`, `data-layout.md:87` and `persistence.md:20,175`.
  - **Reload:** `building.md:148` vs `keybindings.md:59` (reload only during startup).
- **Fix the in-app hints now:** the terminal mic tooltip (term.tsx:498), and the editor empty state's Save hint.
- **Repo specs that are out of date:**
  - `docs/specs/archive/SPEC_PANE_CYCLE_FOCUS.md` (Tab/Ctrl+Tab vs. the real `Ctrl+[`/`]`).
  - `SPEC_FILE_BROWSER_PANE_2026_10_01.md:106` ("all bindings go through keyutil"; they don't).
  - `SPEC_MACOS_NATIVE_MENU_BAR` Phase 2 (accelerators): becomes §6.5.

### 7.1 Shortcut hints across the UI

Every place the UI shows a key gets its label from `formatBinding(command)` on the registry, so a hint can't disagree with the binding.

**The rule:**
- A menu item for a command that has a shortcut shows it.
- A component-local key (Enter or Esc in one search bar) may stay as literal text, but goes through `formatKey` so macOS shows ⌘/⌥/⇧.
- A test fails on a user-visible string matching `(Ctrl|Cmd|Alt|Mod)\+` or `⌘⌥⇧⌃` outside `formatBinding` / `formatKey`.

Inventory on `main` @ `7a2d019ae`:

| Where | Hint today | Verdict | Fix |
|---|---|---|---|
| Hamburger menu: New Tab, New Window, Command Palette (`hamburger-menu.tsx:86,93,134`) | generated by `formatKeyDescription` | Accurate | Keep, switch to `formatBinding` |
| Hamburger menu: Settings, Armory, DevTools, Exit | none | **Missing** once §12 gives them keys (Settings `mod+,`) | Generated hint |
| Pane context menu: Split Up/Down/Left/Right, Close Pane (`block/pane-actions.ts:193-204`) | none | **Missing**: Split Right, Split Down and Close Pane have shortcuts | Generated hints |
| Tab context menu (Close tab, Rename) | none | **Missing**: close tab has a shortcut; rename gets F2 | Generated hints |
| Help pane (`quicktips.tsx`) | its own `KeyBinding` renderer | Accurate where listed; incomplete (§2.2) | Render the registry (§7) |
| Terminal mic tooltip "Speak into this terminal (Ctrl+Shift+V)" (`term.tsx:498`) | Ctrl+Shift+V | **Wrong**: pastes in a terminal | Drop the key from the tooltip |
| Agent mic tooltip (`AgentFooter.tsx:1350`) | Ctrl+Shift+V | Accurate in the composer | Generated |
| Editor "Rendered preview (Mod+Shift+V)" (`editor-view.tsx:918`) and the empty state's Mod+S / Mod+F / Mod +/− (`:98-103`) | the literal word "Mod" | **Wrong label**: no key says "Mod"; Save does nothing on the empty screen | Generated, platform-specific; drop Save from the empty state |
| Document tabs "New tab (Ctrl+T)" (`DocTabStrip.tsx:50`, `editor-tab-strip.tsx:99`, `media-pane.tsx:219`) | Ctrl+T | Accurate | Generated |
| Files pane context menu and toolbar (`files-view.tsx:522-660, 718-806`) | hand-written `isMacOS() ? "⌘X" : "Ctrl+X"` | Accurate per handler. ⌘T, ⌘⇧N and ⌘W double-fire with the app's shortcuts on macOS (§3.3) | Generated; the double fire goes away with the dispatcher |
| Memory editors "Save (Ctrl/Cmd+S)", "Cancel (Esc)" (`AgentNativeMemoryModal.tsx:259`, `GlobalMemoryFullView.tsx:42`, `NativeMemoryFileView.tsx:153`) | "Ctrl/Cmd+S" | Accurate but not platform-specific | `formatKey("mod+s")` |
| Search bars "Previous (Shift+Enter)", "Next (Enter)", "Close (Esc)" (`element/search.tsx:139-155`, `AgentSearchBar.tsx:72-86`) | literal | Accurate | `formatKey` |
| Editor Save As path field "press Enter to save (Esc to cancel)" (`editor-tab-strip.tsx:142`) | literal | Accurate | `formatKey` |
| Copy-error fallback "Press Ctrl+C" (`errors/CopyErrorButton.tsx:171`) | Ctrl+C | **Wrong on macOS** (⌘C) | `formatKey("mod+c")` |
| Settings → Terminal "Shift+Enter → new line": "In agent composer: Shift+Enter inserts a newline…" (`terminal-section.tsx:49-50`) | — | **Wrong description**: the setting (`term:shiftenternewline`) is read only by the terminal (`termViewModel.ts:404`) | "In a terminal: Shift+Enter sends a newline instead of running the line" |
| Command palette rows | none | **Missing**: the palette should show each command's shortcut (VS Code does) | Generated |
| macOS native menu custom items (`macos_menu.rs`) | no accelerators | **Missing** (Phase 2 of the menu spec) | Generated (§6.5) |

---

## 8. Phases

| Phase | What | Risk |
|---|---|---|
| **0. Stop the damage** (small PR, before the framework) | Global bindings stop firing while a terminal, editor or text field has focus, except an allow-list (palette, tab switch, zoom). The terminal stops passing readline/vim keys (Alt+letters on Windows/Linux, Ctrl+P, Ctrl+[) to the global map. `Ctrl+Shift+K` asks for confirmation. Fix the mic tooltip. Remove the debug logger. | Low; fixes every High row in §3.3 |
| 1. Docs and help pane | Docs fixes in §7; add the palette and the missing shortcuts to the help pane | None |
| 2. Remove dead code | §5 items 1–10 (decide on the overlay first, Q3) | Low |
| 3. Registry and dispatcher | §6.1–6.4 and 6.7 for M1, M2 and M6; conflict tests (§6.8) | Medium: touches every global shortcut; the snapshot test guards it |
| 4. Adapters | Terminal, editor, document tabs, Files pane, composer reserved keys | Medium |
| 5. Host and menus | Generated `OnPreKeyEvent` list, macOS accelerators, browser-pane forwarding (§6.5) | Medium (Rust rebuild) |
| 6. Generated help pane and docs, user overrides | §6.6 and §7 | Low |

---

## 9. Proposed new shortcuts

These are included in the default table in §12, each checked against the conflict rules in §11:
- **Settings:** `mod+,`. The docs already promise it, and it's the Apple and VS Code convention.
- **Keyboard shortcuts sheet:** `mod+/` and `F1`, which open the generated help pane.
- **Reopen the last closed window tab:** ⇧⌘T / Ctrl+Shift+Alt+T.
- **New agent pane:** `appmod+a` (`⌘⇧A` on macOS…). The docs already promise one.
- **Last tab:** `mod+9`. This follows Chrome's rule that 9 means last.
- **Move the current tab left or right:** ⇧⌘PgUp / PgDn, Ctrl+Shift+Alt+PgUp / PgDn.
- **Swap the focused pane with its neighbour:** ⌃⇧⌥arrows / Ctrl+Shift+Alt+arrows.
- **Cycle pane focus:** F6 / Shift+F6, the Windows convention for moving between a window's panes. This frees Ctrl+[ (vim's Escape).
- **Resize the focused pane:** Alt+Shift+arrows on Windows/Linux (Windows Terminal's default) and ⌃⌥⌘arrows on macOS.
- **Rename the window tab:** `F2` while the tab strip has focus.
- **Reset zoom on all panes:** `mod+shift+0`.
- **Numbered pane overlay:** shown while ⌃⇧ / Ctrl+Shift is held for 500 ms, which rebuilds the broken overlay (§5, item 1).
- **Command palette:** `appmod+p` (`Ctrl+Shift+P` / `⌘⇧P`), the VS Code and Windows Terminal key. `Ctrl+P` stays as a second binding outside terminals. The dev perf HUD moves to `ctrl+alt+shift+p`.
- **Focus the agent composer:** `mod+l` in an agent pane.

---

## 10. Decisions (owner, 2026-10-04)

The owner accepted the recommendations and asked that they follow researched best practice (§11):

1. **Windows/Linux window, tab and pane shortcuts move off Alt to `appmod` = Ctrl+Shift**, with a changeset and a release note. macOS keeps ⌘.
2. **While typing, global shortcuts don't fire** in text fields, terminals and editors, except the shell-skipping allow-list (§11.3).
3. **The numbered pane overlay is rebuilt,** not removed.
4. **User-configurable shortcuts** come after the registry settles (phase 6).
5. **The new shortcuts in §9 are adopted** as listed in §12, each subject to the conflict test.

---

## 11. Best practices (researched 2026-10-04)

### 11.1 One registry, context rules, last match wins (VS Code)
VS Code keeps every binding in one registry. Each binding has a command and an optional `when` clause built from context keys such as `editorTextFocus` or `terminalFocus`. When a key is pressed, the rules are checked from the bottom up, and the first one that matches both the key and its `when` wins. User rules are appended after the defaults, so they override them, and the file is watched and applied live. VS Code also has a "Show Same Keybindings" view for conflicts and a "Keyboard Shortcuts Troubleshooting" log of every dispatched key. ([when clause contexts](https://code.visualstudio.com/api/references/when-clause-contexts), [keybindings docs](https://code.visualstudio.com/docs/configure/keybindings))
→ **Adopted:** §6.1–6.3 and §6.6 follow this model. Add a "Show same keybindings" view and a key-dispatch troubleshooting log, replacing the current `console.log` calls (§5, item 6).

### 11.2 Terminal emulators use Ctrl+Shift, not Alt, for app shortcuts
Windows Terminal's defaults:
- **Tabs:** `ctrl+shift+t` new tab, `ctrl+shift+w` close, `ctrl+tab` / `ctrl+shift+tab` switch, `ctrl+alt+1…9` go to tab N.
- **Panes:** `alt+shift+d` / `-` / `+` split, `alt+arrows` move focus, `alt+shift+arrows` resize.
- **Other:** `ctrl+shift+p` palette, `ctrl+,` settings, `ctrl+shift+f` find, `ctrl+shift+c` / `v` copy and paste, `ctrl+plus` / `minus` / `0` font size.
([Windows Terminal actions](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/actions))

Terminal users depend on Alt as Meta for readline and editors, and other terminals' defaults are known to clash with editor keys, so the guidance is to move app shortcuts to keys nothing else uses. ([terminal setup notes](https://pi.dev/docs/latest/terminal-setup))
→ **Adopted:** `appmod` = Ctrl+Shift on Windows/Linux (§6.2). Alt+letter stays free for the shell everywhere. Tab N uses Ctrl+1–9 (Chrome's convention) rather than Windows Terminal's Ctrl+Alt+1–9, which is AltGr on many layouts (§11.5).

### 11.3 A terminal gets every key except an explicit skip-list (VS Code)
VS Code's integrated terminal sends keys to the shell unless the command is in `terminal.integrated.commandsToSkipShell`. Users can add to that list, or remove a default with a minus prefix. A setting, `sendKeybindingsToShell`, flips the default, at the cost of shortcuts like Ctrl+F. Chords skip the shell unless `allowChords` is off, and menu mnemonics (which take Alt) are opt-in. ([VS Code terminal advanced](https://code.visualstudio.com/docs/terminal/advanced))
→ **Adopted:** in a terminal, only commands on a skip-list fire: the palette, tab and pane commands on `appmod`, zoom, copy/paste on Ctrl+Shift+C/V, and close. Everything else, including `Ctrl+P`, `Ctrl+[` and every `Alt+` key, goes to the shell. That is phase 0's main fix, and the skip-list becomes user-editable in phase 6.

### 11.4 `event.key` vs `event.code`
`key` is the character typed. It changes with the layout, so `Z` is `Y` on German QWERTZ. `code` is the physical key, which stays put across layouts but no longer matches the letter printed on the key. ([javascript.info: keyboard events](https://javascript.info/keyboard-events))

tinykeys matches either. It recommends `code` for letters under AltGr and has a `$mod` alias (⌘ on macOS, Ctrl elsewhere). ([tinykeys](https://github.com/jamiebuilds/tinykeys))
→ **Adopted:** letters by `key` with a `code` fallback; digits and punctuation (`[`, `]`, `=`, `-`, `/`, `,`) by `code`, so that `appmod+1` and `mod+=` work on AZERTY, where the digit row needs Shift.

### 11.5 Platform conventions
- **macOS:** don't repurpose standard shortcuts (⌘N, ⌘O, ⌘W, ⌘S, ⌘⇧S, ⌘P, ⌘Z, ⌘⇧Z, ⌘X/C/V, ⌘A, ⌘F, ⌘G, ⌘,, ⌘H, ⌘Q). ⌘⇧, ⌘⌥ and ⌘⌃ are for secondary actions, and ⌘. means cancel. ([Apple HIG: keyboards](https://developer.apple.com/design/human-interface-guidelines/keyboards))
  → **Adopted:** ⌘G stops being "change connection" (it's Find Next on macOS) and moves to ⌘⇧G. ⌘P is Print by convention; AgentMux has nothing to print, so it stays the palette as in VS Code, plus ⌘⇧P.
- **Windows:** Ctrl+letter and the function keys are the best choices for app shortcuts. Avoid Alt+Esc, Alt+F4, Ctrl+Esc and Ctrl+F4, and don't make several modifiers the only way to reach basic operations. ([Win32 keyboard accelerators](https://learn.microsoft.com/en-us/windows/win32/menurc/about-keyboard-accelerators), [Windows app keyboard accelerators](https://learn.microsoft.com/en-us/windows/apps/develop/input/keyboard-accelerators))
  → **Adopted:** every multi-modifier shortcut is also reachable from the palette, the hamburger menu or a context menu (§12 "also in").
  - **Avoid bare `Ctrl+Alt+key`.** On Windows, Ctrl+Alt is AltGr, which types characters on German, Polish and other layouts (AltGr+Q is @ on German). The few Ctrl+Shift+Alt rows in §12 are guarded as described there.

### 11.6 Accessibility (WCAG 2.1.4, level A)
A shortcut made of one character key (no modifier) must either be possible to turn off, be remappable to include a modifier, or be active only while its component has focus. ([WCAG 2.1.4 explained](https://getstark.co/wcag-explained/operable/keyboard-accessible/character-key-shortcuts))
→ **Adopted:** the Files pane's `/`, Space and type-ahead, and `F2`, are scoped to the focused component by `when`. The conflict test fails on any unscoped single-character binding.

---

## 12. Proposed default table (from §6, §9, §10 and §11)

`mod` = ⌘ / Ctrl; `appmod` = ⌘ / Ctrl+Shift.

**One rule for the Shifted variants:**
- macOS adds ⇧ to ⌘.
- Windows/Linux adds Alt to Ctrl+Shift, so ⇧⌘W becomes Ctrl+Shift+Alt+W.
- Every such row is also in the palette and a menu (§11.5).

"Skips shell" means the binding fires even with a terminal focused (§11.3). Keys are matched as in §11.4.

| Command | macOS | Windows / Linux | When | Skips shell | Before (Win/Linux) |
|---|---|---|---|---|---|
| Command palette | ⇧⌘P, ⌘P | Ctrl+Shift+P; Ctrl+P outside terminals | — | Ctrl+Shift+P | Ctrl+P |
| Settings | ⌘, | Ctrl+, | — | yes | — |
| Keyboard shortcuts sheet | ⌘/, F1 | Ctrl+/, F1 | — | F1 | — |
| New window | ⇧⌘N | Ctrl+Shift+N | not in the Files pane (new folder) | yes | Ctrl+Shift+N |
| New tab | ⌘T | Ctrl+Shift+T | not `docTabsHost` | yes | Alt+T |
| Reopen closed tab | ⇧⌘T | Ctrl+Shift+Alt+T | not `docTabsHost` | yes | — |
| Close tab | ⇧⌘W | Ctrl+Shift+Alt+W, Ctrl+F4 | — | yes | Alt+Shift+W |
| Next / previous tab | ⌘] / ⌘[, ⌃Tab / ⌃⇧Tab | Ctrl+Shift+] / [, Ctrl+Tab / Ctrl+Shift+Tab | Tab variants not in `docTabsHost` | yes | Alt+] / Alt+[ |
| Go to tab 1–8 / last | ⌘1–8 / ⌘9 | Ctrl+1–8 / Ctrl+9 | — | yes | Alt+1–9 |
| Move tab left / right | ⇧⌘PgUp / PgDn | Ctrl+Shift+Alt+PgUp / PgDn | — | yes | — |
| New terminal pane | ⌘N | Ctrl+Shift+` | — | yes | Alt+N |
| New agent pane | ⇧⌘A | Ctrl+Shift+A | — | yes | — |
| Split right / below | ⌘D / ⇧⌘D | Ctrl+Shift+D / Ctrl+Shift+Alt+D | ⌘D not in `editorFocus` (next match) | yes | Alt+D / Alt+Shift+D |
| Split in a direction | ⌃⇧S then arrow | Ctrl+Shift+S then arrow | not `editorFocus` (Save As) | yes | same |
| Close pane | ⌘W | Ctrl+Shift+W | not `docTabsHost` | yes | Alt+W |
| Maximize pane | ⌘M | Ctrl+Shift+M | — | yes | Alt+M |
| Terminal multi-input | ⇧⌘M | Ctrl+Shift+Alt+M | 2+ terminals | yes | Ctrl+Shift+M |
| Focus pane by direction | ⌃⇧arrows | Ctrl+Shift+arrows | not `textInputFocus` / `editorFocus` | yes | same, but everywhere |
| Cycle pane focus | F6 / ⇧F6 | F6 / Shift+F6 | — | yes | Ctrl+] / Ctrl+[ (took vim's Esc) |
| Focus pane N | ⌃⇧1–9 | Ctrl+Shift+1–9 | — | yes | same |
| Show pane numbers | hold ⌃⇧ 500 ms | hold Ctrl+Shift 500 ms | — | — | (broken) |
| Swap pane with neighbour | ⌃⇧⌥arrows | Ctrl+Shift+Alt+arrows | — | yes | — |
| Resize pane | ⌃⌥⌘arrows | Alt+Shift+arrows | not `textInputFocus` / `editorFocus` | yes | — |
| Replace pane with launcher | ⌃⇧K, with confirmation | Ctrl+Shift+K, with confirmation | not `editorFocus` | yes | Ctrl+Shift+K, no confirmation |
| Change connection | ⇧⌘G | Ctrl+Shift+G | `paneManagesConnection` | yes | Alt+G |
| Find in pane | ⌘F | Ctrl+F; Ctrl+Shift+F in a terminal | — | Ctrl+Shift+F | Alt+F |
| Voice input | ⌃⇧V | Ctrl+Shift+V | voice-capable pane, not a terminal or shell drawer (paste) | no | same |
| Zoom pane in / out / reset | ⌘= / ⌘- / ⌘0 | Ctrl+= / Ctrl+- / Ctrl+0 | — | yes | same |
| Reset zoom on all panes | ⇧⌘0 | Ctrl+Shift+0 | — | yes | — |
| Terminal copy / paste | ⌘C / ⌘V | Ctrl+Shift+C / Ctrl+Shift+V | `terminalFocus` | yes | same |
| Terminal clear scrollback | ⌘K | Ctrl+Shift+L | `terminalFocus` | yes | Alt+K |
| Focus the agent composer | ⌘L | Ctrl+L | agent pane, not `browserPaneFocus` | — | — |
| Refocus pane | ⌘I | dropped (Ctrl+Shift+I is the DevTools convention) | — | — | Alt+I |

**AltGr (§11.4):** on Windows, Ctrl+Alt is AltGr, so `Ctrl+Shift+Alt+…` rows could swallow a character a layout types with AltGr+Shift. The dispatcher skips a binding when `getModifierState("AltGraph")` is set and `event.key` is a printable character other than the bound key. The conflict test covers the common layouts' AltGr letters.

**To check live before phase 3 lands:**
1. Ctrl+Shift+digit against Windows' language-switch hotkey (Ctrl+Shift by default on some installs).
2. Ctrl+Shift+` on non-US layouts.
3. Ctrl+Shift+Alt+letter on German and Polish layouts.
4. F6 inside CodeMirror.

---

## 13. Implementation progress

| Phase | PR | What landed |
|---|---|---|
| 0 | #4320 | The destructive conflicts in §3.3, with no key changes. Terminal clear on Windows/Linux moved from Alt+K to Ctrl+Shift+L so Alt+K reaches the shell. |
| 2 | #4321 | §5's dead code. The numbered pane overlay is rebuilt on DOM key events. `app:globalhotkey` removed. |
| 3 | #4323 | The shortcut table (`frontend/app/keybindings/`), the §12 key map, the dispatcher and the terminal on the table, and every UI hint generated from it (§7.1). Covers the help pane, the hamburger and pane menus, the command palette and tooltips, plus a test that fails on a hand-written hint. The dev perf HUD and diagnostics panel are table rows (dev-only) on Ctrl+Alt+Shift+P and Ctrl+Alt+Shift+F12. |

**Added after phase 3** (#4324):
- move a tab: ⇧⌘PgUp / PgDn, Ctrl+Alt+Shift+PgUp / PgDn;
- rename the window tab: F2, when nothing focused takes it first;
- swap the focused pane with a neighbour: ⌃⌥⇧arrows / Ctrl+Alt+Shift+arrows;
- resize panes: ⌃⌥⌘arrows / Alt+Shift+arrows. These move the nearest border in the arrow's direction, as Windows Terminal does;
- reset zoom on all panes: ⇧⌘0 / Ctrl+Shift+0;
- focus the agent message box: ⌘L / Ctrl+L in an agent pane.

**Still deferred:** reopening a closed window tab. Closing a tab deletes its panes on the server, so reopen needs a backend "recently closed tabs" store before a key can bind to it. It isn't bound or shown anywhere.

**Still to do:**
- Phase 4 (#4325): **done** for document tabs, the editor and the Files pane. They are `pane` rows in the table, matched by `matchPaneKey`. CodeMirror's Save keys are generated from it, and the help pane has Documents, Editor and Files sections. Global rows sharing a key are scoped away from that pane, and the conflict check proves it. The composer's keys (Enter, Esc, history arrows) are widget-internal and stay local.
- Phase 5 (#4326): **done for browser panes.** `host-keys.json` is generated from the table and pinned by `host-keys.test.ts`. `crates/cef` reads it, so a focused browser pane forwards app shortcuts to the app (`app-shortcut` event), consumed before the page sees them.
  - **What's forwarded:** window, tab and pane commands that skip the shell, plus the palette, Settings and the shortcuts sheet.
  - **Left out:** find and zoom, which a page has its own of; typing-scoped keys, because the host can't see whether the page has a text field focused (F6 still leaves the pane); chords.
  - **Decided against: macOS native menu key equivalents.** An NSMenu key equivalent takes the key before the web view sees it, so it would bypass the table's `when` rules: ⌘W would close the pane even in the Files pane, whose own ⌘W closes a folder tab. The menu keeps running commands by click (`menu:invoke`), and its items show no accelerators. The command palette and the help pane show every key instead.
- Phase 6 (#4328): **done.**
  - **A `keybindings` setting** (schema, TS type, `setUserKeybindings`), applied live:
    - user keys come before the defaults, so they win;
    - `-command` unbinds a command, every key or just one;
    - optional `when` and `platform`;
    - a bad entry is skipped with a console warning.
  - **Overrides show everywhere:** the help pane, menus and palette read the effective rows.
  - **`docs/keybindings.md`** is generated from the table and pinned by `doc.test.ts`. The docs site's `keybindings.md` takes the same content, in its own PR, merged once these keys ship in a release.
