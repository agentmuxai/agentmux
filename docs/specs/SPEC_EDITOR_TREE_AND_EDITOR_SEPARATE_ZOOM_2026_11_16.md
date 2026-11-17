# SPEC: the Editor's file tree and the editor zoom separately

**Status:** active. Approved (all four section-7 decisions as recommended); the build is split into two PRs: shared mechanism + Editor first, Files second (§9). Spec task #47856; the build is #47857. Owner: Camper@narko. Written 2026-11-16 against origin/main `ccf775a5f8`. Local and uncommitted: it ships in the build PR.
Everything in §2 was read from the code, not run. The build's first step confirms the three things marked **(verify)** in a `task dev` of its own.

## 1. The request

The request (task #47856): decouple zoom in the Editor so the file tree has its own zoom level; scroll zoom works as it does elsewhere, the zoom keys follow what has the keyboard (the tree or the editor), and every edge case is thought through.

Reading (confirmed against the code): in the Editor pane the file tree and the editor each keep their own zoom level. Ctrl/Cmd+wheel zooms the part under the pointer. The zoom keys zoom the part that has the keyboard.

The Editor is a panes-seat area (mine); the zoom store was Agent1's. This spec changes the store in one place (§4.2) and adds no new zoom feature to it; I will tell Agent1 when the build opens.

## 2. How zoom works in the Editor today

**One level, applied to the whole pane.** The Editor has one zoom: block meta `term:zoom` (clamped 50% to 200%, `null` at 100%; `store/zoom-factor.ts`). `EditorViewModel.zoomAtom` reads it through `paneZoom` (`editor-model.ts:452`), and `editor-view.tsx:134` applies it as CSS `zoom` on the root `.editor-view`. The tree column, the divider, the document tab strip, the Markdown toolbar, CodeMirror and every dialog are children of that one element, so they scale together. That is why they appear coupled: they are one zoomed box.

**What is above it.**

| Zoom | What it is | Reaches the Editor? |
|---|---|---|
| Window (page) zoom | pinned to 1.0 at startup | no |
| Chrome zoom | `--zoomfactor`, `sessionStorage`; title bar, status bar and the pane header (`.block-frame-default-header`) | not the parts, except the PDF case below |
| Pane zoom | `term:zoom`, the one level above | yes: it is today's whole-pane zoom |
| Image viewer's own zoom | a transform per image, not saved; plain wheel | inside the viewer only |
| PDF content zoom | `term:zoom` read as a multiple of fit-width (`viewers/pdf`), range raised by `setZoomMax` | only while a PDF is in front |

**The PDF case today (`SPEC_ZOOM_ANCHOR_AND_AUDIT` §10).** While a PDF is in front, `.editor-view` follows *chrome* zoom and carries `data-zoom-scope="chrome"`; the pages follow `term:zoom`. So today the tree changes size when you open a PDF (it jumps from pane zoom to chrome zoom). Decoupling removes that jump (§5, case 8).

**The entry points.**
- Ctrl/Cmd+wheel: `usePaneZoomWheel` on the Editor root (capture phase, before CodeMirror) zooms `model.blockId`; elsewhere the window's `AppZoomHandler` zooms the block under the pointer (`closest("[data-blockid]")`). A wheel over the chrome (`isOverChrome`) zooms chrome instead.
- Keys: `view:zoom:in/out/reset` (Ctrl/Cmd `=`, `-`, `0`) → `zoomIn/zoomOut/zoomReset` on `getFocusedBlockId()`, the *selected pane*, never a part of it. `view:zoom:resetAll` (Ctrl/Cmd+Shift+0) resets every pane. Ctrl+Shift+wheel steps every pane (`stepAllPanes`).
- The same three commands are in the command palette ("Zoom In", "Zoom Out", "Actual Size") and the macOS View menu. There is no zoom entry in the pane's own header menu.
- The indicator (`ZoomIndicator`): a transient box centred on the pane's body, text "110%". Plain `div`, no live region.

**What the Editor already has that this builds on.**
- A focus-region tracker (`useFocusRegion`, `editor-commands.ts:88`): `"tree" | "body" | null`, updated on `focusin`, cleared when focus leaves the pane. It draws the accent line (`.editor-region--focused`) on the column that has the keyboard, but only while the tree is open.
- `Ctrl/Cmd+Shift+E` (`editor:focusTree`) moves the keyboard between tree and editor; Enter on a file and Escape in the tree hand it back to the editor (`file-tree-state.ts:189-216`). The tree's key handler ignores any key with Ctrl/Meta (`:176`), so zoom keys reach the app's shortcuts.
- The zoom anchor (`store/zoom-anchor.ts`): the Editor's CodeMirror adapter holds the character under the pointer across a zoom. It is keyed by block id.
- Held local zoom (`store/pane-zoom-local.ts`): a level is shown at once and the next step starts from it, keyed by block id.
- The tree is **not** virtualised: plain nested `<For>` rows, padding-left `4 + depth × 16` px (`file-tree-node.tsx`), scroller `.file-tree`. There is **no minimap and no diff view** in the Editor today.
- A split of an Editor pane (`editor-split.ts`) carries only `editor:tree_width`, `tree_expanded`, `show_hidden`, `word_wrap`, `preview_height`. It does not carry `term:zoom`.

## 3. The design in one page

1. **Two zoom parts, one pane.** The *editor part* keeps `term:zoom` exactly as it is (so the all-panes zoom, the PDF content zoom, the agent tools and every saved pane keep working). The *tree part* is new: block meta `editor:tree_zoom`.
2. **The root is no longer zoomed.** CSS `zoom` moves from `.editor-view` to two wrappers: one inside the tree column (the tree part) and one around the main column (the editor part: tab strip, toolbar, CodeMirror or viewer, Markdown preview, find box, banners). The divider and the pane's dialogs sit on the unzoomed root. Same pattern as the agent pane's `.agent-view-zoomed` and the Files view root.
3. **Mouse: the part under the pointer. Keyboard: the part that has the keyboard.** Focus decides, not "a file is selected" (a file is nearly always selected). The accent line (`editor-region--focused`) is the visible ring, and the indicator names the part.
4. **Generalise the zoom store to a *zoom subject*** (§4.2): a block id plus an optional part, with the meta key it reads and writes. Every existing pane is a subject with no part, so nothing else changes.

## 4. Design

### 4.1 Which part a gesture zooms

A pure function, `zoomPartAt(blockId, source)`, decides; one place, tested without a mouse.

- **Wheel (pointer):** the part whose element contains the event target. The tree column carries `data-zoom-part="tree"`; everything else inside the Editor is the editor part. A wheel over the pane header zooms chrome, as today (`isOverChrome` is untouched).
- **Keyboard / command (no pointer):** the part with DOM focus (`useFocusRegion`). When focus is in neither (the pane header, a modal, the palette, a native menu, the window just regained focus), the **last part that had focus in this pane**, kept per block in a module map; a pane that never had focus is the editor. The tree not being shown always resolves to the editor.

### 4.2 Store change (`store/zoom.ts`, `zoom-factor.ts`, `pane-zoom-local.ts`, `zoom-anchor.ts`)

- A subject is `{ id, metaKey }`: the editor part is `{ blockId, "term:zoom" }`, the tree part is `{ blockId + "#tree", "editor:tree_zoom" }`. The held-level map and the anchor registry are keyed by `id`, so a tree step and an editor step never overwrite each other.
- `paneZoomPatch` / `writePaneZoom` take the key. The editor part still writes `null` at 100%. **The tree part writes `1` explicitly at 100%**, so "set" and "never set" stay distinguishable for the seeding in §4.5.
- The tree subject clamps to the shared 50%-200%: it never takes the per-block raised maximum (`setZoomMax`), which belongs to the PDF in the editor part.
- `paneTabCapability(..., "paneZoom")` gains an optional `parts`: `[{ id: "tree", metaKey: "editor:tree_zoom", baseFontSize: 13, label: "File tree" }]`. This is the mechanism other panes can use later (§5, case 16).
- `AppZoomHandler` and `AppAllPanesZoomHandler` read `target.closest("[data-zoom-part]")` and, when the block's view declares that part, step its subject instead of the pane's.
- The Editor's `usePaneZoomWheel` (which is on the root and would catch a wheel over the tree first) gets `applies = (ev) => !inTreePart(ev.target) && !(contentZoomed() && isOverChrome(...))`, so a wheel over the tree falls through to the window handler. A second `usePaneZoomWheel` on the tree column is not needed.

### 4.3 What each wrapper applies

- **Editor part** (`editor-main-zoom`): `zoom: model.zoomAtom()`. While a PDF is in front, its tab strip and viewer bar follow chrome zoom and the pages the pane zoom, exactly as today (`data-zoom-scope` moves from the root to this wrapper).
- **Tree part** (`editor-tree-zoom`, inside `.editor-tree-column`): `zoom: model.treeZoomAtom()`, `data-zoom-part="tree"`, `data-zoom-scope="pane"` (so the PDF's chrome scope never reaches it).
- **The divider:** `treeResizeHandler` divides by the tree zoom, not by a root zoom (§5, case 7). `previewResizeHandler` (the Markdown preview height) divides by the editor zoom.

### 4.4 Anchoring

Each part has its own anchor (the tree under its own subject id). The tree's adapter holds **the row under the pointer** (its `data-path` and its offset from the scroller's top) and puts it back after the zoom; the keyboard anchor is the selected row (`tabStop`) when it is on screen, else the pointer if over the tree, else the row at the tree's vertical centre. The tree is not virtualised, so rows are measured from the DOM and one pass is enough; the shared settle loop still runs.

### 4.5 Where the levels are saved, and the first start

- Per pane, in block meta (srv): `term:zoom` (editor, as now) and `editor:tree_zoom` (new, additive).
- **First start after the upgrade ("both parts start from today's zoom"):** when an Editor model initialises and `editor:tree_zoom` is **unset**, it writes `editor:tree_zoom = term:zoom` (or `1`) once. Writing at 100% too is deliberate: otherwise a pane at 100% that is later zoomed to 150% in the editor would, after a reload, seed the tree from 150%. One write per pane, ever. A pane that has the key never reseeds.
- **Downgrade:** an older version ignores `editor:tree_zoom`; the whole pane follows `term:zoom` again.

## 5. The 17 cases, one line each

**Target of an action**

1. **Wheel (Ctrl/Cmd; a trackpad pinch arrives as Ctrl+wheel with a small delta and takes the same path):** over the tree (rows, empty space under them, its header and toolbar) → tree. Over the editor, the doc tab strip, the Markdown toolbar, the preview, a viewer, the empty state, a banner or a dialog → editor. Over the 4 px divider → editor (the pane default; the divider belongs to neither column). Over the pane header (`.block-frame-default-header`), the title bar or the status bar → chrome zoom, unchanged. Over a pane that is not the selected one → the part under the pointer, without selecting or focusing the pane (as every pane does today).
2. **Keys (Ctrl/Cmd `=` `-` `0`) by focus:** focus in the tree (a row, or its rename or new-entry box) → tree; focus in the editor (text, a viewer) or its find box or tab-rename box → editor; focus in the pane header, a modal, the palette, a native menu, or nowhere in the pane → the last part that had focus here, else the editor. **Focus decides, not selection**: "a file is selected" is almost always true. The shown cue is the accent line on that column (`editor-region--focused`, kept) and the indicator naming the part (case 14). While the tree is hidden the editor is the only target.
3. **After a click in the tree:** a single click previews the file and leaves the keyboard on the clicked row, so the next shortcut zooms the tree (read from `file-tree-state.ts`; **verify** that a preview load does not pull focus into CodeMirror). Double-click pins the file and leaves the keyboard in the tree as well. Enter on a file and Escape in the tree move the keyboard to the editor (existing), so the next shortcut zooms the editor. Arrow keys move the selection and preview with the keyboard staying in the tree. A pane that is selected from elsewhere (pane arrows, a new pane) gets `giveFocus()`, which is the editor body, so shortcuts zoom the editor, as today.
4. **Reset (Ctrl/Cmd+0)** resets only the targeted part (tree → 100%, editor → 100%). There is a "reset both for this pane" as a palette entry (case 5) and the existing `view:zoom:resetAll` (Ctrl/Cmd+Shift+0) resets **both parts of every pane**.
5. **Menus.** The pane's header menu has no zoom entries today and gets none. The generic palette and macOS entries ("Zoom In", "Zoom Out", "Actual Size") act on the part the keyboard is in (case 2); their labels stay, because the app-level macOS menu cannot say which part. The palette gains seven entries with the part in the label: "Zoom in file tree", "Zoom out file tree", "Reset file tree zoom", "Zoom in editor", "Zoom out editor", "Reset editor zoom" and "Reset zoom in file tree and editor". They run on the selected Editor pane and are listed only when the selected pane is an Editor. New shortcuts: none (the table's six `view:zoom:*` rows are enough; part-specific keys would only add rows to remember).

**What scales**

6. **Tree:** one CSS `zoom` on one wrapper scales its text, icons, row height, indent (`4 + depth × 16` px), the chevrons and the inline rename box together, by construction; there is no per-element sizing. Scroll: the anchor (§4.4) keeps the row under the pointer, or the selected row, in place; `scrollIntoView` for the selected row still runs on key moves. Row measurement: the tree has no virtual list, but `pageSize()` (PageUp/PageDown) reads a row's `offsetHeight` and the scroller's `clientHeight` in the same zoomed subtree, so it needs a test at 50% and 200%.
7. **Tree width:** the stored width (`editor:tree_width`, today's 150-600) is the width **at 100% tree zoom**; the column is drawn at stored × tree zoom, so the number of characters per row is the same at any level (what the person has today, because the whole pane scaled). A narrow pane still shrinks the column below that (`flex: 0 1`). Dragging the divider writes `(start drawn width + dx) ÷ tree zoom`, so the saved value does not change meaning when the zoom changes. The alternative (fixed screen pixels, zoom only enlarges the text and truncates names) is cheaper to build but loses the "same fit" property; see decision 1.
8. **Editor:** the editor wrapper scales CodeMirror's text and gutter, the doc tab strip, the Markdown toolbar and preview, banners, the find box and the empty state together. There is no minimap and no diff view today; one added later sits in the main column and follows the editor zoom. Image, hex and media viewers: the editor zoom scales their chrome and fit area as today and leaves their own zoom (the image's transform, not saved) alone. PDF: unchanged from §10 of the anchor spec (pages follow the editor zoom, tab strip and viewer bar follow chrome zoom). **New:** the tree does not follow chrome zoom while a PDF is in front any more; it always follows its own level, so opening a PDF no longer resizes it.
9. **Several open files:** one editor zoom per pane, not per file. All tabs share one CodeMirror container, and a per-file zoom would resize the pane on every tab switch and need a saved level per path.
10. **Limits and steps:** each part is 50%-200% (the editor part may go higher only while a PDF is in front, as now). Steps: keys 10%, wheel 5% × notches (a trackpad's small deltas take a fraction of a step), the same stepper that skips a step landing on the same font size; the tree uses 13 px as its base like the editor. **Combination:** a part's level is its CSS `zoom` and nothing multiplies into it: window zoom is pinned at 1.0 and chrome zoom never scales a part (apart from the PDF's strip and bar in the editor part). The smallest result is the shared 50% floor every pane has today (no new floor; the look check judges the tree at 50%); the largest is 200%, where the tree at its stored width draws up to 2× wider, and the pane's shrink rule keeps the editor from collapsing below its minimum.

**Keeping it**

11. **Saved per pane** in block meta, so it survives a restart, a tear-off to another window and a move between tabs (the same block object keeps its meta), and an upgrade (new key is additive; first start seeds it, §4.5). **Duplicate** = the split: it carries neither level today and keeps doing so (both parts start at 100%); decision 2 asks whether it should carry both.
12. **Tree hidden or collapsed** (`treeExpandedAtom` false): no tree column, so a wheel never targets it and the keyboard resolves to the editor. The tree's level stays in meta and is applied again when the tree is shown. The tree toggle in the doc tab strip belongs to the editor part.
13. **New Editor pane** (including `Pane open editor` from an agent, and a split): both parts 100%, no `term:zoom` and, after the first start, `editor:tree_zoom = 1`. Agent-open seeding of `ui:zoom` is for agent panes only and does not apply.

**Feedback and reach**

14. **Indicator:** the existing centred transient box, no window modal. When the tree is shown the text names the part ("File tree 130%", "Editor 110%"); with the tree hidden it stays "110%". The box centres on the part's own column (the target takes the part, not only the block). The words go through `t()` in the indicator component; the store passes a part key, not English.
15. **Accessibility:** the indicator becomes a polite live region (`role="status"`), which announces "File tree 130%" for everyone using it (a small shared fix, today it has none). Keyboard-only: `Ctrl/Cmd+Shift+E` reaches the tree and `Ctrl/Cmd+=`/`-`/`0` zooms it, then Enter/Escape returns and the same keys zoom the editor; no mouse needed. Touch and pen: a touch pinch has no zoom handler today and stays out of scope; a pen has no wheel; a pen or touch tap in the tree puts focus on the row, so the keys follow case 2. Reduced motion: no new motion.
16. **The mechanism for other panes** (Files, Settings, Kanban, any list beside content): the `paneZoom.parts` capability and `data-zoom-part` attribute of §4.2, plus the per-block last-focused-part map. A pane declares a part (id, meta key, label), marks its element, and reports focus; the store, wheel routing, indicator, all-panes step and anchor then work for it. Files and Settings are not changed here.
17. **Tests** (vitest unless noted):

| Case | Test |
|---|---|
| 1, 2 | `zoomPartAt`: pointer in tree, editor, divider, header; focus in tree, rename box, editor, find box, header, nowhere; last-part memory; tree hidden |
| 2, 4, 5 | `command-registry`/`keymodel-registered`: every new palette command has a handler; palette lists them only for an Editor; "reset both" writes both keys |
| 3 | `file-tree.behavior`: a click, a double-click and an arrow leave focus in the tree; Enter and Escape move it to the editor (existing, extended) |
| 6 | tree anchor adapter on a fake scroller: row under the pointer held; selected row held; none selected |
| 6 | `pageSize()` at 50% and 200% |
| 7 | `treeResizeHandler`: width written is drawn ÷ tree zoom; narrow pane shrink |
| 8, 9 | `editor-view.render`: zoom style on the two wrappers, not the root; PDF in front: scope on the editor wrapper only, the tree wrapper `pane` |
| 10 | stepper: tree clamps at 50% and 200% and ignores `setZoomMax` |
| 11 | model init: seeds `editor:tree_zoom` once from `term:zoom` and from none; never reseeds; split allow-list unchanged |
| 12 | tree hidden → editor target; level survives hide and show |
| 13 | new pane: no keys until first start, then `editor:tree_zoom = 1` |
| 14, 15 | indicator text per part; `role="status"` present; `t()` keys exist |
| all-panes | `stepAllPanes` steps both parts and the summary range covers them |
| Look check | real pointer and keyboard on a `task dev`: the wheel over each part, the accent line, indicator position, the tree at 50% and 200% in a 380 px pane, the PDF jump gone, the anchor holding a row. Clare (macOS) and Maricon (Linux) before merge. |

## 6. More cases found while reading the code (beyond the 17)

- **Ctrl+Shift+wheel and Ctrl+Shift+0 (all panes):** each Editor pane steps **both** parts, each by its own level; one summary indicator. Chosen because "all panes" means everything scales; decision 3.
- **A magnified or tiled Editor:** no difference; the parts zoom inside whatever box the pane has.
- **A wheel over a modal in the pane** (open-from-remote, tree dialogs, confirms): they render on the unzoomed root, so they never scale; the wheel over them goes to the editor (the pane default).
- **A held level while the tree is mounting:** the tree subject uses the same held-level rule (shown at once, step from the held level), so two quick ticks are two steps.
- **A wheel arriving mid-rename in the tree:** zooming does not cancel the rename box; focus stays in it.
- **`Pane` tool (agents):** an agent that sets block meta can write `editor:tree_zoom`; out-of-range values clamp when read, as `readZoom` does for `term:zoom`.
- **Docs and tips:** `keybindings/defaults.ts` rows keep their labels but the help and `docs/keybindings.md` say which part each key zooms; `tips.ts` gets a line for the tree.

## 7. Decisions (approved as recommended)

1. **Tree width when it zooms:** scales with the zoom, so the same names fit at any level (recommended, matches what you have today), versus a fixed width in screen pixels where zoom only enlarges the text.
2. **Split carries zoom?** Neither level today and after (recommended: no change), versus a split carrying both.
3. **All-panes zoom:** steps both parts of an Editor (recommended), versus the editor part only.
4. **Tree stays on its own level while a PDF is in front** (recommended), versus keeping today's jump to chrome zoom.

## 8. Plan and size

Two PRs (build #47857): the shared mechanism with the Editor first, Files second (§9). About 1 to 1.5 days for the first.

1. Confirm in a `task dev` of my own: CSS `zoom` on the two wrappers lays out correctly in a flex row (width, heights, the shrink rule), a preview load does not pull focus into CodeMirror, and `offsetHeight` in the tree is consistent (the three **(verify)** items).
2. Store: subjects, parts capability, keyed held-level and anchor, the wheel routing; tests.
3. Editor: two wrappers, tree zoom atom, seeding, divider conversion, anchor adapter, `zoomPartAt`, palette entries, indicator naming and live region, all-panes step.
4. Docs: keybindings doc and tips.
5. Look checks on macOS and Linux; merge on muxreview approval; report to AgentY and AgentX.

Files: `store/zoom.ts`, `zoom-factor.ts`, `pane-zoom-local.ts`, `zoom-anchor.ts`, `pane-tab-registry.ts`, `app.tsx` (two handlers), `element/zoomindicator.tsx`, `store/command-registry.ts`, and in `view/editor/`: `editor-view.tsx`, `editor-model.ts`, `editor-zoom-resize.ts`, `editor-commands.ts` (last-part memory), `editor-view.scss`, a new `editor-tree-zoom-anchor.ts` and `editor-zoom-part.ts`.

Not in scope: Settings and Kanban parts (§5 case 16; Files is §9), touch pinch, per-file zoom, a zoom for the doc tab strip on its own.


## 9. Files on the same mechanism (second PR)

The request widened the work: Files joins, and the Editor and Files share one parts mechanism, so a third pane only declares its parts. The mechanism is the one in §4: a subject is a block id or `blockId#part`; a pane lists its parts in its `paneZoom` capability and marks each part's element `data-zoom-part`; `store/zoom-parts.ts` and the zoom store do the routing (wheel, keys, all-panes zoom, indicator name, seeding). The first PR ships that mechanism and the Editor; the second adds Files with no change to the store.

**Behaviour.** Ctrl/Cmd+scroll over a part of a Files pane zooms that part, as in the Editor. Over the pane's other areas it zooms the main part. The zoom keys follow the part that last had focus or the pointer (the same last-part memory). All-panes zoom and reset step every part. The indicator names the part ("Places 130%" or "Files 110%") while both are on screen.

**Parts.** Places (the side list) is the `places` part, stored under `files:places_zoom`, label key `zoom.part.places`. Everything else (toolbar, list, preview and footer) is the main part and keeps `term:zoom`. This keeps the one rule of §4: a part that existed before keeps its key and today's look, and a new part is seeded once from the main level, so nobody's pane changes size on upgrade.

**Open design item, settled at build time in a `task dev`.** Files has a toolbar across the full width above Places, the list and the preview, and a footer below. `.files-view` is the container for the container queries that hide Places and the preview in a narrow pane (they count zoomed pixels today because the whole root is zoomed). Two ways to give the main part its own zoom:
1. Move the main part into one wrapper beside Places (toolbar, list, preview and footer zoom together). The toolbar then no longer spans over Places, a visible layout change.
2. Keep the toolbar full width and zoom the three main siblings (toolbar, list area, footer) separately, each with the main level.
The recommendation is 2: no layout change, and the same container threshold is kept by measuring the root's width in unzoomed pixels. The container-query thresholds are re-derived either way, and `files-column-resizer` and `iconScale` take the main level from the main part rather than the root.

**Files touched.** `files.tsx` (capability), `files-view-parts.tsx` (root zoom moves to the wrappers), `files-model.ts` (icon scale, Places level atom, seed), `files-column-resizer.tsx` (its `zoom()`), `files.scss` (container queries), `FilesPlaces.tsx`, `files-view.test.tsx` and the snapshots. Look checks on macOS and Linux as for the Editor.
