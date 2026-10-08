# Remotes becomes a section of Connectors; the model/effort panel keeps its size

**Status:** implemented — both parts (§3 Remotes in Connectors, §6 the model/effort panel) in PR #4491.
**Date:** 2026-10-08.
**Requested by:** repo owner (asafebgi): "i think we should get rid of the remotes pane and add it as a section to
Connectors"; "you can still see multiple remotes at the same time by opening multiple connector panes"; and, for §6,
"in the agent pane's model/effort panel, if I change the option, we see strange panel resize behavior, we want any
reactive updates to have the proper space without resizing the container panel".
**Author:** Masty.
**Related:** `SPEC_REMOTES_PANE_2026_10_05.md` (the Remotes pane; its §4.2 onward, the list itself, is unchanged),
`SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md` (Connectors, and the pattern for retiring a pane),
`SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md` (the model/effort panel).

---

## 1. Goal

Connectors is "what agents connect to outside AgentMux": Accounts and MCP servers. Remote machines are the third
thing of that kind. Remotes stops being a pane type of its own and becomes the **Remotes** section of Connectors,
beside Accounts and MCP servers. Nothing in the list changes: the same rows, actions, detail panel and settings.

Several remotes lists at once still work: open several Connectors panes, each on Remotes. The list is srv's
(`RemotesList`), so every copy shows the same one, as before.

## 2. What exists (`main` at `7a5ce8919`)

| Piece | Where |
|---|---|
| The Remotes pane manifest, `view: "remotes"` | `frontend/app/view/remotes/remotes.tsx`, listed in `block/block-registry.ts` |
| The list: model (`RemotesViewModel`) and view (`RemotesView`) | `view/remotes/remotes-model.ts`, `remotes-view.tsx` |
| Links into it from another pane: open Remotes as a tab in that pane, or switch to the one there, with a host's row expanded (`remotes:expand`) | `view/remotes/open-remotes.ts` (`openRemotesInPane`, `remoteSettingsMenuItems`) |
| Who calls them: the connection picker's "Manage remotes…", Hangar's Remote heading, the pane menu's "Remote settings…" | `modals/conntypeahead.tsx`, `view/files/files-view.tsx`, `block/blockframe.tsx` |
| Command palette "Open Remotes" | `store/command-registry.ts` |
| The launcher widget `defwidget@remotes` (in More) | `crates/srv/src/config/widgets.json` |
| Connectors: a section pane, one meta key `connectors:section` | `view/connectors/connectors.tsx`, `view/section-pane/` |
| Layout files: `remotes` is not a known type, so a saved Remotes pane opens as an empty pane | `crates/srv/src/backend/layout_file.rs` |

A section in a section pane is a bare `component: () => JSX.Element`: it gets nothing from the pane. Accounts and MCP
don't need anything; the Remotes list needs the pane's block id (New terminal splits beside it) and its meta (the
`remotes:expand` request).

## 3. Design

### 3.1 The section

- `ConnectorsSection` gains `"remotes"`; the sections are Accounts, MCP servers, Remotes, in that order.
- The section: label "Remotes", icon `server`, tooltip "Remote machines (SSH, WSL)". The pane's title reads
  "Connectors · Remotes", as for the other sections.
- A section's `component` receives the pane's `SectionPaneModel`, which now also exposes the block's `meta`. The
  Remotes section builds its `RemotesViewModel` from the pane's block id, meta and `setMeta`, and disposes it on
  cleanup. The other sections ignore the argument.
- Like every section, it stays mounted while the pane is open, so switching to it is instant.

### 3.2 Links from other panes

The links keep what they do today, aimed at Connectors instead: in the pane holding the caller, switch to its
Connectors tab if it has one, or add one beside it, set to the Remotes section, with the named host's row expanded.
Opening Remotes never jumps to another pane.

### 3.3 Opening it directly

- Command palette "Open Remotes" stays (people look for it by that name) and opens a new Connectors pane on Remotes.
- The `defwidget@remotes` launcher widget is removed: Connectors is already a widget, one click from Remotes. A
  pinned list that names `remotes` reads it as `connectors` (`RENAMED_PINS`, as for `armory`), and a
  `defwidget@remotes` entry in the user's own `widgets.json` is moved to `defwidget@connectors` by srv
  (`RENAMED_WIDGETS`, as for `knowledge`), so it leaves no stray Remotes widget. Its `view: "remotes"` is
  rewritten to `connectors` on the Remotes section: a click then focuses the open Connectors pane instead of
  making another, and the Connectors tab takes the entry's label and icon.

### 3.4 Saved panes and layout files

- A block saved as `view: "remotes"` still loads, through a legacy manifest (the Armory's pattern, `legacyOf:
  "connectors"`): it rewrites the block to `view: "connectors"`, `connectors:section: "remotes"`, keeping
  `remotes:expand`, and shows Connectors from then on.
- `layout_file.rs`: a block still saying `remotes` is written as `connectors` with section `remotes`; a file that
  says `remotes` opens as Connectors on Remotes. Both with tests.

### 3.5 Words

- Settings → Terminal: "The Remotes pane can set it per host" and "Each host can override this in Remotes" name the
  section ("Connectors → Remotes").
- `SPEC_REMOTES_PANE_2026_10_05.md`: a status line pointing here; its §4.1 (the pane) is superseded by §3 above.

## 4. Steps

1. Section pane: pass the model to `component`; add `meta` to `SectionPaneModel`.
2. `panes.ts`: the section id; `connectorsRemotesMeta(connection?)`.
3. `connectors.tsx`: the Remotes section.
4. `remotes.tsx` → the legacy manifest; `open-remotes.ts` → Connectors; the command.
5. `widgets.json`, `RENAMED_PINS`, `layout_file.rs`.
6. Words and docs; tests updated and added.
7. §6, the model/effort panel.

## 5. Tests

- Connectors lists Remotes; selecting it shows the list; the expand request reaches it and is cleared.
- `openRemotesInPane`: adds a Connectors tab on Remotes; reuses one already in the pane, switching its section.
- A `remotes` block migrates to `connectors` / `remotes` and keeps `remotes:expand`.
- Pins: `remotes` → `connectors`. Widgets: no `remotes` in More.
- `layout_file.rs`: export and import of `remotes`.

## 6. The model/effort panel keeps its size

### 6.1 What is wrong

The agent pane's runtime panel (`AgentRuntimeDropup.tsx`) stays open across selections, so a user changes Mode,
Model and Effort in one visit. Every change rebuilds the panel from what is selected and what is running, and several
parts of it come and go or change length. The panel is sized by its content, so it grows and shrinks as the user
clicks. It opens upward from the trigger (`top-start`), so a change of height moves every row under the pointer, and
if it no longer fits above, `flip()` moves it below the trigger.

| # | What changes | When |
|---|---|---|
| C1 | The Effort section is five rows, or one "Not applied — …" note | Picking Haiku, or leaving it |
| C2 | The running-model note under Model appears, disappears, and changes length (it carries the full model id, and "(not opus)") | After the first reply, after a restart, after a model change |
| C3 | The drift banner at the top appears ("Applies after the current turn", 1–3 detail lines, "Restart to apply") and goes away | Right after a change while a turn runs; 1.5 s after a mismatch; when the restart lands |
| C4 | The width is anything from 180 to 300 px, by the longest row or note | With C2 and C3 |
| C5 | The trigger, the panel's anchor, changes width ("Bypass · Opus · high" → "Bypass · Haiku"); the composer strip measures its slots and can move the trigger or move it to another row; the panel follows its anchor | Any change of Mode, Model or Effort |

### 6.2 The fix: every part has its place from the first frame

The panel's size depends only on things that don't change while it is open (the provider's model list and mode
texts), never on what is selected or running:

- **Width.** One fixed width, `min(300px, calc(100vw - 16px))` (the old cap), instead of a range. Longer text wraps.
- **Effort (C1).** Always the five rows. When effort doesn't apply they are shown dimmed and can't be selected (not
  by click, not by the keyboard), and the reason goes on the section's header line, on one line, with the full text
  as its tooltip.
- **Running model (C2).** A line under the Model rows, always there, two lines high (longer text is clamped, the full
  text in its tooltip). Before there is anything to say, it says so in muted text ("Shown after the agent's first
  reply").
- **Status (C3).** The drift banner becomes a status line at the top, always there, two lines high: "Changes apply on
  the next turn." when the agent runs what is selected; "Applies after the current turn." with what is waiting; "The
  agent is not running what is selected." with what differs. The details are one line (clamped, full text in the
  tooltip). "Restart to apply" keeps its place: it is only visible, and only focusable, when the agent differs.
- **Anchor (C5).** While the panel is open the trigger keeps the width it had when the panel opened; its label is
  ellipsized if it got longer. The strip's measurement sees no change, so the trigger doesn't move. The width is let
  go when the panel closes.

With the size fixed, nothing moves under the pointer and the panel doesn't flip while it is open. A change in the
provider's model list while it is open (the live catalog arriving) can still change its height; that is rare and
real.

### 6.3 Tests

- Effort keeps five rows when the model becomes Haiku; they are disabled and not reachable by the keyboard.
- The status line and the running-model line are rendered in every state.
- "Restart to apply" is hidden and not focusable unless the agent differs.
- The trigger's width is held while open and released on close.
- In the running app (CDP): the panel's bounding box is the same before and after picking another Mode, Model
  (including Haiku) and Effort.
