# SPEC: Drag and drop: audit, one facility, one drop indicator

**Date:** 2026-09-27
**Status:** proposed; nothing in this spec is implemented. It supersedes `SPEC_PANE_FILE_DROP_TARGET_HIGHLIGHT_2026_09_27.md`: its window-level controller and per-pane `accept`/`drop` hook carry over as §5.3 (now registered per block, not on the manifest), and its thick-border look is replaced by §4. It revives the unlanded `SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR_2026_07_11.md` as phase 4. Written against `main` @ `135251326`; spot-verify file:line citations before trusting them.
**Author:** Korp@narko
**Related:** `SPEC_PANE_FILE_DROP_2026_05_30.md` (OS file drop: the CEF drag handler and the path stash), `SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md` (what an agent pane does with files), `SPEC_TAB_WINDOW_DRAG_CONSOLIDATION_2026_07_13.md` (landed-vs-open map of window drags), `SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md` (pane tabs, and `pane-tab-tearoff.ts`, the pattern §5.6 generalises), `SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28.md` (shelved), `docs/retro/retro-md-drop-window-hijack-and-55-6-relaunch-failure-2026-08-16.md`.

---

## 0. The ask

> when dragging any file over a pane, we want to highlight the pane … for now only agent panes will be supported, but we will have future pane support too, like media and editor.

> get terminal panes in too … I didnt see the faint dashed border when I did a dnd before, lets consolidate everything there into one facility. … do the best practice as far as indicator, the pattern is well established

> do an audit of the area, ID ops for DRY, perhaps rethink a more efficient architecture, write spec to file

## 1. Summary

**Drag and drop in AgentMux today:**
- Seven drag systems run on three mechanisms: pragmatic-dnd, native HTML5, and host-native loops.
- About ten separate stores all mean "a drag is in progress".
- There are nine visual drop treatments in three colour sources, two of them hard-coded green.
- Each pane handles OS file drops by hand.

**What users see:**
- The one file-drop indicator never shows. It is the "faint dashed border" you didn't see.
- Dropping into a floating pane on Windows fails.
- The drone canvas shows a copy cursor for a file drop and then swallows it.

**What it costs:**
- One `dragover` runs N+4 window listeners (N = open window tabs).
- Every pane in every tab runs a DOM query every 100 ms, even with no drag.

**The plan:**
- **One drag facility per window.** It owns:
  - the drag session (what is being dragged);
  - one set of window listeners;
  - hit-testing;
  - one drop-indicator component and one set of tokens.
- **Panes opt in to file drops** by registering a live hook for their block: agent and terminal in phase 1; media and editor later.
- **The indicator follows the established pattern** (§4): valid drop zones are outlined as soon as files enter the window; the zone under the cursor gets a translucent accent tint, an accent border and a prompt saying what will happen; invalid zones say why before you let go.
- **Five independently shippable phases** (§7). Phase 1 is the user-visible file-drop work. The rest is consolidation that each phase proves with tests before deleting old code.

## 2. Audit

### 2.1 OS file drops

**The indicator is dead.**
- `DragOverlay` (`frontend/app/element/dragoverlay.tsx:11`) destructures `({ message, visible })`. In SolidJS that reads `visible` once, at creation, when it is `false`.
- The agent and terminal panes both render it (`agent-view.tsx:2552`, `term.tsx:452`). Neither has ever shown it.
- It also uses Tailwind's `border-accent`, which is `--color-accent`: a fixed `rgb(65,159,224)` (`tailwindsetup.css:35`) that themes don't override.

**The agent and terminal handlers are hand-copied twins.** `useAgentDropAttach.ts` vs `term.tsx`:

| Logic | Agent | Terminal |
|---|---|---|
| settings atoms + concurrency normalisation | 77-88 | 324-330 |
| `hostHas("nativeFileDrop")` gate + listener add/remove | 104, 261-268 | 377, 430-437 |
| inline `types.includes("Files")` instead of `isFileDrag` (`util/dnd.ts:87`) | 112-113 | 386-387 |
| `dragleave` `relatedTarget` containment | 125-126 | 397-398 |
| no-cwd toast | 136 (**before** consuming the stash) | 336 (after) |
| empty-stash toast (byte-identical) | 149 | 420 |
| `copyFilesToDir` + success / "Copy failed" toasts | 189-256 | 346-370 |
| overlay message (same strings) | 97-101 | 441-444 |

**Every notice is written by hand.**
- 13 hand-written `pushNotification` literals cover drop, copy and paste: 7 in `useAgentDropAttach.ts`, 4 in `term.tsx`, 1 in `AgentFooter.tsx:522`, 1 in `attachment-draft.ts:471`.
- Each repeats `icon` / `timestamp` / `type` / `expiration` by hand.
- The failure line `` `${baseName(f.source)}: ${f.error}` `` appears 5 times.
- The `cmd:cwd` lookup appears 5 times: `useAgentDropAttach.ts:90-93`, `term.tsx:333`, `term.tsx:442`, `AgentFooter.tsx:541`, `app_api/attachments.rs:63`.

**Three routes put a file into a container pane's working folder:**
- a drop: CEF `copy_file_to_dir` (`useAgentDropAttach.ts:178-257`);
- right-click Paste: CEF, a separate copy (`AgentFooter.tsx:540-550`);
- Ctrl+V: HTTP upload then srv `attachments.copy-to-workdir` (`attachment-draft.ts:244-253`).

Right-click Paste doesn't consult `dnd:enabled`, `dnd:concurrency` or `dnd:agentinserttoken`. That is correct, not a gap: the settings schema defines those settings for drops, and paste is a separate clipboard workflow. The duplication is in the copy, results and notices, not in the gating.

**Copying into a folder is written twice in Rust:**
- **CEF, `providers.rs:161-263`:**
  - it runs synchronously inside the async IPC router (`ipc.rs:434`), unlike its `spawn_blocking` neighbours;
  - de-conflicting checks with `exists()` and then copies with `std::fs::copy`, which is racy and overwrites;
  - it has no `dot > 0` guard, so `.env` becomes `_1.env`;
  - it has no tests.
- **srv, `store.rs` `copy_original_to`:** race-free (`create_new`), sanitised, tested.
- The comment at `util/dnd.ts:40` promises `report (1).csv`; both produce `report_1.csv`.

**Other problems:**
- **Floaters on Windows:** they are built with `is_browser_pane = true` (`agentmux-cef/src/floating_pane.rs:246-250`, `:468-472`), so `drag_handler()` returns `None` (`client/handlers.rs:57-60`). Paths are never stashed.
- **The path stash:** it is one process-wide slot with no window key (`drag_stash.rs:24`). **Fix: §5.4 keys it by window label; the renderer passes `consume_drag_paths {windowLabel}`.**
- **The drone canvas:** `onDragOver` always calls `preventDefault` with `dropEffect = "copy"` (`drone-view.tsx:288-291`), so an OS file shows a copy cursor and is swallowed.
- **Dead setting:** `dnd:maxfilesizemb` (`settings-template.jsonc:23`) is read nowhere.
- **Untested:** the terminal drop path, `installGlobalDropGuard`, `on_drag_enter`, and the CEF copy.

### 2.2 In-app drags

| System | Mechanism | Where |
|---|---|---|
| Pane (tile) move | pragmatic-dnd | `layout/lib/TileLayout.core.tsx:449`, `tilelayout-shared.tsx:318,436` |
| Window tab reorder / tear-off | pragmatic-dnd | `app/tab/droppable-tab.tsx:88,220`, `tab-reorder.ts` |
| Pane-tab pills | pragmatic-dnd | `app/element/PaneTabStrip.tsx:361,670,687` |
| Drone chip → canvas | native HTML5 + pointer events | `app/view/drone/drone-view.tsx` |
| Global-memory list reorder | native HTML5 | `global-bundle-manager.tsx:83-116` |
| Cross-window tear-off | dragend monitors ×3 platforms | `app/drag/CrossWindowDragMonitor.*.tsx` |
| Floater move / redock | host native loop | `workspace/floating-pane-workspace.tsx`, `app-init.ts:117-270` |

**"A drag is in flight" is stored many times over:**
- **A tile drag** is recorded in six places at once:
  - `dragState.nodeId` (`tilelayout-drag-state.ts:21`)
  - `tileDragInFlight` (`dragInFlight.ts:20`)
  - `layoutModel.activeDrag` (`layoutModel.ts:249`)
  - the payload `kind: "tile"`
  - `elementDragInFlight()` (`element-drag-state.ts:22`)
  - the tile's own `isDragging`
- **A window-tab drag** is recorded in five: `globalDragTabId`, `isDragging`, payload `"tab"`, `elementDragInFlight`, and host `setJsDragActive`.
- **`dragInFlight.ts` exists only** because the payload is cleared at drop, before `dragend`.
- **Four separate nets catch a swallowed `dragend`:**
  1. `TileLayout.win32.tsx:87`, once per TileLayout
  2. `element-drag-state.ts:36`
  3. `tab-reorder.ts:343-363`
  4. the win32 monitor's 800 ms poll
- **`dragEscaped`** lives in the tab module (`tabbar-dnd.ts:31`) but gates tile and pane-tab payloads in all three monitors, and nothing sets it during a tile drag.

**Per-event cost:**
- **One `dragover` runs N+4 app-level window listeners:**
  - pragmatic's lifecycle and honey-pot fix (2);
  - the file-drop guard (`app-init.ts:1029`);
  - **one per mounted TileLayout** (`TileLayout.core.tsx:249-258`; every window tab stays mounted), each resetting a 100 ms debounce that then calls `getBoundingClientRect`;
  - the win32 tab listener (`tab-reorder.ts:132`) or the darwin/linux monitor's document listener.
- **With no drag at all:** each pane re-registers its draggable by polling `setInterval(register, 100)` for its whole life (`TileLayout.core.tsx:522`). That is 10 × (panes in all tabs) `querySelector` calls a second.
- **Type checks are scattered:**
  - three type tags live in three modules (`tilelayout-shared.tsx:42`, `tabbar-dnd.ts:6`, `PaneTabStrip.tsx:46`);
  - `source.data.type === tileItemType` is repeated 5 times.

### 2.3 Cross-window drags

**The three monitors are mostly copies.**
- `CrossWindowDragMonitor.{win32,darwin,linux}.tsx` (464 / 302 / 351 lines) share roughly 140–175 lines per pair. darwin and linux are nearly identical.
- Copied three times: `DragItemPayload` and its getter and setter, `handleCrossWindowDragEnd`, and `performTearOff` (whose floater-open retry appears **six** times).
- Every drag makes an unused `listWindows()` IPC call (win32:203-207, result never read), and `performCrossWindowDrop` is an empty function (win32:269-276).
- `app/drag/pane-tab-tearoff.ts` already does this right: one module with a `platform` parameter and a de-duplicated `openFloatingPaneWindow` retry (:195-213).

**"Which window is under the cursor" is answered five times:**
- `commands/drag.rs:193` (Windows only, map order rather than Z-order; returns `None` on macOS/Linux, `:220`);
- `window/motion.rs:280` with `ui_tasks/window.rs:1500`: a Z-ordered walk on Windows only. On macOS/Linux, `ResolveWindowAtCursorTask` (`ui_tasks/window.rs:1448`) tests CEF window bounds and, among overlapping non-main windows, picks the **lexicographically smallest label**, not the one on top. **Fix: §5.6 step 1 makes it stack-aware before anything reuses it;**
- `tear_off_hook.rs:703`;
- the macOS CGWindowList path;
- the frontend `pane-tab-tearoff.ts:48` `isInsideWindow`, which exists only because of the `None` at `drag.rs:220`.

**A cross-window tab drop commits twice.** Both `DragOverlay.tsx:84-123` and `tab-tearoff-events.ts:231` commit it, and they are de-duplicated at runtime by `wasTabRecentlyMerged` (`tabbar-dnd.ts:196`).

**Dead host surface:**
- `set_js_drag_active` is a no-op (`drag.rs:537-540`), called only from `TileLayout.linux.tsx`.
- `set_drag_cursor` / `restore_drag_cursor` have no callers.
- `tear_off_sc_move_handshake` is unreachable because `skipScMove` is always true (`tab-tearoff-rpc.ts:309`).
- The `stubs.rs:14-19` comment is stale.

### 2.4 Visual feedback

| Treatment | Colour | Shape |
|---|---|---|
| `drop-target-hover` mixin (`drop-feedback.scss:59-69`) | `--accent-color`, pulsing to hard-coded green `rgba(34,197,94,.5)` (`:21`) | 2px outline, invert strobe ×2 + pulse |
| Tile placeholder (`tilelayout.scss:232-260`) | `rgba(accent,.2)` fill, `.5` border | 1px |
| Floater redock ghost (`app.scss:54-62`) | `accent/.18` fill | 2px |
| Cross-window overlay (`drag/drag-overlay.scss:4-33`) | accent border, hard-coded green fill `rgba(34,197,94,.08)` (`:14`) | 2px dashed |
| File-drop overlay (`element/dragoverlay.tsx`) | Tailwind fixed blue, black scrim + blur | 2px dashed |
| Pill insertion (`PaneTabStrip.scss:356-362`), memory card (`native-memory-manager.scss:562-565`), tab gap (`droppable-tab.tsx:69-78`) | accent | 2px inset / padding gap |

**Other inconsistencies:**
- **Being dragged:** four different treatments for the dragged source (tile blur 8px; tab opacity .35; pill .4; memory card .5).
- **Keyframes:** `drop-feedback.scss`'s keyframes are emitted twice, once per `@use`.
- **Two components are named `DragOverlay`:** `app/drag/DragOverlay.tsx` (cross-window) and `app/element/dragoverlay.tsx` (file drop).

## 3. Goals

1. **One facility per window** owns drag state, window listeners, hit-testing and drop indication. Features plug into it; they don't reimplement it.
2. **File drops use the established indicator pattern (§4)** and are handled once. Agent and terminal panes are in phase 1. Any pane type opts in by declaring a hook.
3. **No behaviour regressions** in the fragile internal-drag paths. Each consolidation step lands behind characterisation tests of today's behaviour before old code is deleted.
4. **Measurably cheaper:**
   - one app-level `dragover` listener per window (plus pragmatic's own);
   - no idle polling;
   - no wasted IPC per drag.
5. **One Rust implementation per job:** one "copy into a folder" and one "window under the cursor".

## 4. The drop indicator: best practice

**What the sources converge on.** For file drops into a region of an app, NN/g, design systems and desktop apps agree:
- **Show valid drop zones as soon as a drag starts.** Dotted or dashed borders are "especially common in file uploads"; show "a visual signifier when the dragged object is within the active drop zone" [NN/g].
- **Mark the zone under the cursor with a fill.** VS Code covers the target editor group with a semi-transparent overlay (`editorGroup.dropBackground`, required to be translucent so content shows through) and floats a text prompt over it (`editorGroup.dropIntoPromptForeground/Background`) [VS Code].
- **Change the border as well.** Carbon's uploader changes the border's colour and thickness when files are over it [Carbon].
- **Animate briefly** (~100 ms) [Smart Interface Design Patterns].
- **Warn about an invalid drop while hovering, not after** [Pencil & Paper].
- **Make the target big**, e.g. Slack's whole-pane zone. A whole pane is the target here, which satisfies this.

**Decision:**

| State | When | Look |
|---|---|---|
| **Armed** | files are over the window; on every visible pane whose `accept` says ok. Evaluated once when the drag enters the window, and refreshed once when the file names arrive (§5.3), so a pane can switch between Armed and Blocked a few milliseconds in. | 1px dashed accent outline at 50% opacity, inset. No fill, no text. |
| **Target** | the accepting pane under the cursor | translucent accent tint over the whole pane (`--drop-target-bg`, accent at ~14%) + 2px solid accent inset border + a centred **prompt chip** (icon + "Drop 3 files to attach" / "Copy 3 files to C:\work") |
| **Blocked** | an accepting pane that can't take this drop right now | no tint; prompt chip in the warning style with the reason ("No working folder for this agent"); cursor no-drop |
| none | panes that don't accept files, window chrome | nothing; cursor no-drop |

**Details of the look:**
- **Why the tint, not a thicker border:** the tint is what makes the target unmistakable against every pane border, focused or not. The border only frames it, so it keeps the standard 2px width.
- **Alone in its tab:** the indicator draws on a layer above the pane content inside `.pane-stack`, not on the focus-ring pseudo-element. It shows even when the pane is alone in its tab and the ring is hidden (`PaneChrome.scss:68-70`).
- **Motion:** 100 ms fade in and out, and nothing under `prefers-reduced-motion`. No strobe or pulse on the file-drop states, because a file drag can last seconds.
- **Tokens:** one partial, `frontend/app/drag/drop-indicators.scss`, defines them, all derived from the theme's `--accent-color` / `--accent-color-rgb` and `--warning-color`:
  - `--drop-zone-armed-border`
  - `--drop-target-bg`
  - `--drop-target-border`
  - `--drop-prompt-bg` / `--drop-prompt-fg`
  - `--drop-blocked-fg`
- **Phase 3** moves every internal drop treatment in §2.4 onto the same tokens: the tile placeholder, the redock ghost, the cross-window overlay and the pane-tab foreign hover. That deletes both hard-coded greens and emits the keyframes once. Their shapes (insertion line, tab gap, placeholder) stay; only colour and motion unify.
- **Accessibility:** pointer-only, like every drag. The prompt text is real text, so the target is not signalled by colour alone. Keyboard users keep the existing non-drag routes (attach via Ctrl+V; copy via the file picker, where a pane has one).

Sources: [NN/g: Drag-and-drop](https://www.nngroup.com/articles/drag-drop/) · [VS Code theme colours](https://code.visualstudio.com/api/references/theme-color) · [Carbon: file uploader](https://carbondesignsystem.com/components/file-uploader/usage/) · [Smart Interface Design Patterns: drag-and-drop UX](https://smart-interface-design-patterns.com/articles/drag-and-drop-ux/) · [Pencil & Paper: drag & drop](https://www.pencilandpaper.io/articles/ux-pattern-drag-and-drop)

## 5. Architecture

Everything below lives under `frontend/app/drag/` unless noted. A module is created once per renderer: each main window, tear-off and floater has its own.

### 5.1 `drag-session.ts`: one answer to "what is being dragged"

```ts
type DragKind = "files" | "tile" | "window-tab" | "pane-tab" | "drone-kind" | "list-item";
interface DragSession {
    kind: DragKind;
    dragId: string;                 // minted at begin (§5.6)
    source?: { nodeId?; tabId?; blockId?; wsId? };
    /** Kind-specific data captured at drag start that can't be re-derived later. */
    payload?: {
        // pane-tab: the visible pane's rect at start (PaneTabStrip.tsx:641). A background
        // pill has no [data-blockid] element to measure at tear-off time, so without this
        // pane-tab-tearoff.ts:154 falls back to 720×480.
        paneSize?: { width: number; height: number };
        // window-tab: the lone-tab eligibility flag (§5.1)
        crossWindow?: boolean;
    };
    escaped: boolean;
    startedAt: number;
}
session(): DragSession | null        // reactive
begin(kind, source) / end(reason)    // end: "drop" | "cancel" | "dragend" | "button-up" | "files-idle" (files only)
```

- **Writers:**
  - the pragmatic `onDragStart` sites (tile, window tab, pane tab) begin a session. `begin` takes an eligibility flag: a **lone-tab** window-tab drag begins with `crossWindow: false`, so it is never eligible for tear-off or content-drop handling. Today it carries no payload at all (`droppable-tab.tsx:141-149`), and only the native strip-merge path may handle it (§5.8);
  - the drone chip;
  - the list reorder;
  - the file-drop controller (§5.3), on the first `dragenter` whose types include `"Files"`.
- **It replaces:**
  - `dragInFlight.ts`;
  - `element-drag-state.ts`;
  - the three `_currentDragPayload` copies;
  - `globalDragTabId`;
  - `dragEscaped`;
  - the ad-hoc `isDragging` locals, which become `session()?.source.nodeId === mine`.
- **`layoutModel.activeDrag` stays a per-tab atom** that other code reads. It is now written from one place: a session subscriber.
- **Who may end a session.** This has to follow today's ordering, not simplify it.
  - An **accepting target-side drop** in this window ends it immediately.
  - A **source-side `onDrop`** (pragmatic calls it even when the release lands outside every in-window target) must **not** end it. Today's handlers deliberately keep the payload there for the cross-window monitor (`TileLayout.core.tsx:504-514`, `droppable-tab.tsx:175-200`, `PaneTabStrip.tsx:624-625`). They only mark the session `released`.
  - The **cross-window monitor** ends a `released` session after it has handled the document `dragend` (tear-off or cross-window drop).
  - The final **safety net** ends whatever is left.
- **One safety net** replaces the four. It is based on the real drag state, never on inactivity:
  - the document `dragend` (after the monitor);
  - on Windows, today's button-state poll (`getMouseButtonState`, win32 monitor), which catches a swallowed `dragend`.

  **Internal sessions have no inactivity timeout.** A drag held over another window for any length of time sends the source renderer no events, and that is normal. The only inactivity watchdog is the file-drop one (§5.3), and it applies only to `"files"` sessions. There it clears visuals only: the path stash is untouched, and re-entering the window begins a new session.
- **The drag tags** (`tileItemType`, `tabItemType`, `paneTabItemType`) move to one `drag-types.ts`, with `isTileSource(source)` style helpers.
- **Every field today's payloads carry moves into the session**, not only the ids. When phase 4 retires a payload, the characterisation tests (§5.8) must show each of its fields is still delivered to its consumer. `paneSize` is the known case.

### 5.2 `window-drag-events.ts`: one set of window listeners

- **One set of capture-phase `dragenter` / `dragover` / `dragleave` / `drop` / `dragend` listeners per window.** Subscribers register with a kind filter:

  ```ts
  onWindowDrag({ kinds: ["tile"], over(e), leave(e), drop(e) })
  ```

- **Decisions are synchronous; only visuals are coalesced.**
  - Every `dragover` and the `drop` event hit-test synchronously and decide `preventDefault` / `dropEffect` from the pane actually under the pointer. The test is a `closest()` plus a cached verdict for that pane, so it's cheap enough per event.
  - Only the visual state updates (Armed/Target/Blocked, the prompt) are batched to one per animation frame.
  - A decision is never taken from an earlier frame's answer. Otherwise a release that lands before the next frame would act on the pane the pointer just left.
- **It replaces:**
  - `installGlobalDropGuard` (`app-init.ts:1029`), whose guard becomes the hub's "files and nobody accepted → prevent default";
  - the per-TileLayout window `dragover` (×N);
  - `tab-reorder.ts`'s win32 listener;
  - the darwin/linux monitors' document `dragover`.
- **Result:** one app listener per event, however many window tabs are open.

### 5.3 `file-drop.ts`: the file-drop facility

It carries over from the superseded spec, with the §4 look.

**How a pane opts in:** the pane instance registers its **live** hook by block id when it is created, and disposes it with the instance.

- **Why not the manifest:** the manifest registry holds type-level manifests only. `resolvePaneTabView` canonicalises a view name (`pane-tab-registry.ts:189-195`), and `adaptPaneTabInstance` doesn't expose per-instance objects (`pane-tab-host.tsx:36-77`). After a hit-test resolves `data-blockid`, nothing could reach that block's `accept`/`drop` closures without creating a second instance.
- **Where the call goes:** in the view model's constructor, or the view's mount, with `onCleanup(dispose)`. The registry is the source of truth for which panes accept files; no manifest flag is needed.

```ts
interface FileDropHook {
    accept(drag: DragFiles): { ok: true; message: string; icon?: string } | { ok: false; reason: string };
    /** paths: host paths from the stash; empty when the host has none (no nativeFileDrop,
     *  virtual files). files: the drop event's FileList, always present. */
    drop(input: { paths: string[]; files: FileList }): void | Promise<void>;
    /** true only if the hook cannot work from bytes alone. Default false. */
    needsPaths?: boolean;
}

// file-drop.ts: a per-window registry of live hooks, keyed by block id
registerFileDropTarget(blockId: string, hook: FileDropHook): () => void;   // returns the disposer
```

```ts
interface DragFiles {
    count: number;
    /** MIME per item, from DataTransferItem.type during dragover; "" when the OS gave none. */
    types: string[];
    /** File names, when known before the drop (see below); undefined until then. */
    names?: string[];
}
```

**What a hook can know before the drop:**
- **MIME types** come from `dataTransfer.items[i].type`, which Chromium exposes during `dragover` (names don't). They can be `""` for unregistered extensions.
- **Names:** CEF already has the full paths at drag-enter (`on_drag_enter` stashes them). A non-consuming `peek_drag_paths {windowLabel}` IPC, called once per drag on the first file `dragenter`, returns the names. The verdicts are re-asked once when the names arrive, a few milliseconds later.
- **Where neither is available**, e.g. a host without `nativeFileDrop`, a hook decides on what it has. Media and editor hooks answer `ok` with a softer prompt ("Open here") only when the type is unknown, and their `drop` reports a clear notice for a file they can't open. While hovering, Target/Blocked is exact once names or types are known. A release can still beat the asynchronous peek, so hover is best-effort and **the drop re-classifies** from the drop event itself (below). On CEF the drop is always classified exactly.

**How a drag is handled:**
- **Verdicts for every visible pane, up front.** When a `"files"` session begins (the first file `dragenter`), the controller:
  - enumerates the visible panes (the active layout's `[data-role="pane"]` elements) whose block id has a registered hook;
  - calls each `accept` once with the metadata known so far;
  - caches the verdicts for the drag.

  That is what lets every valid pane show **Armed** immediately (§4), not only after it has been hovered. When `peek_drag_paths` returns names, every visible pane's `accept` is re-run once and the Armed/Blocked states refresh. Blocked panes are never drawn Armed.
- **Hit-test:** `e.target.closest('[data-role="pane"]')` → `data-blockid` → the registered hook's cached verdict. A pane that appears mid-drag, e.g. from a tab switch, is evaluated when first hit. The per-event decision reads the cache, so `accept` never runs per `dragover`.
- **Clearing:** state clears on the session's end (drop, dragend, `dragleave` with `relatedTarget: null`, or the files-only idle watchdog, 1.2 s without a `dragover`: well above the HTML spec's dragover cadence of 350 ms ± 200 ms while the cursor is still, so holding a file still never clears the indicator). The watchdog also covers the cursor crossing a native browser pane, where the renderer gets no events.
- **On `drop`:**
  1. **Re-validate synchronously:**
     - hit-test the `drop` event's own target;
     - rebuild `DragFiles` from the drop event's `dataTransfer.files`, whose names and types are available at drop;
     - re-run that pane's `accept` on it.

     Nothing is taken from the last Target state or the cached hover verdict. A file that looked unknown while hovering but turns out unsupported is refused here, before the stash is consumed.
  2. If the pane doesn't accept, or its verdict is Blocked: `preventDefault` (the window guard) and nothing else. The stash isn't consumed; it expires on its TTL.
  3. Otherwise `consumeDragPaths()`, then call the pane's `drop({ paths, files })`:
     - **with paths:** the normal case;
     - **without paths:** still dispatched, with the event's `FileList`, unless the hook declared `needsPaths`. A `needsPaths` hook gets one "couldn't read the dropped files' paths" notice instead.
     - **Byte fallbacks:**
       - the agent pane uses the bytes path its paste already has (`uploadFiles` into the tray; the upload + `attachments.copy-to-workdir` transport of `copyIntoWorkdir` in copy mode);
       - the terminal pane uses the same bytes transport;
       - media and editor panes don't copy anything, so they never use `copyIntoWorkdir`: they **open** the file, from the path when present and from the `File` otherwise (the media and editor rows below).

       So no phase-1 hook needs `needsPaths`.
- **A drop anywhere in the pane counts**, including its header and tab strip; today only the content does.
- **The indicator** is rendered by `PaneChrome` as `<DropIndicator state message />`, from a per-window `Map<blockId, state>` signal. It replaces `element/dragoverlay.tsx`, which is deleted along with its name clash.

**Shared helpers, so pane hooks stay a few lines** (`file-drop-actions.ts`):
- `paneWorkdir(blockId)`: the one `cmd:cwd` lookup.
- `copyIntoWorkdir(blockId, source, { mention, concurrency })`: the copy, the results, the `@name` mentions and the notices.
  - **Two transports, because the inputs differ:**
    - `source` of `{ paths }` (a drop, right-click Paste) → CEF `copy_file_to_dir` over the **host-only** IPC (via `util/dnd.ts` `copyFilesToDir`) in every phase. Phase 2 replaces the implementation behind that IPC (§5.5), not the channel, so callers never change;
    - `source` of `{ files: File[] }` (Ctrl+V, where the browser supplies bytes, not paths) → the existing upload + `attachments.copy-to-workdir`, kept (`AgentFooter.tsx:555-563`, `attachment-draft.ts:244-253`).
  - The three routes in §2.1 become **one helper with two transports**, so results, mentions and notices are written once.
  - **The helper doesn't read any setting.** Callers pass `mention` and `concurrency`:
    - the file-drop hooks apply `dnd:enabled`, `dnd:agentinserttoken` and `dnd:concurrency` (drop settings, per the schema);
    - both paste routes keep today's behaviour: always mention, default concurrency, independent of `dnd:*`.
- `notifyDrop.{noCwd, noPaths, copied, copyFailed, attachFailed}`: one builder per message, replacing the 13 literals, with consistent expirations.

**Pane hooks in phase 1:**

| Pane | `accept` | `drop` |
|---|---|---|
| Agent (`view: "agent"`) | "Drop N files to attach" (tray) · "Copy N files to <cwd>" (container agent or attachments off) · blocked: `dnd:enabled` off; **in copy mode only**, no cwd (the tray needs no working folder, as today: `useAgentDropAttach.ts:134-145` rejects a missing cwd only when `!toTray()`) | **with paths**, tray: `attachmentDraft.ingestPaths(paths)`, then `copyIntoWorkdir` on the paths it hands back (`non_images`; always empty today, but an older images-only backend returns them, `useAgentDropAttach.ts:159-189`) · **with paths**, copy mode: `copyIntoWorkdir(blockId, { paths })` · **without paths** (virtual files, no `nativeFileDrop`), tray: `attachmentDraft.uploadFiles(files)` · **without paths**, copy mode: `copyIntoWorkdir(blockId, { files })`, the upload + `attachments.copy-to-workdir` transport |
| Terminal (`view: "term"`) | "Copy N files to <cwd>" · blocked: no cwd / setting off | `copyIntoWorkdir` (no mention) |

**Later panes:**

| Pane | `accept` | `drop` |
|---|---|---|
| Media (`view: "media"`) | ok when `count === 1` and the type (MIME, else the name's extension) is image, video or audio ("Open here"); blocked otherwise, e.g. "Media panes open one image, video or audio file" | **with a path:** point the pane at it (today's `/agentmux/stream-local-file` route, which also live-updates) · **without:** show the `File` from an object URL (`URL.createObjectURL`; no live updates), revoked when the pane changes source |
| Editor (`view: "editor"`) | ok when every name has a text extension, or the MIME is `text/*` or a known text type ("Open N files"); blocked when any is a known binary; unknown types answer ok, and `drop` sniffs the content | **with paths:** open each path as an editor tab (today's `readeditorfile`, save goes back to disk) · **without:** open an untitled tab with `await file.text()`, labelled "(not on disk)" so a save asks where to write |

**The drone canvas** checks its own MIME type before accepting (`drone-view.tsx:288`). It then stops advertising "copy" for OS files; the hub's guard handles them.

### 5.4 Floaters and the path stash (CEF)

- **Give floater clients the drag handler.** Split `is_browser_pane` so "no context menu / no drag handler" applies only to real browser panes, and set a `drag_capture` flag for floater and pane-pool clients (`floating_pane.rs:246-250`, `:468-472`). `client/navigation.rs:374-383` notes that the flag is inherited when clients are cloned; keep that behaviour for browser panes.
- **Key the stash by window label**, so `take()` returns only paths from a drag that entered *this* window. The key has to be something both ends know:
  - `consume_drag_paths` is served by the process-wide IPC router with no CEF browser context, and the renderer never sees the native browser id;
  - `on_drag_enter` maps its browser to a label with `window_label_for` (`client/mod.rs:243`) and calls `put(label, paths)`;
  - the frontend passes its own label, `currentWindowLabel()` from `?windowLabel=` (`pane-overlay.ts`), as an argument: `consume_drag_paths {windowLabel}` → `take(label)`.

  A browser the host can't map yet (a window mid-registration) stores under an unlabelled slot, which `take` falls back to.
- **The stash lives as long as the drag.** Today's 5 s TTL (`drag_stash.rs:12-22`, `:35-38`) expires under a user who hovers longer before dropping, so the drop then reports "couldn't read the paths" mid-drag. The entry is kept until one of:
  - it is consumed;
  - a new drag entering that window replaces it (`on_drag_enter`);
  - the 10 min backstop below.

  **The renderer never clears it**, not on leave, Esc or the files-idle watchdog:
  - the watchdog fires on a pause in drag events, which also happens over a native child surface or during a renderer stall while the OS drag is still live, so clearing then would lose the only copy of the paths;
  - a stale entry is harmless, because the next drag that enters the window replaces it before any drop can consume it.

  - **Every drag entry replaces the slot, even with zero paths.** Today `on_drag_enter` stashes only `if !paths.is_empty()` (`client/handlers.rs:162-178`), so a pathless drag (virtual or browser-originated files) leaves the previous drag's paths in place, and its drop would ingest or copy the wrong file. Now an empty list writes an empty entry (a tombstone) for that label and also clears the unlabelled fallback.
  - **Defence in depth at drop:** the controller checks the stashed paths' base names against the drop event's `FileList` names. On a mismatch it ignores the paths and dispatches the bytes (§5.3 step 3).
  - Regression test: a cancelled path drag, then a pathless drop, must dispatch the new drag's `FileList` and never the old paths.

  A long backstop TTL (10 min) only guards against a leak. `peek_drag_paths` never shortens it. The single process-wide slot (`drag_stash.rs:24`) otherwise goes away.

### 5.5 One "copy into a folder"

- **One Rust implementation**, in `agentmux-common`: `copy_into_dir(src, dir, name) -> PathBuf`. It covers:
  - `create_new` de-conflicting (`name_1.ext`, with the `dot > 0` guard);
  - a **copy naming rule** that preserves valid source names, dotfiles included. It replaces path separators, control characters and, on Windows, `: * ? " < > |` with `_`, and on Windows trims trailing dots and spaces. It suffixes Windows reserved names (`CON`, `NUL`, …) and **keeps a leading dot**. The same rule applies to `copy_original_to`'s request-supplied name, so a name that today passes through `safe_file_name` can't reach `create_new` invalid. It is **not** the attachment store's `safe_file_name`, which strips leading dots (`store.rs:934`) and would turn `.env` into `env`. That function stays as is for the store's own `named/` copies. Test: dropping `.env` twice gives `.env` and `.env_1`;
  - streaming with progress and cancel;
  - recursive directories.

  srv's `copy_original_to` and CEF's `copy_file_to_dir` both call it.
- **The channel stays host-only.** An arbitrary source-path + destination copy must **not** become a srv RPC. srv's full-auth surface is also reachable by agent processes, which are given `AGENTMUX_AUTH_KEY` (`agent_handlers/input.rs:548-564`), and the container exec denylist (`container.rs:231-245`) doesn't remove it. A caller-supplied `{paths, dir}` RPC would let an agent copy host files into its own working folder.
  - The copy therefore stays behind CEF's renderer-only IPC (`copy_file_to_dir`, `ipc.rs:434`), and only its implementation changes: it calls `agentmux_common::copy_into_dir` inside `spawn_blocking`, instead of the racy, synchronous `providers.rs:161-263`.
  - The one implementation is shared by crate, not by moving the call to srv.
- **Settings:** `dnd:maxfilesizemb` is either enforced in the CEF copy or removed from the settings template. Decide in phase 2 (open question 4).

### 5.6 Cross-window drags

- **One `CrossWindowDragMonitor.tsx`** with a small platform adapter:
  - `listen(onEnd)`: win32 OLE `dragleave`/`dragenter` plus the button-state fallback; or darwin/linux `dragend`;
  - `cursorPoint(e)`: physical pixels vs DIP;
  - `tabAnchor(...)`.

  This follows `pane-tab-tearoff.ts`. The floater-open retry becomes that module's `openFloatingPaneWindow`, used by every tear-off.
- **Removed:**
  - the unused `listWindows()` call;
  - `performCrossWindowDrop`;
  - the duplicated `DragItemPayload`, now in `drag-session.ts`.
- **One hit-test, made truly Z-ordered first.** The resolver is Z-ordered only on Windows today; elsewhere it picks the lexicographically smallest overlapping label (§2.3). Reusing it as-is would drop into a window hidden behind another. Phase 5 therefore:
  **Phase boundary:** nothing uses `resolve_window_at_cursor` for drag targeting before phase 5, and phases 1–4 don't change cross-window hit-testing at all. The file-drop hit-test in §5.3 is a DOM check inside one renderer (`closest('[data-role="pane"]')`) and never asks the host which window is under the cursor. The steps below all belong to phase 5.

  1. **Makes `resolve_window_at_cursor` stack-aware on every platform:**
     - macOS: front-to-back order from `CGWindowListCopyWindowInfo`, already used by the tear-off hook's macOS module;
     - Linux X11: `_NET_CLIENT_LIST_STACKING`;
     - **native Wayland** (the default when `WAYLAND_DISPLAY` is set, `agentmux-cef/src/app/mod.rs:667-708`): **no cross-window target lookup at all.** Wayland withholds both global cursor coordinates and absolute window positions (`SPEC_TAB_TEAROFF_NATIVE_DRAG_LOOP_2026-05-07.md:119`). Focus order could rank windows but can't tell which one is under the cursor. So the resolver returns "unknown" there, and phase 5 keeps today's native-Wayland behaviour unchanged: no cross-window drop target, and a release outside the window does what it does today. Under XWayland (`--ozone-platform=x11`), the X11 stacking path applies. This is an explicit limitation; a compositor-supported signal can lift it later.
  2. Adds a test with two overlapping windows, where the front one must win.
  3. Only then points `update_cross_drag` at it, and deletes `hit_test_windows` (`drag.rs:193-222`). The frontend `isInsideWindow` (`pane-tab-tearoff.ts:48`, used at `:73-75`) is deleted **only where the resolver can identify the source window**. It stays on native Wayland, where the resolver returns "unknown". Without it, releasing a pane tab over empty space inside its own window would look like a release outside every window and tear the pane off. §5.8 gets a row for this: a native-Wayland release inside the source window must not tear off. macOS/Linux gain target detection.

  The tear-off hook's `WindowFromPoint` path stays: it runs inside a low-level mouse hook, where the resolver's locking isn't safe. That exception is documented in place.
- **One target-side committer, keeping every route.** Today the two host events cover different drops:
  - `cross-drag-end` → `DragOverlay.tsx:77-123` is the **only** path for a pane dropped on another window (`MoveBlockToTab`) and for a tab dropped on window content (`MoveTabToWorkspace`);
  - `tabdrag:merge-direct` (`tab-tearoff-events.ts:231`) handles only a tab released over the tab strip. It rejects coordinates outside the strip (`:251-255`) and inserts at an index.

  So neither can simply be made the only committer. Both event handlers instead call one `commitCrossWindowDrop(drop)` in `app/drag/cross-window-commit.ts`, which routes by kind and position:
  - a pane → `MoveBlockToTab`;
  - a tab over the strip → an insert at the index, **except a source window's last tab**, which keeps today's branches (`tab-tearoff-events.ts:266-296`):
    - from a secondary window: `RestoreTornOffTab`, then close the emptied source window;
    - from `main`: declined, because the main window is never closed.
  - a tab over content → `MoveTabToWorkspace`.

  - **De-duplication needs a drag id that exists before the native mouse-up.** `tabdrag:merge-direct` is emitted synchronously by the mouse hook at button-up (`tear_off_hook.rs:596-618`). The host cross-drag session is created later, when the renderer's `dragend` monitor calls `startCrossDrag`, which on Windows comes after an extra 50 ms (`CrossWindowDragMonitor.win32.tsx:172-173`, `:245`). Lone-tab drags never call it at all. So:
    - the renderer's `drag-session.begin` mints a `dragId` at drag start;
    - it passes the id to the host when tracking starts: `start_tab_drag_tracking {dragId}` for tabs, and `startCrossDrag {dragId}` for the others;
    - the host stamps it on `tabdrag:merge-direct` and on `cross-drag-end`;
    - `commitCrossWindowDrop` commits each `dragId` at most once.

    This replaces the time window in `wasTabRecentlyMerged` (deleted).
  - **The source side keeps its end work.** When a tab leaves a window, the `cross-drag-end` listener also runs in the **source** renderer and disposes the tab's `LayoutModel` (`deleteLayoutModelForTab`, `DragOverlay.tsx:126-131`). That is source-side cleanup, not a commit. It moves into the session lifecycle, `onSessionEnded({ result: "moved-out" })`, rather than disappearing with the overlay's commit code; otherwise every cross-window tab move would leak the model and its reactive roots (`layoutModelHooks.ts:52-56`).
  - `DragOverlay.tsx` then only draws, and becomes `CrossWindowDropOverlay.tsx` in phase 3.
  - A characterisation test lands for each route before the move: pane → content; tab → strip; tab → content; last tab → strip from a secondary window and from `main`; and source-side model disposal.
- **Dead host surface deleted:**
  - `set_js_drag_active` and its Linux caller;
  - `set_drag_cursor` / `restore_drag_cursor`;
  - `tear_off_sc_move_handshake` and `HookMode::TearOff`, if the maintainers confirm they're shelved for good (open question 3);
  - the stale `stubs.rs` comment.

### 5.7 Tile draggable registration without polling

- **Replace the 100 ms poll** (`TileLayout.core.tsx:522`) with a `MutationObserver` on the tile node (`childList`, `subtree`). It calls the **existing** `register()` unchanged.
- **Why not header `ref` callbacks:** `BlockFrame_Header` exists twice, the live one and an `ErrorBoundary` fallback that is never inserted into the DOM, and the fallback's ref writes last (`TileLayout.core.tsx:420-424`). A ref-driven registrar would bind pragmatic-dnd to a detached element, or let the fallback's cleanup remove the live registration, which would break whole-pane dragging.
- **The observer keeps today's selection logic:**
  - `tileNodeRef.querySelector('[data-role="block-header"]')` picks the connected header this tile owns;
  - `register()` compares identity and re-registers only on change.
- **This keeps the poll's two jobs:** first mount (one `register()` in `onMount`), and re-registration when a `Show` gate replaces the header. It does both when the DOM actually changes, instead of every 100 ms forever. A characterisation test mounts a tile with a detached fallback header and asserts the live header is the registered handle, before and after the change.
- **The "never tear down mid-drag" rule** (`:438`) moves into the registrar, keyed on `session()`.
- **Retry after the drag.** A mutation that arrives mid-drag is skipped by that rule, and when the drag ends there may be no further mutation. So the registrar sets `pendingRegister`, and a session-end subscriber runs `register()` once if it is set. Otherwise a header replaced by a `Show` gate mid-drag would never become draggable.

### 5.8 Behaviours phases 4–5 must preserve

Today's drag code encodes many deliberate edge cases, and a prose spec can't list them all. Phases 4 and 5 therefore start each move by writing **characterisation tests of today's behaviour** for the system being moved. The move passes only when those tests pass unchanged. The table below is the floor, not the ceiling: whoever does the move adds a test for every deliberate special case the old code comments on.

| Behaviour | Today | Test |
|---|---|---|
| A lone-tab drag carries no cross-window payload; only the native strip merge handles it | `droppable-tab.tsx:141-149` | releasing a lone tab outside any strip does nothing and strands no window |
| A last tab dropped on another strip: secondary → `RestoreTornOffTab` + close the source; `main` → declined | `tab-tearoff-events.ts:266-296` | both cases |
| A source `onDrop` for an outside release keeps the payload for the monitor | `TileLayout.core.tsx:504-514`, `droppable-tab.tsx:175-200`, `PaneTabStrip.tsx:624-625` | tear-off still receives the payload |
| No pragmatic registration teardown mid-drag, and the skipped registration is retried when the drag ends | `TileLayout.core.tsx:438` (the poll retried implicitly) | `activeDrag` resets after a drop while the header is replaced mid-drag, and the replacement header is draggable after the drop |
| The live header is registered, never the detached `ErrorBoundary` fallback | `TileLayout.core.tsx:420-424` | a tile with a detached fallback header |
| A swallowed `dragend` on Windows is caught by the button poll | win32 monitor | the session ends and `activeDrag` resets |
| The source renderer disposes the moved tab's `LayoutModel` | `DragOverlay.tsx:126-131` | the model map shrinks after a move-out |
| Merge-direct and cross-drag-end commit once | `wasTabRecentlyMerged` today | both events with the same `dragId` → one commit |
| Tearing off an **inactive** pane-tab pill opens at its source pane's size, not 720×480 | `PaneTabStrip.tsx:641` → `pane-tab-tearoff.ts:154` | the floater size matches the captured `paneSize` |
| On native Wayland, releasing a pane tab inside its own window doesn't tear it off | `pane-tab-tearoff.ts:48`, `:73-75` (`isInsideWindow`) | a release inside the source window with the resolver returning "unknown" |
| Escape aborts a tab, pane-tab or tile drag | `tab-reorder.ts:148-154`, `PaneTabStrip.tsx:626-649`, `tab-tearoff-events.ts:218` | each kind |

## 6. Efficiency, before and after

| | Today | After |
|---|---|---|
| App-level window listeners run per `dragover` | N+4 (N = mounted window tabs) | 1 (+ pragmatic's own 2) |
| Idle DOM queries per second | 10 × panes across all tabs | 0 |
| IPC per cross-window drag | +1 unused `listWindows` | 0 wasted |
| Stores meaning "a drag is in flight" | ~10 | 1 session (+ `activeDrag`, derived) |
| `dragend` safety nets | 4 | 1 |
| Cross-window monitor code | 3 files, ~140–175 shared lines per pair | 1 module + 3 small adapters |
| Window hit-tests (Rust + frontend) | 5 (one Z-ordered on Windows only) | 2, both Z-aware (general + the mouse-hook exception) |
| Rust "copy into folder" | 2 | 1 |
| Routes into a container working folder | 3 separate implementations | 1 helper, 2 transports (paths, bytes) |
| Drop/copy notice literals | 13 | 5 builders |
| Drop colour sources | 3 (+2 hard-coded greens) | 1 token set |

## 7. Phases

Each phase is one PR, or a short stack, and is independently shippable.

1. **Phase 1: file drops**
   - **Build:**
     - `drag-session` for `"files"` only;
     - `file-drop.ts` and its per-block hook registry;
     - `DropIndicator` + `drop-indicators.scss`;
     - `file-drop-actions.ts`, whose `copyIntoWorkdir` paths transport is the **existing CEF `copy_file_to_dir`** in this phase (the same host-only IPC every phase uses);
     - the agent and terminal hooks;
     - the drone canvas type check;
     - the floater drag handler and the label-keyed stash (§5.4).
   - **Delete:** `element/dragoverlay.tsx` and the per-pane drop listeners.
   - **User-visible:** the indicator; terminals get feedback; floaters accept drops; header drops work.
   - The hub (§5.2) can start as just the file subscriber, keeping `installGlobalDropGuard` until phase 4.
2. **Phase 2: copy and paste**
   - one Rust copy, `agentmux_common::copy_into_dir`, behind the existing host-only CEF IPC and in srv's `copy_original_to` (§5.5);
   - no transport change for callers: `copyIntoWorkdir` keeps the CEF IPC, and only the code behind it is replaced;
   - both container paste routes on `copyIntoWorkdir` (paths and bytes transports; paste keeps ignoring `dnd:*`);
   - CEF's old `copy_recursive` / `deconflict_path` deleted, replaced by the shared function;
   - the `dnd:maxfilesizemb` decision.
3. **Phase 3: one look**
   - every internal drop treatment in §2.4 moves onto the §4 tokens;
   - the hard-coded greens go;
   - keyframes are emitted once;
   - `app/drag/DragOverlay.tsx` is renamed `CrossWindowDropOverlay.tsx`.
4. **Phase 4: session and hub**
   - `drag-session.ts` takes over tile, window-tab, pane-tab, drone and list drags;
   - `window-drag-events.ts` replaces the window listeners and the global guard;
   - one safety net;
   - `drag-types.ts`;
   - no polling (§5.7);
   - `dragInFlight.ts` and `element-drag-state.ts` deleted.

   This is the 2026-07-11 drag-session refactor, done incrementally: one drag system per commit, each behind its own characterisation tests.
5. **Phase 5: cross-window**
   - the single monitor + adapters;
   - one hit-test;
   - one commit path;
   - the dead host surface removed (§5.6).

Phases 2–5 have no user-visible features. They're worth doing because every future drag feature (media and editor drops, drag-to-reorder attachment tiles) otherwise adds another copy of each pattern.

## 8. Tests

- **Phase 1:**
  - **Controller unit tests** (jsdom, synthetic `DragEvent`s): Armed / Target / Blocked / none by pane; `accept` called once per visible pane at session start (so every valid pane is Armed before being hovered) and once more when names arrive; clear on each end reason including the watchdog; drop dispatch with stashed paths; empty stash → one notice; internal drags ignored; **a source-side `onDrop` for a release outside the window leaves the session `released` for the cross-window monitor, and an internal session held idle for 10 s is not ended**; **a drop released over a blocked pane or chrome within the same frame as leaving a Target dispatches nothing and leaves the stash unconsumed**; `accept` receives MIME types, and names after `peek_drag_paths` resolves.
  - **Per-pane `accept` tables** for agent and terminal.
  - **`DropIndicator`** renders each state; the reactivity bug in §2.1 gets a regression test.
  - **`copyIntoWorkdir`:** mentions, partial failure, notices.
  - **CEF:** the stash keyed by window label (a put from window A isn't taken by window B; the unlabelled fallback), and floater clients getting a drag handler.
- **Phase 2:** Rust `copy_into_dir`: de-conflicting under a race (two threads), `.env`, directories, cancel.
- **Phase 4, before each system moves:** characterisation tests of today's behaviour — tile move, window-tab reorder and tear-off, pane-tab reorder and tear-off, Escape abort, swallowed `dragend`. After it moves, the same tests pass unchanged.
- **Phase 5:** hit-test parity on Windows; overlapping-window tests (front window wins) for the stack-aware resolver on each platform; one characterisation test per cross-window commit route (pane → content, tab → strip, tab → content), run before and after the committer moves.
- **Manual on Windows each phase:** main window, floater, two windows, Esc mid-drag, drag across a browser pane, window tabs mounted in the background.

## 9. Risks

- **Drag code has broken often** (see the retros and the 07-11 / 07-13 specs). Phases 4–5 touch those paths. Mitigations:
  - characterisation tests first;
  - one drag system per commit;
  - the per-platform adapters stay where the platforms genuinely differ (win32 OLE `dragleave`, DIP vs physical pixels).
- **Coalescing:** only visuals are batched per animation frame. `preventDefault` / `dropEffect` and the `drop` authorisation are computed synchronously from the pane actually under the pointer on every event (§5.2, §5.3). A test covers releasing over a blocked pane within the same frame as leaving a Target: nothing is dispatched.
- **Host file access through the shared auth key (found while reviewing this spec, outside its scope).**
  - Agents get the full `AGENTMUX_AUTH_KEY`, and container agents keep it (§5.5).
  - With it, existing routes already read arbitrary host paths:
    - `/agentmux/stream-local-file` (`server/files.rs:199`, the media pane);
    - `readeditorfile`;
    - `attachments.ingest {paths}` together with `attachments.copy-to-workdir` or the attachment HTTP route.
  - If a container can reach srv, the container boundary therefore does not protect host files today.

  This spec adds no new route of that kind (§5.5 keeps the copy host-only). The existing ones need their own spec: an agent-scoped key without file-path RPCs, or a host-minted capability token per operation. That is tracked as a follow-up, not here.

## 10. Open questions

1. **Armed outline on panes in other window tabs?** They aren't visible, so no. Only the visible layout is armed.
2. **Blocked panes:** show the prompt with the reason (recommended), or nothing?
3. **SC_MOVE / `HookMode::TearOff`:** is it shelved for good? If yes, phase 5 deletes it; if not, it stays with a comment.
4. **`dnd:maxfilesizemb`:** enforce a per-file cap in the host-only CEF copy, or drop the setting? Recommendation: drop it. The attachment store already has its own limits, and a copy into a working folder is the user's explicit act.
