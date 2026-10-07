# Report: four UI tweaks — Theme menu stays open, runtime panel text fits, tool previews show output only, paths keep their end

**Date:** 2026-10-07
**Author:** agent3 (Agent3@narko), at the operator's request
**Status:** active — built in this report's PR. Opacity keeps the menu open too, and the §2 wording is as proposed (operator, 2026-10-07). Two departures from the plan:
- §1: instead of only checking whether a choice remounts the open submenu, the menu no longer rebuilds its items at all.
- §2: the command palette's runtime commands keep the full explanation (`detail`); only the panel shows the short note. Read against `main` @ `f74243b24`.

The operator, 2026-10-07:

1. Choosing an item in **≡ → Theme** should leave the menu open, so themes can be tried one after another; the menu closes by clicking elsewhere.
2. In the agent pane's model / effort panel, the descriptive text overflows the panel. Shorten it and make it fit.
3. A tool call's expanded preview should show only the output. The command is already in the hover, so don't print it again.
4. In Read, Edit, Write and similar rows, a long path should show its end, with the ellipsis on the left: "Read …<end of path>", not "Read <beginning of path>…". It stays responsive as before. (Added later the same day.)

## 1. Theme menu stays open after a choice

**Where it closes.** The hamburger menu (`frontend/app/window/hamburger-menu.tsx`) is a `FlyoutMenu`. Every theme is a leaf item whose `onClick` writes `window:theme` (`hamburger-menu.tsx:44–52`). `FlyoutMenu.handleOnClick` (`frontend/app/element/flyoutmenu.tsx:192`) closes the whole menu first, then runs the click:

```ts
const handleOnClick = (e: MouseEvent, item: MenuItem) => {
    e.stopPropagation();
    if (item.subItems) return;
    onOpenChangeMenu(false);   // closes the menu on every leaf click
    item.onClick?.(e);
};
```

There's no way for an item to opt out. `MenuItem` (`frontend/types/custom.d.ts:632`) has no such field.

**Precedent.** The app's other menu, `PopoverMenu` (`frontend/app/element/popover-menu.tsx:20, :131`), already has `keepOpen?: boolean` and skips its close for such an item (#1586, used by the title-bar menu at `titlebar-context-menu.tsx:97`). The runtime panel also stays open by decision (`SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md` §9.2), and that spec names `FlyoutMenu`'s close-on-select as the behaviour it departs from.

**Fix.**
- Add `keepOpen?: boolean` to `MenuItem`. In `handleOnClick`, run `item.onClick` and return without `onOpenChangeMenu(false)` when it's set.
- Set `keepOpen: true` on the theme items.
- Outside click and `Esc` already close the menu through `FlyoutMenu`'s outside-click handler (`flyoutmenu.tsx:131`). That path doesn't change, so "closes by clicking elsewhere" holds as it is.

**Check before calling it done.**
- `menuItems` is a `createMemo` over `settingsAtom()`, so picking a theme rebuilds every item object to move the checkmark.
- If the submenu's rows are keyed by object identity, that rebuild remounts the Theme submenu and may drop its hover state, which closes the submenu one click later.
- This needs a real check in an isolated `task dev`. If it does close, keep the item objects stable and derive only `checked` reactively.

**Decided (operator, 2026-10-07).**
- **Opacity** (the next submenu) has the same "try a few" use, so it gets `keepOpen` too.
- **Layouts** items open dialogs, so they should keep closing the menu.

**Tests.** A `FlyoutMenu` test that a `keepOpen` leaf runs its `onClick` and leaves the menu mounted, while a plain leaf still closes it. A hamburger-menu test that the theme items carry `keepOpen`.

## 2. Runtime panel text fits the panel

**Where the text comes from.** The panel is `frontend/app/view/agent/components/AgentRuntimeDropup.tsx`. Each option row is the check icon, then `label`, then an optional `description` span (`:516`). There are three sources:

| Section | Source | Longest today |
|---|---|---|
| Mode | `permissionModeText()` notes (`frontend/app/view/agent/runtime-capabilities.ts:161`) | 110 characters: "writes are asked about and allowed automatically, so this does NOT stop edits; the plan is approved automatically" |
| Model | each provider's `models[].description` (`frontend/app/view/agent/providers/catalog.ts`) | 41 characters ("Anthropic's Sonnet, served by Antigravity"), next to a 28-character label ("Claude Sonnet 4.6 (Thinking)") |
| Effort | `EFFORT_OPTIONS` (`AgentRuntimeDropup.tsx:66`) | none have a description |

**Why it overflows.** `.menu.agent-runtime-dropup-panel` has `min-width: 180px` and no maximum (`frontend/app/view/agent/styles/_composer-strip.scss:247`). `.agent-runtime-dropup-description` is `white-space: nowrap` with an ellipsis (`:315`). The row never constrains the description, so the ellipsis never applies: the panel grows to the longest single line and runs past the pane.

**Fix, in two parts.**

1. **Shorter text.** These keep the meaning the long notes explain (see the comment above `permissionModeText`):

   | Mode, case | Today | Proposed |
   |---|---|---|
   | Default, no one to ask | unapproved writes and commands are refused (there is no one to ask) | refuses unapproved writes and commands |
   | Accept Edits, no one to ask | edits are allowed; other unapproved commands are refused | edits allowed; other commands refused |
   | Default, prompts auto-answered | writes and commands are asked about, and every ask is allowed automatically — same as Bypass for now | every ask auto-approved (like Bypass) |
   | Accept Edits, auto-answered | edits are not asked about; any other ask is allowed automatically — same as Bypass for now | other asks auto-approved (like Bypass) |
   | Auto, auto-answered | the CLI's classifier decides what to ask; every ask is allowed automatically | classifier asks; all auto-approved |
   | Plan, auto-answered | writes are asked about and allowed automatically, so this does NOT stop edits; the plan is approved automatically | does not block edits; plan auto-approved |

   Model descriptions over 30 characters:
   - "Anthropic's Sonnet/Opus, served by Antigravity" becomes "Anthropic, via Antigravity".
   - "Fast frontier model, balanced/more reasoning" becomes "Fast, balanced" / "Fast, more reasoning".
   - "Most capable for long-horizon work" becomes "Long-horizon work".
   - "Deep reasoning and complex coding" becomes "Deepest reasoning".

   The long explanation stays available as the row's tooltip, so nothing is lost. A `title` on the row is enough; this panel doesn't use the `Tooltip` component.

2. **Layout that can't overflow.**
   - Give the panel a `max-width` (proposed `min(300px, calc(100vw - 16px))`).
   - Put the description on its own line under the label: smaller, secondary colour, `white-space: normal`, wrapping. This needs a row column wrapper.
   - The check icon stays aligned with the label line.

   The section notes (`.agent-runtime-dropup-note`: "Running …", "Not applied — …") and the drift box get the same wrapping, since they're built from runtime text such as a full model id.

**Tests.** `AgentRuntimeDropup.test.tsx` asserts the persistent-agent notes (around `:458–471`). Update those assertions, and add a test that each mode note is at most 40 characters, so a later long note fails CI instead of the layout. Check the panel visually in an isolated `task dev` at a narrow pane width.

## 3. Tool previews show output, not the command

**Where the command is printed.** Only Bash prints its input in the expanded panel; the other tools' panels show results. It's printed at three points:

| State | Where | Code |
|---|---|---|
| Output streaming | `ChunkList`, above the log lines | `frontend/app/view/agent/components/ToolOverlayLog.tsx:505` (`bashCommand()`, `:125`) |
| Waiting for the first output | `ToolOverlayResult`'s "Thinking…" fallback | `ToolOverlayLog.tsx:556` |
| Finished | `BashOutputViewer`, above stdout / stderr | `frontend/app/view/agent/components/BashOutputViewer.tsx` (`<BashCommandView command={props.params.command} />`) |

**The command is already shown twice elsewhere:**
- the collapsed row, truncated (`ToolBlock.tsx:467`, `ShellTokens`);
- the hover peek, full and word-wrapped, in every state (`ToolBlock.tsx`'s `PeekOverlay`, `SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md` §1, #3780).

**History.** The streaming and waiting copies were added with tokenized command highlighting (#4314), "so it is visible from the first frame". The hover now covers that from the first frame too, so this reverses a deliberate addition. That's intended, and it should be cited in the change.

**Fix.**
- Remove the three `BashCommandView` renders and the `command` props that feed them (`bashCommand()` in `ToolOverlayLog`; the `params.command` read in `BashOutputViewer`).
- `BashCommandView` itself stays: the hover still uses it, through `ShellTokens`.
- A finished call with no stdout or stderr would otherwise show a nearly empty panel. Add a small "No output" line above the exit code, so the panel never looks broken.
- `BashOutputViewer`'s header comment ("Displays bash command and output") becomes "output and exit code".

**Tests.**
- `ToolBlock.test.tsx` and `ToolOverlayLog.test.tsx` reference the command in the panel; switch those to assert its absence, and that the hover still has it.
- Add a test for the "No output" line.

## 4. Paths keep their end in the tool row

**Where it's cut.** The tool row's name run (`ToolBlock.tsx`, `.agent-tool-name`) is a single text run: icon, label, line range (for a Read), then the detail. It has `text-overflow: ellipsis` (`frontend/app/view/agent/styles/_document-nodes.scss`), so a long path is cut on the right and the file name is the part that disappears: "📖 Read /c/Users/me/projects/agentmux/front…".

**Which tools.** The descriptors whose detail is a file path (`pathOf` in `frontend/app/view/agent/tool-meta/tool-descriptors.ts`):
- Read (`read`, `read_file`)
- Write (`write`, `write_file`)
- Edit (`edit`, `str_replace_editor`, `multiedit`)

Bash commands, Grep and Glob patterns, and web queries are not paths and keep the right-side cut.

**Fix.**
- **Descriptor:** a new fact, `detailIsPath`, is true for those three descriptors and false in the catch-all. `toolHeaderParts` passes it on, and only when there is a detail.
- **Row layout:** for such a row, `ToolBlock` adds `agent-tool-name--path` and wraps the path in `<span class="agent-tool-detail agent-tool-detail--path"><bdi>…</bdi></span>`. The name run becomes a flex row: icon, label and range keep their width, and only the path shrinks.
- **The ellipsis:** the path span is `direction: rtl` with `text-overflow: ellipsis`, which puts the ellipsis on the left. The `<bdi>` isolates the path, so it still reads left to right. A flex row drops the spaces between its items, so `column-gap` stands in for them, and the row's text is unchanged.
- **Width:** it adapts to the row's width, as before: a path that fits shows in full, left-aligned.

**Tests.**
- `tool-header.test.ts`: which tools mark a path; no path means no path layout.
- `ToolBlock.test.tsx`: a Read row has the path layout with the full path in its `<bdi>`, and a Grep row doesn't.
- `tool-descriptors.parity.test.ts` accepts the new field.
- jsdom can't measure an ellipsis, so the left-side cut itself is checked in an isolated `task dev`.

## 5. Delivery

- **One agentmux PR**, frontend only, with a changeset and the tests above. The four tweaks are small and independent; commit each separately so a review note on one doesn't hold the others.
- **Visual check** in an isolated `task dev`, which this report needs for the §1 submenu question and the §2 layout. Include before/after notes in the PR description.
- **Docs:** `agentmux-docs` `main-menu.md`, whose Theme row (`:19`, `:30`) should say the menu stays open while you pick, in a follow-up docs PR after the app PR merges. No docs page describes the tool panel showing the command.
- **Decisions:** taken, see §1.
