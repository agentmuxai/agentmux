# Spec: Settings > Keyboard shortcuts: rebind every shortcut, with Help and the rest following live (#1861)

**Date:** 2026-11-06 · **Owner:** AgentA (shortcuts and Help work), at the request of AgentY and the operator (2026-11-06: "we want to be able to set the keybindings. it would be a new view in settings. in that case, the help panel would reactively show the shortcuts set in the settings and the default is what we have today. use latest best practices for managing the UI").
**Status:** proposed. Nothing in this spec is built. The build (#1862) starts when this task is approved.
**Written against:** `origin/main` @ `7b6b4790f` (2026-11-06).
**Builds on:** `docs/reports/REPORT_KEYBINDINGS_AUDIT_AND_CONSOLIDATION_2026_10_04.md` (§6 "One framework", §11 best practices), `docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md`, `docs/specs/PLAN_SHORTCUT_KINKS_2026_10_10.md`, `docs/keybindings.md` (generated).

## 1. Summary

**The single command registry the brief asks for already exists, and so does the override format.** `frontend/app/keybindings/defaults.ts` is the one table (command id, label, category, keys per platform, `when`, terminal pass-through, owning pane). `registry.ts` resolves a key press against it. A `keybindings` array in `settings.json` (VS Code's shape: `{key, command, when?, platform?}`, a `-command` entry to unbind) is applied over the defaults whenever it changes, and the Help pane re-renders from it. Overrides are a diff from the defaults, so a key added to the defaults later still reaches a person who rebound something else. The dispatcher, Help, the generated `docs/keybindings.md`, the palette, tooltips, the host key list and the MuxAPI `ListShortcuts` all read this table; a test (`no-hardcoded-hints.test.ts`) fails if a UI file hard-codes a key.

**What a person cannot do is use any of it without editing JSON by hand.** This spec adds the editor, and closes the five gaps that stop "rebind anything and everything follows" being true:

| # | Gap (read in the code on 2026-11-06) | Fix |
|---|---|---|
| G1 | No UI: the setting is hand-edited JSON; no recording, no conflict warning while editing (`findConflicts` only runs in tests). | The Settings view (§4) and a live conflict check (§5). |
| G2 | Tooltips and palette hints are not reactive: `shortcutFor` reads the table but not `keybindingsVersion`, so only Help follows a change. | Make every label helper track the version; audit the call sites (§6). |
| G3 | A browser pane is a native CEF child window, so its keys are forwarded by the host from a **build-time list of the default keys** (`host-keys.json`). A key the person binds is never forwarded; one they unbind still is (and is then ignored by the app). | Push the effective list to the host when it changes (§7). |
| G4 | An entry that cannot be used is skipped with a `console.warn`: the person never sees it. | Show it as a row-level error in the view (§4.4) and in Help. |
| G5 | Reserved keys (the OS or browser takes them) are not modelled, so a person can bind a key that can never arrive. | A reserved-keys table with block and warn tiers (§5.3). |

The rest (chords, layouts, terminal pass-through, agents) is already handled by the registry; §8 lists what it does today and the few things to add.

## 2. Research (short; sources in §12)

What mature apps do, and what we take from each:

| | Record a key | Conflicts | Storage | Platform defaults | Reset | Search |
|---|---|---|---|---|---|---|
| **VS Code** | A "Define Keybinding" box listens for the combination; a chord is two presses | Shown, never blocked; "Show Same Keybindings" lists them; `when` decides; **last matching rule wins** | `keybindings.json` holds **only user rules**, appended after the defaults; `-command` removes a default | Cmd / Win / Meta per OS; Settings Sync keeps shortcuts per OS by default | Reset or Remove per row | By action text **and by pressing keys** (Record Keys) |
| **JetBrains** | "Add Keyboard Shortcut", press it; a checkbox makes the second stroke | A dialog on assignment: remove the other, leave both, or cancel (the wording is from a community answer; verify in the UI before copying it) | A built-in keymap is read-only: editing makes a child keymap that stores only the changes | Keymaps per OS (macOS, macOS System Shortcuts, Windows, Linux); flags OS conflicts and can filter to them | Reset one; Restore Defaults for all | By action; "Find Action by Shortcut" |
| **Zed** | Keymap editor with keystroke recording; changes written to `keymap.json` | Resolved by context depth, then the later binding; a `null` action disables a key and stops fallback | `keymap.json`: an array of `{context, bindings}` layered over per-OS default files | Per-OS default files and a selectable base keymap | Edit the file | Filter in the editor |
| **Slack** | none (read-only list on Ctrl+/) | n/a | n/a | Separate Mac and Windows/Linux tables | n/a | scan the list |
| **Discord** | "Record Keybind", press, "Stop Recording"; several per action | not described | per user, desktop only | n/a | delete the entry | n/a |
| **macOS System Settings** | Double-click, press the combination | "Your new shortcut won't work": nothing is swapped, you change one of them | per app, by exact menu title | n/a | A checkbox disables without deleting; Restore Defaults | none for keys |

Layout and platform facts that matter for a Chromium shell: `KeyboardEvent.code` is the physical key and ignores the layout, `key` is the produced character; `navigator.keyboard.getLayoutMap()` (Chromium only, secure context) maps a `code` to the label on the person's layout, which is how to show a physical-key binding by its real name; keys must be ignored while `isComposing` (IME); WCAG 2.1 SC 2.1.4 (Level A) asks that a single-character shortcut can be turned off or remapped to include a modifier, or fire only with focus in the component.

**Best practices we adopt** (each is in §4 to §7):
1. A diff over the defaults, in a human-editable file (the existing `keybindings` array; no second store).
2. Record by pressing keys, with Clear and Cancel; chords of two steps; Enter and Escape recordable through a toolbar button, as JetBrains does.
3. Search by action text and by pressing a key.
4. Conflicts shown while recording, with an explicit choice; never a silent "it won't work".
5. A visible "changed" mark, reset per row, reset all.
6. An explicit unbind that blocks fallback (`-command`), and a row that can hold no key.
7. Match letters by what is typed and digits and punctuation by physical key (what `parseKey` does today), label with the person's layout.
8. Modifier required for printable keys by default, with an off-switch and a `when` guard for bare keys (WCAG 2.1.4).
9. Per-platform overrides, so a Cmd binding made on a Mac never means Win on Windows.
10. The view is a keyboard-operable list, using the kit's rows and controls.

## 3. One source of truth (what stays, what is added)

The table, its row shape and `parseKey` / `matchKey` stay as they are. Three additions, all in `frontend/app/keybindings/`:

1. **`registry.ts: bindingsView(platform)`**: for each command, `{command, label, category, defaults: string[], keys: string[], changed: boolean, unbound: boolean, issues: string[]}`, computed from `DEFAULT_KEYBINDINGS` and the user entries. The Settings view, Help, `ListShortcuts` and the palette read this; none re-derives it. Today `listShortcuts` and `helpSections` each walk `effectiveRows`.
2. **`edit.ts` (new, pure)**: `planChange(userEntries, command, newKeys, platform) -> {entries, conflicts}` turns "this command should now have these keys" into the smallest list of entries, and `conflictsFor(candidate)` finds what a key would collide with (§5). No UI, no store access, fully unit-tested.
3. **`reserved.ts` (new)**: the reserved-keys table (§5.3).

**How an edit is written** (the file stays a diff):
- A default key the person removes becomes `{"command": "-tab:new", "key": "ctrl+shift+t"}`; a key they add becomes `{"command": "tab:new", "key": "ctrl+alt+t", "platform": "other"}`.
- Removing every key of a command writes the bare `{"command": "-tab:new"}` (no `key`), so a default added later does not bring it back; this is what "unbound" means.
- Resetting a row deletes every entry for that command; "Reset all" empties the array.
- Entries for a command are rewritten as a group, in order (unbinds first, then adds), because the registry applies them in list order.
- Every entry the view writes carries `platform` (`"mac"` on macOS, `"other"` on Windows and Linux), so a settings file shared across machines never turns a Mac's `meta+k` into `Win+K`. A hand-written entry without `platform` keeps working as today and shows in the view for both platforms. (Linux uses `other` entries over the Linux default column; Linux defaults that differ from Windows are shown as the defaults on Linux.)

## 4. The Settings view

### 4.1 Where

A new section **Keyboard shortcuts** in Settings (`SettingsSection` gets `"keyboard"`; a rail entry; its rows go in `SETTINGS_INDEX` so Settings search finds "shortcut", "keybinding", "hotkey"). The Help pane's shortcut page gets a **Customize** link that opens it, scrolled to the command. Both open from the command palette (`app:settings` plus a `settings:keyboard` command).

### 4.2 Layout

- A toolbar: search box, a **Record keys** toggle next to it (§4.5), category filter, a **Changed only** filter, **Reset all**, **Import / Export** (§9).
- One list, grouped by category (the table's categories), a row per command (a row per `helpGroup` for "Go to tab 1 to 8": expandable to its members).
- A row: label, the command id in muted text (also what agents and the JSON use), the current keys as chips, a **changed** badge when it differs from the default, the default keys in muted text when changed, a pencil/Add button, **Reset** (only when changed), an **Unbind** control. A chord shows as `Ctrl+Shift+S then ↓`, as everywhere else.
- Rows with an `issues` entry (G4) show it inline in the error style ("unknown command", "a chord is at most two keys"), with a **Remove entry** button.
- Dev-only rows (`devOnly`) are hidden, as in Help. Rows for panes that are not open are still shown (the person is editing the map, not the pane).
- Styles come from the kit (list row, chip, button, input); no new colour or size tokens. The new `keyboard-section.tsx` is ordinary section code in `view/settings/sections/` with its state in a `keyboard-section-state.ts` next to it (the pattern of `notifications-section*.ts`).

### 4.3 Keyboard and screen-reader behaviour

The view is a list the keyboard can drive: Up/Down between rows, Enter on a row starts recording, Delete unbinds the focused key chip, Esc cancels a recording, `/` focuses the search. A recording state is announced ("Press the new shortcut for New tab. Escape cancels.") with `aria-live="polite"`; a conflict is announced when it appears. Row actions have visible labels, not icon-only buttons. The view must pass the existing settings accessibility tests and add its own.

### 4.4 Invalid and unknown entries

`setUserKeybindings` already returns a message per skipped entry. It will also return the entry's index and command, and the view lists them under the row (or under "Other entries" when the command is unknown). Nothing is deleted automatically.

### 4.5 Recording a key

Clicking a key chip (or Enter on a row) opens an inline capture field:
1. It listens to `keydown` in the capture phase on the field, calls `preventDefault`, and records the pressed combination as the string the table uses (`parseKey` syntax): modifiers in the fixed order, a letter by `key`, a digit or punctuation key as `code:` (what `parseKey` expects for layout independence), named keys by `key`.
2. While recording, **the global dispatcher is off** (the same switch the command palette uses), so pressing a bound key records it instead of running it.
3. A modifier on its own never records. Escape cancels; **Enter and Escape are recordable** through the field's toolbar button, as in JetBrains.
4. A second press within the chord timeout makes a **two-step chord** (the existing maximum); the field shows "then…" and a Done button; no third step.
5. Ignored while `isComposing` (IME) and on key repeat.
6. The proposed key shows live with its conflicts (§5) before the person confirms with Enter or **Save**.
7. A key typed as a **bare printable character** (no Ctrl, Alt or Meta) is refused with a reason unless the person opens **Advanced: allow plain keys**, and then it is saved with the guard `!textInputFocus && !terminalFocus` (WCAG 2.1.4).

**Search by key:** the **Record keys** toggle next to the search box captures a key and filters the list to every command bound to it (VS Code's Record Keys).

## 5. Conflicts

### 5.1 What counts

Two bindings conflict when they are for the same platform, the same key (the full chord, or a single key that is the first step of the other's chord: "shadows"), different commands, and their contexts can both hold (`whenDisjoint` on the `when` clauses and the pane scopes, as `findConflicts` does today). The existing logic is reused; it gains `conflictsFor(candidate)` for one candidate key instead of walking every pair.

### 5.2 How it is shown and resolved

While recording, a conflict appears below the field: "Ctrl+Shift+D is *Split right*. In the same places as *Close pane*." with three choices (JetBrains' set, plus VS Code's context option):
- **Replace**: unbind the other command's key (writes a `-other key` entry) and bind this one.
- **Keep both**, only offered when the contexts differ, or when the person gives this binding a context (a `when` picker: "Everywhere", "Not in a text field", "Not in a terminal", "Only in: Terminal / Editor / Files / Browser"). The picker writes the existing `when` strings.
- **Cancel**.
"Replace" never touches a *pane-scoped* default silently: a pane row that the new key would hide is named in the message. Saving is not possible while a same-context conflict is unresolved.

### 5.3 Reserved keys

`reserved.ts` lists keys the app can never receive or must not take, by platform, in two tiers:
- **Blocked** (cannot arrive, or would trap the person): the OS secure attention sequence (Ctrl+Alt+Del), Alt+F4 / Cmd+Q / Cmd+Tab / Cmd+Space / Win-key combinations the OS takes before any app, the `Esc` that closes dialogs (it is fixed), and the keys the recorder uses to cancel.
- **Warned** (the desktop environment or a common input method often takes it): the GNOME and KDE workspace and window keys already measured in `PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md` §2, macOS Mission Control and Spaces keys, Ctrl+Alt+letter on Windows (AltGr on many layouts, see `matchKey`), and Ctrl+Shift+0 (layout switch on some Windows setups). A warned key can be saved after the person confirms.
The tables cite the existing measurements; a key with no measurement is not listed (we do not guess).

## 6. Everything else follows the person's bindings

### 6.1 The rule

Anything that shows or runs a key reads `bindingsView` / `keysFor`, and **tracks `keybindingsVersion`** so a change re-renders it. Today only `shortcutHelp()` does. G2 is two changes:
1. `shortcutFor`, `keyLabel` of a command and `codeMirrorKeys` read `keybindingsVersion()` first, so any JSX or memo that calls them re-runs.
2. A short audit of every call site (about thirty files import the helpers) for a value computed once outside a reactive scope: a constant at module load, a `createMemo` that does not call the helper, a menu built once. The build's first PR lists them and fixes each; `no-hardcoded-hints.test.ts` gains a check that no file calls the helper at module top level.

### 6.2 Help

Help shows the current key, and when it differs from the default shows "Default: Ctrl+Shift+T" in muted text beside it. A command with no key shows "Not set" with a **Set…** link that opens the Settings row. The page's text for "how to customise" (today a JSON recipe in `docs/keybindings.md`) becomes "Change in Settings"; the JSON recipe stays in the docs for people who edit the file. The audit in §6.1 also checks Help for any hand-written key text that ignores remaps (the 2026-10-10 plan listed some; the zoom keys have since moved to the shared pane zoom).

### 6.3 Command palette, tooltips, menus

The palette shows each command's current key; tooltips use the helper; context menus use it. The macOS application menu has no accelerators (the app owns the chords, `crates/cef/src/macos_menu.rs`), so nothing there changes.

### 6.4 `docs/keybindings.md`

Stays generated from the **defaults** (it documents what ships); the doc gains one line saying the in-app Help and Settings show the person's own bindings.

## 7. Browser panes (G3)

A focused browser pane never sees the app's DOM key events: the CEF host forwards the keys in `host-keys.json` (generated from the defaults at build time) as `app-shortcut`, and the app then runs whatever the effective table says for that key. So a key the person **binds** is never forwarded (the page gets it), while a key they **unbind** is still forwarded and ignored.

Fix, in the build's last phase: the app computes `hostKeys()` from the **effective** table (the same filter the generator uses: global rows allowed in a terminal, single keys, not find/zoom) whenever `keybindingsVersion` changes, and sends it to the host over the existing host IPC; the host replaces its list and keeps `host-keys.json` as the startup list. Which IPC message carries it, and whether the host list is per window, is confirmed in the first hour of that phase by reading `crates/cef` `forward_app_shortcut` (it was not read for this spec beyond the file's header comment). If the host change proves large, this phase can ship later without blocking the rest; the Settings view says, next to a rebound key, "Not applied while a browser pane has focus" until it does (honest, not silent).

## 8. Edge cases

| Case | Behaviour |
|---|---|
| Chords | At most two steps (kept). The recorder supports them (§4.5); a single key that is the first step of a chord is reported as a conflict ("shadows"). |
| A focused terminal | Only rows with `skipShell` run there; every other key belongs to the shell. A binding the person makes defaults to **not** skipping the shell unless they pick "Everywhere, including terminals" in the `when` picker (which writes `skipShell` through the `terminalFocus` clause, as `registry.ts` does today). The editor and the palette warn that a terminal keeps the key. |
| Editor (CodeMirror), Files list | Pane rows (`pane:`) are matched by the pane's handler; the editor keymap is built from the table (`codeMirrorKeys`) and re-reads it on a change (it needs the reactive helper, §6.1). |
| The agent composer | A text input: only modifier combinations or `!textInputFocus` bindings fire there. The recorder refuses bare keys (§4.5). |
| Browser panes | §7. |
| A widget's commands | Not bindable, as today (`ext:` ids are rejected so a key cannot join the table agents' `RunCommand` reads). The Settings row explains why. |
| Non-US layouts | Letters match the typed character with a physical-key fallback; digits and punctuation match `code`, so AZERTY and German layouts keep working; the recorder stores punctuation as `code:`. A `code:` key is labelled with `navigator.keyboard.getLayoutMap()` when available (Chromium), else with the US name. AltGr (Ctrl+Alt) combinations that type a character are not matched (`matchKey`). |
| IME | Keys are ignored while composing. |
| A key empty after unbinding | Allowed. The row shows "Not set"; the command stays in the palette and can still run by name. A bare `-command` entry is the stored form. |
| Existing custom bindings | The array is already live; nothing to migrate. The view reads existing entries as they are, flags the unusable ones (G4) and rewrites only the entries of a command the person edits. A hand-written entry the view cannot represent (an unusual `when`) is shown as raw text and edited as text, never silently rewritten. |
| New defaults | Reach everyone who did not override that key, because the file is a diff. A bare `-command` keeps a command unbound on purpose; a `-command key` entry removes only that key. |
| Linux | The `linux` default column (the Ctrl+Shift chords chosen in #4641) stays the default; user entries use `platform: "other"` and apply on Linux too. |
| Two windows | Settings changes arrive through the existing settings signal in every window. |
| Settings write fails | The view keeps the person's edit visible and shows the error; it never half-applies. |
| Mouse and gesture rows (`tips.ts`) | Not rebindable (they are not keys); they keep appearing under "Mouse and gestures" in Help. |

## 9. Import, export, sync

The format is the `keybindings` array the person already can read. **Export** copies it (or saves it) as JSON; **Import** shows a preview ("12 changes: 3 conflict with your current ones") and merges or replaces. There is no separate keymap file. Settings sync, if it exists for `settings.json`, carries the array; entries carry `platform`, so a Mac's and a PC's bindings coexist. A named "keymap" layer (VS Code or JetBrains presets) is out of scope (§11, D8).

## 10. MuxAPI

`ListShortcuts` already returns `{command, label, category, keys, raw}` per command from `effectiveRows`. It gains `defaults`, `changed` and `unbound`, from `bindingsView`, so an agent can tell a person's own key from a default. `RunCommand`, `PressKeys` and their refusal list are unchanged.

**Whether agents may change bindings is a decision (D6).** Recommendation: **yes, with the protections the workspace rule asks for**: `POST /api/v1/ui/shortcuts/set` and `/reset` (beside the existing `list`, `run` and `press`) write the same `keybindings` entries through the same `planChange`, refuse reserved keys and same-context conflicts (return them instead of replacing), keep the previous array so the person can undo from the view ("Undone N changes by AgentX"), and show a notice. Agents already drive `RunCommand` and `PressKeys`; a rebound key only changes which key reaches the same commands. Reading is open to all.

## 11. Decisions for the operator

1. **Storage.** Use the existing `keybindings` array in `settings.json` as the only store (diff from defaults, hand-editable, already live). *Recommend yes.* The alternative, a separate keymap file, only helps if you want shareable keymap presets (D8).
2. **Conflict policy.** Block saving while a same-context conflict is unresolved, and offer Replace / Keep both with a context / Cancel. *Recommend yes.* (The alternative, VS Code's "allow and let `when` decide" with only a badge, is less safe for people who do not know what a context is.)
3. **Plain keys.** Refuse a bare printable key (no Ctrl/Alt/Meta) unless the person turns on "Advanced: allow plain keys", and then guard it with `!textInputFocus`. *Recommend yes* (WCAG 2.1.4 and the agent composer).
4. **Reserved keys.** Block the ones that cannot arrive, warn on the measured desktop ones (§5.3). *Recommend yes.*
5. **Browser panes (G3).** Push the effective key list to the CEF host so a rebound key works while a browser pane has focus. *Recommend yes, as the last phase*, and ship the rest without it if it is large (the view says so next to the key until then).
6. **Agents.** May an agent change bindings, with undo and a notice (§10)? *Recommend yes.*
7. **Where the editor lives.** A section of Settings (§4.1), with a link from Help. *Recommend yes;* the alternative, an editable Help pane, mixes reading and editing.
8. **Presets.** Do you want named keymaps (a "VS Code-like" or "Default" preset) later? *Recommend no for now;* the diff over one default table is enough, and a preset layer can sit on top of the same array later.
9. **Widget commands.** Keep them unbindable (as today)? *Recommend yes;* a key-bound widget command would join the table agents' `RunCommand` reads.
10. **Per-platform entries.** Always write `platform` from the view (§3)? *Recommend yes,* so a shared settings file is safe.

## 12. Plan, tests and checks

### Phases and PRs (about 9 working days, 6 PRs, each with a changeset)

| Phase | Content | Days |
|---|---|---|
| K0 | `bindingsView`, `edit.ts` (`planChange`, `conflictsFor`), `reserved.ts`, with their unit tests; `setUserKeybindings` returns indexed issues. No UI. | 2 |
| K1 | Reactive helpers (G2) and the call-site audit; Help shows current and default keys, "Not set" and the Customize link; `ListShortcuts` gains `defaults`, `changed`, `unbound`. | 1.5 |
| K2 | The Settings section: list, search, filters, rows, reset, changed badge, invalid entries (G4); Settings search index entries; keyboard and accessibility behaviour. | 2.5 |
| K3 | Recording, live conflicts with Replace / Keep both / Cancel, the `when` picker, plain-key guard, search by key, import/export. | 2 |
| K4 | Agent `shortcuts/set` and `/reset` routes with undo (only if D6 is yes). | 0.5 |
| K5 | Host key push (G3), if D5 is yes. | 1 (to confirm after reading `crates/cef`) |

macOS and Linux look checks run on K2 and K3 before merge, as the platform rule asks. K5 changes the host crate and gets a Windows and a macOS run of "rebind, focus a browser pane, press the key" by hand through the test hosts, with the operator's OK first (no lab without it).

### Tests (written with each phase, none needs a lab)

- **Registry and edit:** `planChange` produces the smallest entry list for add, replace, remove-one, remove-all, reset; entries round-trip through `setUserKeybindings`; a new default added to the table reaches a user who overrode a different key; order of unbinds and adds; `platform` handling on mac, Windows and Linux (the `linux` column, `other` entries on Linux).
- **Conflicts:** same key same context, disjoint contexts, chord shadowing both ways, pane row over global row, a candidate that equals the command's own existing key (not a conflict), reserved blocked and warned keys.
- **Per-platform defaults:** every default parses on its platform (exists), plus a snapshot of `bindingsView("mac"|"other"|"linux")` for a few commands.
- **Recorder:** key events to table strings (modifier order, `code:` for digits and punctuation, named keys, AltGr, IME composition, key repeat, bare-key refusal, chord timeout), driven by synthetic `KeyboardEvent`s.
- **View:** Solid tests in the style of the existing settings sections: render, search, record, conflict choices, reset, issues; keyboard navigation and the announced strings.
- **Reactivity:** change the setting and assert Help, the palette hint and a tooltip show the new key without remounting; the module-top-level-call check.
- **Host (K5):** unit tests of `hostKeys(effective)`; the IPC message shape.
- Existing: `keybindings.test.ts`, `host-keys.test.ts`, `no-hardcoded-hints.test.ts`, `doc.test.ts`, `app-api.test.ts` stay green.

### Sources (read 2026-11-06)

- VS Code: keybindings editor, `keybindings.json`, `when`, `-command`, chords: <https://code.visualstudio.com/docs/configure/keybindings>; source of that page: <https://raw.githubusercontent.com/microsoft/vscode-docs/main/docs/configure/keybindings.md>; keybinding dispatch and layouts: <https://github.com/microsoft/vscode/wiki/Keybinding-Issues>; Settings Sync per OS: <https://code.visualstudio.com/docs/configure/settings-sync>.
- JetBrains: <https://www.jetbrains.com/help/idea/configuring-keyboard-and-mouse-shortcuts.html>, <https://www.jetbrains.com/help/idea/settings-keymap.html.md>.
- Zed: <https://zed.dev/docs/key-bindings>; <https://raw.githubusercontent.com/zed-industries/zed/main/docs/src/key-bindings.md>.
- Slack: <https://slack.com/help/articles/201374536-Slack-keyboard-shortcuts>. Discord: <https://discord.com/blog/how-to-use-keyboard-shortcuts-on-discord-create-custom-keybinds>.
- macOS: <https://support.apple.com/guide/mac-help/mchlp2262/mac>, <https://support.apple.com/guide/mac-help/create-keyboard-shortcuts-for-apps-mchlp2271/mac>.
- Web platform: <https://developer.mozilla.org/en-US/docs/Web/API/KeyboardEvent/code>, <https://developer.mozilla.org/en-US/docs/Web/API/Keyboard/getLayoutMap>, <https://developer.mozilla.org/en-US/docs/Web/API/KeyboardEvent/isComposing>; WCAG 2.1.4: <https://www.w3.org/WAI/WCAG21/Understanding/character-key-shortcuts.html>; Keyboard Lock: <https://wicg.github.io/keyboard-lock/>.
- Not confirmed from primary documentation: the exact wording of JetBrains' conflict prompt, and Zed's conflict column covering only user bindings; neither is relied on here.
