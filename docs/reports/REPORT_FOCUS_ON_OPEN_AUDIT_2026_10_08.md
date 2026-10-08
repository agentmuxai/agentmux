# REPORT: Inputs that should take the caret when they open

**Date:** 2026-10-08
**Author:** AgentO
**Trigger:** Repo owner, 2026-10-08: "if a user selects 'Broadcast' in the swarm, the input box should select right after
the button click … scan the codebase for other places where a user expects to be able to type as soon as it opens, like
settings … find them all, write report to file."
**Status:** active — every row of §2 and the Settings gap in §3 are fixed in #4481 (§7); the rest of §3 and §5 is open.
**Related:** `SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md` (#4479: the caret follows the selected pane after a close; adds
`data-pane-focus` and the retry this report refers to), `SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md` (`claimFocusOnMount`).

Line numbers are against `agento/focus-follows-selection` (main `bf00c9103` plus #4479; only #4479's own files differ
from main). Rows marked *unverified* were reasoned from the code, not run.

---

## 1. Verdict

**The `autofocus` attribute does nothing in this app.** Solid only sets the DOM property and never calls `focus()`.
Chromium (CEF) honours `autofocus` on an inserted element only until something on the page has been focused, so in
practice only at startup. Nothing in the codebase polyfills it. Every input that relies on the attribute alone does not
take the caret when it opens; inside `<Modal>` the attribute is dead code, because Modal focuses its first focusable
element itself (`modal.tsx:292-303`).

The rest of the gaps share one shape: a button is replaced by the input it opens (a `<Show>` swap, a list replaced by a
form), the button had the caret, it is removed, and the caret falls to `<body>`.

There is no shared helper for "focus this input when it appears"; the surfaces that work each do it their own way
(§5). #4481 adds one, `focusOnOpen` in `frontend/util/focusutil.ts`, and fixes every row of §2 with it (§7).

## 2. Did not take the caret (most-hit first; all fixed in #4481, §7)

| # | Surface | Opened by | Input | Why not | Select text? | Fix |
|---|---|---|---|---|---|---|
| 1 | **Terminal Find** | `pane:find` (Cmd+F / Ctrl+F), `keymodel.ts:214-218` sets `searchAtoms.isOpen` | `element/search.tsx:167-173` (`<Input autoFocus>`, portaled to `<body>`) | `autoFocus` only becomes the attribute (`input.tsx:120`). `termViewModel.giveFocus()` (`:376-380`) returns true while search is open without focusing it. The caret stays in xterm: **typed text goes to the shell.** | yes, if a previous query is kept | `Input` honours `autoFocus` through `focusOnOpen` (§6), with select |
| 2 | **Agent "Deny + feedback"** | Deny button `AgentDecisionPanel.tsx:419-423` or Shift+Enter `:236-238`; `<Show when={denyMode()}>` `:357` | textarea `:360` | `autofocus` only (`:371`). The clicked button is removed; caret to `<body>`. "Back" does the same. | no | `focusOnOpen` on the textarea. Not on the panel itself: the agent opens that |
| 3 | **Swarm Broadcast composer** | "Broadcast" `swarm-fleet-toolbar.tsx:150-157`; the input is the `fallback` of `<Show when={!broadcastOpen()}>` `:120-158` | `.swarm-fleet-broadcast-input` `:124-135` | `autofocus` only; the button is removed by the swap | no | **Fixed in #4481** (`focusOnOpen`) |
| 4 | **Connection switcher** ("Connect to …") | conn button `blockutil.tsx:77-80`, or Cmd/Ctrl+Shift+G (`keymodel.ts:158-163` → `blockframe.tsx:1202-1204`) | `TypeAheadModal` → `Input` `typeaheadmodal.tsx:218-222`; `autoFocus={isNodeFocused()}` `conntypeahead.tsx:421` | attribute only; `TypeAheadModal`'s `giveFocusRef` prop (`:86, :178-183`) has no caller. Typing goes to the shell | no | `TypeAheadModal` focuses its input on mount when `autoFocus` (also fixes #9) |
| 5 | **Agent Memory modal "+ New file"** | `AgentNativeMemoryModal.tsx:213-219` → `setShowNewInput(true)`; `<Show>` `:191` | `:193` | `autofocus` only (`:199`); the button is hidden (`visibility:hidden`) | no | `focusOnOpen` |
| 6 | **Memory "Edit"** (Global Memory `GlobalMemoryFullView.tsx:111`, Personal Memory file `NativeMemoryFileView.tsx:144`, the agent memory modal) | Edit button, swapped out | `MemoryContent.tsx:69` | no focus call | no | opt-in prop on `MemoryContent` that focuses on entering edit mode |
| 7 | **Global Memory "New memory"** | tile `global-bundle-manager.tsx:197-202` | Name `GlobalMemoryFullView.tsx:67` | no focus call | no | `focusOnOpen` on Name |
| 8 | **Bundle / Skill / MCP "New" and "Edit"** | `bundle-manager.tsx:156/:231`, `skill-manager.tsx:57/:131`, `mcp-manager.tsx:107/:170` | Name: `bundle-manager.tsx:275`, `skill-manager.tsx:147`, `mcp-manager.tsx:186` | no focus call; `PrimitiveListDetail` (`primitive-list-detail.tsx:30-45`) unmounts the list and its button | Edit: optional | `PrimitiveListDetail` focuses the detail's first text field when `showDetail` turns true: one fix for all three |
| 9 | **Editor "Open from remote…"** | `editor-view.tsx:706`, menu `editor-model.ts:1370` | `open-from-remote-modal.tsx:97, :110` (`TypeAheadModal autoFocus`) | as #4; in step 2 the step-1 input is removed after Enter | no | as #4 |
| 10 | **Accounts "Add account" / "Edit"** | `identity-accounts-tab.tsx:101`, `accounts/AccountsGallery.tsx:51`, Edit `identity-accounts-tab.tsx:367, :372, :376`; overlay `accounts/accounts-manager.tsx:141-142` | Name `identity-account-form.tsx:234` | a plain overlay, not `<Modal>`; no focus call | Edit: yes | `focusOnOpen` on Name (the key field `:352` for a key-preset tile) |
| 11 | **Remotes "Add remote"** | `remotes-view.tsx:66`; `<Show when={adding()}>` `:70` | Alias, `remotes-view.tsx:439` | no focus call | no | `focusOnOpen` |
| 12 | **Agent shell drawer**, opened by the user | Log toggle `agent-view.tsx:1408`; drawer `:1486` | xterm in `AgentShellSubblock` | no focus code in the drawer | n/a | focus the drawer terminal on the user's toggle only, never on the `!cmd` auto-open (`agent-view.tsx:939`), which the agent drives |
| 13 | **Personal Memory agent filter** | Memory pane opened or selected | `MemoryAgentFilterBar.tsx:56` | no `data-pane-focus`; `SectionPaneModel` has no `giveFocus` | no | `data-pane-focus` (hidden sections are `display:none`, so it is picked only when showing) |
| 14 | **Drone "New" name; Variables "+ Add"** | `drone-view.tsx:85`; `:853-855` | name `:55` ("Untitled Drone"); new row `:826` | no focus call | new name: yes | `focusOnOpen` (select for the name) |
| 15 | **Bundle "Add provider override"** | `BundleProviderInstructionsSection.tsx:184` | new row `:121` | the Add button keeps the caret | no | `focusOnOpen` on the new row's provider field |
| 16 | **Pane title editor** (only with the pane-label setting on; off by default, `titlebar.tsx:29`) | click on the title `titlebar.tsx:86`, or the pencil `:113` | `titlebar.tsx:93-103` | `autofocus` only (`:101`); the click also reaches the block's click handler, which focuses the pane's own input | **yes** | `focusOnOpen(el, { select: true })` |

## 3. Partly working or fragile

- **Opening Settings** (Cmd/Ctrl+, `keymodel.ts:61`; the hamburger menu `hamburger-menu.tsx:141`; both
  `openOrFocusPaneByView("settings")`, `block-component-registry.ts:192-215`). Settings is always a pane. Its search
  (`settings-search-bar.tsx:80-93`) is marked `data-pane-focus` by #4479.
  - Already open in this window tab: the search takes the caret at once.
  - Created fresh: the view isn't mounted when the insert runs. #4479's retry (`focusManager.ts:186-199`) keeps trying
    only while the caret is "parked" (on `<body>` or the new block's dummy input). Opened with Cmd+, from a terminal or
    an agent composer, the caret is still in that other pane's input, so the retry may stop on its first frame and the
    search never gets it. *Unverified live*; most likely it works through the selection observer, but the path is
    fragile.
  - Fix: when the selection changed because the user opened a pane, treat a caret in *another* pane as parked.
- **My Agents picker.** The first card claims focus on mount (`AgentCard.tsx:87-98`), so it wins over the filter
  (`AgentPickerFilterBar.tsx:55`); #4479's type-to-filter routes the first printable key into the filter, so typing
  works. On a later re-select the filter wins. Pick one target.
- **Agent launch modal** (`AgentLaunchModal.tsx:571`). In New mode the name field gets the caret. When past instances
  exist, the modal switches to Continue after an RPC (`useContinueOrNewMode.ts`, about `:170-190`) and the name field
  becomes `disabled` (`:578`): the caret is lost and Enter-to-submit (`:490`) stops working. Fix: focus the Launch button
  (`data-modal-initial-focus`) when Continue activates.
- **Agent transcript search** (`AgentSearchBar.tsx:53-58`). Focuses through `setTimeout(0)`, without `preventScroll` or
  `select()`. Opens on Ctrl+F only (`useAgentKeyboard.ts:34`): Cmd+F on macOS is `pane:find`, which does nothing in an
  agent pane (no `searchAtoms`). × leaves the caret on `<body>`; a second Ctrl+F closes the bar instead of refocusing it.
- **My Agents → Rename / Duplicate** (`MyAgentsList.tsx:260`, opened at `:1178`, `:598`). Focuses (`:271-275`) but
  doesn't select the prefilled name. It is portaled to `<body>`, so after Save or Esc the caret stays on `<body>`. Fix:
  select, and return the caret on close.
- **Window-tab rename** (double-click `tab.tsx:327`, menu `:104`, F2 `:190`). Sets `contentEditable` and a selection
  range (`:170-186`) with no explicit `focus()`; Chromium focuses the editing host implicitly. *Unverified live.* Fix:
  `focus({ preventScroll: true })` before selecting.
- **Pane header name editor** (`ViewNameEditor`, `blockframe.tsx:245-269`). `setTimeout(0)` focus and select; on a pane
  that wasn't selected it races the selection observer's next-frame focus. `focusOnOpen`'s microtask would make it
  deterministic.
- **Files "New file / New folder"** (`files-model.ts:755-772`). Focuses when the RPC returns, without checking the pane
  is still selected, so it can take the caret from wherever the user went. Route it through `claimFocusOnMount`.
- **In-app login code field** (`InAppLoginPanel.tsx:45-50, :163`). A next-frame focus whenever `authUrl` appears, with no
  `preventScroll`, no selected-pane check and no window-focus check: inside a running agent pane (`AgentDocumentView.tsx
  :260-280`) it can take the caret from another pane. Guard it like `claimFocusOnMount`.
- **New browser pane** (`use-pane-rect-sync.ts:218` → `browser-model.ts:747-761`). The page gets the caret, not the
  address bar (`browser-nav-bar.tsx:434`). Right for panes opened with a URL or by an agent; for an empty pane the user
  created, focus and select the address bar.
- **Launcher.** Focuses (`launcher.tsx:100-106`), but its hidden search updates on `onChange` (`:269`), which in Solid
  is the native `change` event, so filtering as you type probably never fires. *Unverified;* a separate bug.
- **Selection without focus.** A Settings search result scrolls to its row (`settings-view.tsx:76-89`) but doesn't focus
  the control. Bundle import preview (`BundleImportPreviewModal.tsx:112`) focuses its name without selecting it. The file
  tree's rename (`file-tree.tsx:310-316`) selects the whole name including the extension; the Files view selects only
  the stem.

## 4. Already works

Command palette (`command-palette.tsx:57`); `UserInputModal` (Modal's first focusable; opened by the backend, by
design); pane-tab rename (`PaneTabRenameInput.tsx:35-37`); window rename in the instance panel (`InstancePanel.tsx:535
-541`); agent composer on pane open (`AgentFooter.tsx:928-941`, `claimFocusOnMount`) and after launch
(`composer-focus.ts:120`); agent modal-layer panels (add account `AgentNewIdentityModal.tsx:100`, new memory
`AgentNewBundleModal.tsx:101`, create from template `AgentCreateFromTemplateModal.tsx:375`, Claude login, browser auth
`BrowserAuthModal.tsx:75`); editor text, Find, Save As (`editor-tab-strip.tsx:105-115`), file-tree New and Rename
(`file-tree.tsx:307-316`); Files filter (`files-view.tsx:149-155`), path edit (`:750-756`), rename (`:1425-1428`);
launcher focus; browser address bar on Cmd/Ctrl+L (`browser-nav-bar.tsx:138-155`); SSH approval secret
(`SshApprovalWindow.tsx:80-81`); the Remotes filter and Settings search when already open (`data-pane-focus`, #4479).

Not present, so nothing to fix: a Swarm fleet search toggle, Toolchain or Drone search boxes, workspace rename. Toolchain,
Warden, Armory, Connectors, Help, Sysinfo and Media have no text field that opens on demand.

## 5. Related problems found

**Focus calls that can scroll** (no `preventScroll`): `modal.tsx:302`, `ModalLayer.tsx:147` (harmless today, it resets
`scrollTop` first), `AgentSearchBar.tsx:57`, `AgentFooter.tsx:741, :957, :1300`, `useAgentDropAttach.ts:60`,
`InAppLoginPanel.tsx:45-50`, `blockframe.tsx:268`, `PaneTabRenameInput.tsx:36`, `command-palette.tsx:57`,
`InstancePanel.tsx:538`. The composer ones matter most: a focus scroll there has shifted whole window tabs before.

**Must never take the caret on open**, because an agent or the backend opens them: the decision and question panels
(`AgentQuestionPanel.tsx:539`), the shell drawer's `!cmd` auto-open, the in-app login panel, Files create-new after its
RPC, panes opened by `OpenEditor`, `OpenAgent` or the browser, `UserInputModal`. Rule: focus directly only what the
user's own click or key just opened; anything else goes through a `claimFocusOnMount`-style guard (window has focus,
the pane is the active tab's selection, no modal, the user's caret isn't in a text field).

**How the working surfaces focus today** (five different ways): `onMount` (`PaneTabRenameInput.tsx:35`,
`editor-tab-strip.tsx:110`, `files-view.tsx:1425`); `queueMicrotask` (`command-palette.tsx:57`, `InstancePanel.tsx:537`,
`files-view.tsx:149, :750`); `setTimeout` in a ref (`AgentSearchBar.tsx:57`, `blockframe.tsx:268`); `createEffect`
(`MyAgentsList.tsx:271`, `file-tree.tsx:310`); a next-frame effect (`InAppLoginPanel.tsx:45`). Also a duplicate
`isTextEntry` (`composer-focus.ts:62` and `util/focusutil.ts`).

## 6. Recommendation (one helper, not sixteen fixes)

1. **`focusOnOpen(el, { select })`** in `frontend/util/focusutil.ts` (added in #4481): a microtask, then
   `focus({ preventScroll: true })` and an optional select, if the element is in the document. The microtask runs after
   the render that inserted the element and before #4479's next-frame selection focus, which then sees the caret in the
   pane and leaves it. Use as `ref={(el) => focusOnOpen(el)}`. Add a `"stem"` select mode when the file renames move to
   it.
2. **Make the wrappers honour their prop through it:** `Input` (`element/input.tsx:62, :120`) and `TextInput`
   (`element/ui/inputs.tsx:35`). That fixes Terminal Find, the connection switcher, Open from remote and every
   `TextInput autofocus` call site without touching them.
3. **Replace the remaining raw `autofocus` attributes** (`AgentDecisionPanel.tsx:371`, `titlebar.tsx:101`,
   `AgentNativeMemoryModal.tsx:199`, `userinputmodal.tsx:115`, `BrowserAuthModal.tsx:79`), then enable the lint rule
   `jsx-a11y/no-autofocus` (already referenced and disabled at `AgentLaunchModal.tsx:584`) so the dead attribute can't
   come back.
4. **List-to-form swaps:** `PrimitiveListDetail` (and similar) focuses the first text field of the detail when it
   opens: one change for Bundle, Skill and MCP.
5. **Move the ad-hoc focus code in §5 onto the helper** as those files are touched, which also fixes the missing
   `preventScroll`s.

## 7. Fixed in #4481

One helper pair in `frontend/util/focusutil.ts`, used everywhere below:

- `focusOnOpen(el, { select })`: on the next microtask, if the element is in the document, `focus({ preventScroll: true
  })` and optionally select its text. Skipped when the user's caret is in a text field in *another* pane, so a form that
  remounts in a background pane doesn't take it; an input outside every pane (Terminal Find is portaled to `<body>`) is
  exempt, since it opens over the pane whose caret it takes.
- `focusWhenRendered(find, opts)`: the same for an element the user's action renders but that has no mount of its own to
  hook (a row appended by "+ Add", a field that stays mounted when "New" resets it).

| # | Surface | Fix |
|---|---|---|
| 1 | Terminal Find | `Input` honours `autoFocus` through `focusOnOpen` (with `autoSelect`, the previous query is selected); Find passes `autoSelect` |
| 2 | Deny + feedback | `focusOnOpen` on the textarea |
| 3 | Swarm Broadcast | `focusOnOpen` on the field |
| 4, 9 | Connection switcher, Open from remote | through `Input` (`TypeAheadModal` passes `autoFocus`) |
| 5 | Agent Memory "+ New file" | `focusOnOpen` |
| 6 | Memory Edit (3 places) | `MemoryContent`'s new `autoFocus` prop, passed by the three edit views |
| 7 | Global Memory "New memory" | `focusOnOpen` on Name when new (the content editor doesn't take it there) |
| 8 | Bundle / Skill / MCP New and Edit | `focusOnOpen(…, { select: true })` on Name, which mounts with the form |
| 10 | Accounts Add / Edit | `focusOnOpen(…, { select: true })` on Name |
| 11 | Remotes Add remote | `focusOnOpen` on the first field |
| 12 | Agent shell drawer | `useShellLogBridge`'s `withShellFocus` wraps the user's toggle; the shell takes the caret when its terminal is ready. The `!cmd` auto-open doesn't go through it |
| 13 | Personal Memory agent filter | `data-pane-focus` |
| 14 | Drone New, Variables "+ Add" | `focusWhenRendered` (name selected on New; the new row's name on Add) |
| 15 | Bundle "Add provider override" | `focusWhenRendered` on the new row |
| 16 | Pane title editor | `focusOnOpen(…, { select: true })` |
| — | Settings: picking a section with the mouse | the search takes the caret back, its text selected, so the next keystrokes search (owner request, 2026-10-08). Arrow keys in the tab list keep the caret on the tabs |
| §3 | Settings opened from a terminal or composer | `focusManager`'s retry treats a caret still in the pane the selection just left as parked, so it keeps trying until the Settings search mounts |

`TextInput` (`element/ui/inputs.tsx`) also honours `autofocus` through `focusOnOpen` now, so its call sites inside modals
no longer rely on the modal's first-focusable rule alone.

Checked live on a dev build: Cmd+F in a terminal, Cmd+Shift+G, and Cmd+, from a terminal each put the caret in the field
that opened. The lint rule from §6 item 3 and the rest of §3 and §5 are not done.

