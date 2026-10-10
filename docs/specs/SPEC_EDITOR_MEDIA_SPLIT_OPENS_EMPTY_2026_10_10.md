# Spec: Splitting an editor or media pane opens an empty one

**Status:** implemented — PR #4602
**Date:** 2026-10-10
**Author:** Clamk

## Ask

In an editor or media pane, a split should open a new pane of the same kind with **no documents open**. Today the new pane opens with the same document tabs as the one it was split from.

## Today

`handleSplitPane` (`frontend/app/block/pane-actions.ts`) gives any view without a `splitBlockDef` a copy of the source pane's whole meta (`copiedSplitBlockDef`). Both views keep their open documents in that meta:

| View | Keys that hold documents |
|---|---|
| Editor | `doctabs` (the tab list, `DOC_TABS_META`), `file` (the pre-tabs single file), `editor:line` (open `file` at a line), `editor:pending_open_files` (opens queued before mount), `editor:scratch` (open a scratch buffer when nothing else is open) |
| Media | `doctabs`, `media:path` (the file in front), `media:open` (opens queued from elsewhere) |

So the copy hydrates the same tabs. The split is a second view of the same documents, not a new pane.

The other two split paths, the command palette and the split shortcuts (`command-registry.ts`, `keymodel-blockcreate.ts`), don't copy: a view without `splitBlockDef` gets the configured new block there, a terminal by default. So "split" already means different things for these panes depending on how it was asked for.

## Change

Follow `SPEC_AGENT_PANE_SPLIT_OPENS_PICKER_2026_09_30.md`, which replaced a deny-list of meta keys (`splitDropsMeta`) with `splitBlockDef`, because a deny-list only drops what it names and every new key has to be added by hand. Editor and media declare what their split creates: a fresh pane of the same kind that keeps the source pane's **settings**, an allow-list, and nothing else.

1. **`PaneTabCapabilities.splitBlockDef?: (source: Block) => BlockDef`.** It now receives the pane being split, so a view can carry its settings across. `splitBlockDefFor` passes it on every split path. The agent view ignores it, unchanged.
2. **Editor** (`editor.tsx`): `{ view: "editor" }` plus, from the source, `editor:tree_width`, `editor:tree_expanded`, `editor:show_hidden`, `editor:word_wrap`, `editor:preview_height`, and `connection` when it is an SSH host (the same rule the copy used: a local pane has no `connection`). No `doctabs`, `file`, `editor:line`, `editor:pending_open_files` or `editor:scratch`, so it opens with no tabs. The file tree still shows its usual roots (home and drives), which never came from the documents.
3. **Media** (`media.tsx`): `{ view: "media" }`. Media has no per-pane settings to carry; it opens on its "Click to load media" state.

Because `splitBlockDef` applies to every split path, the palette and the split shortcuts now also open an empty editor or media pane from one, instead of a terminal. That matches the agent pane, where every path opens the picker, and makes "split" mean one thing for these panes.

## Not changed

- Splitting any other view: still a copy of the pane (or the configured new block, for the palette and shortcuts).
- Opening a file into an existing editor or media pane, and document tabs within a pane.
- Moving a document tab into a new pane (tear-off), which is a move, not a split.

## Tests

- `split-block-def.test.ts`: `splitBlockDefFor` passes the source pane to `splitBlockDef`.
- Editor: a split of an editor with tabs, a legacy `file`, a pending open, an open-at-line and `editor:scratch` gives `{ view: "editor" }` with the five settings and none of those keys; an SSH `connection` is kept, a local one is not.
- Media: a split of a media pane with tabs, `media:path` and `media:open` gives exactly `{ view: "media" }`.
- `block-registry.test.ts`: `splitBlockDef` is declared by exactly agent, editor and media, so every split path (pane menu, palette, shortcuts) routes through it for them.

## Open question

The palette and shortcut paths changing from "a terminal" to "an empty editor/media pane" (above) is the recommended reading of "if you split the pane". If the operator prefers those paths to keep opening a terminal, the alternative is to apply the editor/media rule only to the pane menu's split.
