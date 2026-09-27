# SPEC: Highlight the pane a dragged file will land in

**Date:** 2026-09-27
**Status:** proposed — nothing in this spec is implemented. Written against `main` @ `ade4ea16f`; file:line citations are from that commit.
**Author:** Korp@narko
**Related:** `SPEC_PANE_FILE_DROP_2026_05_30.md` (OS file drop into Terminal and Agent panes: the CEF drag handler, the path stash, copy to the working folder), `SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md` and `SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md` (what an agent pane does with a drop), `docs/retro/retro-md-drop-window-hijack-and-55-6-relaunch-failure-2026-08-16.md` (why a window-level drop guard exists), `SPEC_PANE_TAB_CONTRACT_V1` (the pane manifest this extends).

---

## 0. The ask

> when dragging any file over a pane, we want to highlight the pane a special indication, like a very thick border (to differentiate from the other borders) when dragging over the agentmux window. for now only agent panes will be supported, but we will have future pane support too, like media and editor.

## 1. Today

- **There is no visible drop feedback at all.** Agent and Terminal panes render `<DragOverlay visible={…}>` (`agent-view.tsx:2552`, `term.tsx:452`). The component destructures its props, `({ message, visible }: DragOverlayProps)` (`frontend/app/element/dragoverlay.tsx:11`). In SolidJS that reads `visible` once, at creation, when it is `false`. The overlay never appears. Nothing else marks the pane either.
- **Each pane listens for itself.**
  - The agent pane listens on `.agent-view` (`useAgentDropAttach.ts:104-126`, `:261-268`).
  - The terminal pane listens on `.view-term` (`term.tsx:374-439`).
  - Both cover only the pane's content. A drop on the pane header or its tab strip isn't handled.
- **Detecting a file drag.** `isFileDrag(e)` checks `dataTransfer.types` for `"Files"` (`frontend/util/dnd.ts:87-90`). Internal drags carry no `"Files"`, so the check can't misfire on them. Pane moves, pane tabs and window tabs all use pragmatic-dnd (`setData("application/vnd.pdnd")`), and the drone canvas uses `application/x-drone-kind`.
- **Paths.** CEF's drag handler stashes the OS paths on drag-enter (`agentmux-cef/src/client/handlers.rs:162-178`, `drag_stash.rs`). The pane fetches them on drop with `consumeDragPaths()` (`dnd.ts:29-36`).
- **The window guard.** `installGlobalDropGuard()` (`frontend/app-init.ts:1029-1036`) prevents the default action for any file drag that no pane handles, so a stray drop can't navigate the window. Every window, including floaters and tear-offs, runs it.
- **Pane chrome.** Every pane's root is `div.pane-stack[data-role="pane"][data-blockid]` (`frontend/app/element/PaneChrome.tsx:390-397`). Its ring is `.pane-stack::after` with a `2px solid` border, in `--border-color` or the focused `--accent-color` (`PaneChrome.scss:42-56`). The ring is hidden when the pane is alone in its tab (`:68-70`).
- **Two gaps the highlight would expose:**
  - **Windows floating panes can't take a drop.** They are built with `is_browser_pane = true` (`agentmux-cef/src/floating_pane.rs:246-250`, `:468-472`), and `drag_handler()` returns `None` for such clients (`handlers.rs:57-60`). Paths are never stashed, and the drop fails with "Couldn't read the OS paths".
  - **Native browser panes paint above the renderer.** While the cursor is over one, the renderer gets no drag events (`frontend/app/drag/element-drag-state.ts:8-13`).

## 2. Goals

1. While OS files are dragged over an AgentMux window, the pane the drop would land in gets a **very thick border**. It must be distinct from every other pane border: the 2px ring, the focused ring, and the agent-colour ring.
2. A short line in the pane says what the drop will do ("Drop 3 files to attach", "Copy to C:\work").
3. Panes that can't take the drop show nothing. The cursor shows no-drop.
4. The highlight never promises a drop that will fail.
5. **Phase 1: agent panes only.** Other pane types opt in later by declaring support in their manifest, with no new plumbing. Media and editor panes are next (§9).

## 3. Non-goals

- Changing what an agent pane does with a drop. Attachments and copy to the working folder stay as they are.
- Internal drags (moving a pane, a pane tab or a window tab). They keep their own feedback (`drop-feedback.scss`, `tilelayout.scss`).
- Dropping text or URLs.
- Dropping onto native browser panes (§8).

## 4. Behaviour

| Where the cursor is, during an OS file drag | Pane under the cursor | Other accepting panes in the window | Cursor |
|---|---|---|---|
| Over a pane that accepts, and the drop would work | **Target**: thick border + message | **Armed**: faint dashed ring | copy |
| Over a pane that accepts, but the drop would fail right now (e.g. attachments off and no working folder) | **Blocked**: message with the reason, no thick border | Armed | no-drop |
| Over a pane that doesn't accept files, or over window chrome | nothing | Armed | no-drop |
| Outside the window, or the drag ended or was cancelled | everything cleared | cleared | — |

- **The target is the whole pane**, `.pane-stack`: header, pane tab strip and content. A drop anywhere in it goes to that pane's **active** pane tab (`data-blockid` is the active block, `PaneChrome.tsx:397`).
- **"Armed"** tells you where you *could* drop before you get there. It is deliberately quiet, so the thick border stays the one strong signal.
- **The message** uses the file count from `dataTransfer.items.length`, which Chromium exposes during `dragover`. Names aren't available until the drop.

## 5. Look

The thick border comes from the pane's existing ring pseudo-element, so nothing shifts or reflows:

```scss
.pane-stack[data-file-drop="target"]::after {
    border: var(--file-drop-border-width, 4px) solid var(--file-drop-color, var(--accent-color));
    box-shadow: inset 0 0 0 1px rgb(0 0 0 / 35%), inset 0 0 18px rgba(var(--accent-color-rgb), 0.35);
    z-index: var(--zindex-file-drop, 60); /* above the focused ring (50) */
}
.pane-stack[data-file-drop="armed"]::after {
    border: 2px dashed rgba(var(--accent-color-rgb), 0.55);
}
```

- **How it stands apart from other borders:**
  - it is at least **2× thicker** (4px against 2px);
  - an inner glow makes it read as a fill edge, not an outline;
  - it shows even when the pane is alone in its tab, where the normal ring is transparent (`PaneChrome.scss:68-70`).
- **Theme variables:**
  - `--file-drop-color` and `--file-drop-border-width` let themes restyle it;
  - the default colour is the theme's `--accent-color`, not Tailwind's fixed `--color-accent`, which the old overlay used and themes don't override.
- **The message chip** is centred in the pane content. It reuses the old overlay's look: a dark scrim chip that works in every theme. There's no full-pane scrim, so the pane stays readable under the cursor. It has `pointer-events: none`.
- **Motion:** the border fades in over 120 ms. Under `prefers-reduced-motion` it appears instantly. There's no pulsing: the drag can last seconds, and a pulse would pull the eye.
- The width is a variable because "very thick" is a judgement call. Tune it once in a dev build.

## 6. Design

### 6.1 One controller per window

`frontend/app/drag/file-drop-targets.ts` installs capture-phase `dragenter`, `dragover`, `dragleave` and `drop` listeners on `window`. It runs once per renderer, from `initAppInner` next to the drop guard. Floaters and tear-off windows are separate renderers, each with its own state.

- **Ignore anything that isn't a file drag**, or that happens while an internal drag is in flight (`elementDragInFlight()`, `layoutModel.activeDrag`).
- **On dragover, hit-test the event target:**
  - find `e.target.closest('[data-role="pane"]')`;
  - read `data-blockid`;
  - look up the block's view, resolved through `resolvePaneTabView` (`pane-tab-registry.ts:189-196`);
  - look up the instance's file-drop hook (§6.2).

  Signals update only when the answer changes. `dragover` fires about every 50 ms, and a no-op tick must cost only a `closest()` and one comparison.
- **Set `dataTransfer.dropEffect`:** `copy` for a target, `none` otherwise. The window guard keeps calling `preventDefault` so nothing navigates.
- **Clear everything** on any of these:
  - `drop`;
  - `dragend`;
  - a `dragleave` whose `relatedTarget` is `null` (the drag left the window);
  - a **watchdog**: no `dragover` for 350 ms.

  The watchdog covers the cases where Chromium sends no reliable leave: a cancelled drag with Esc, a drop in another app, and a cursor that crossed into a native browser pane (§1).
- **On drop over a target:**
  - `preventDefault()` and `stopPropagation()`, so the pane's old listener can't also fire during migration;
  - `consumeDragPaths()`;
  - call the pane's `drop(paths, files)`.

  One place handles the drop. Panes no longer each listen, hit-test and track leaves.

### 6.2 How a pane opts in

Opting in takes two parts: a static capability on the manifest, and an instance hook returned from `create()`. The hook is per instance because the answer depends on that pane's state: its working folder, its settings, whether it's a container agent.

```ts
// PaneTabManifest.capabilities
fileDrop?: true;

// returned from manifest.create(ctx)
fileDrop?: {
    /** Asked on every target change (not every dragover). Cheap and synchronous. */
    accept(drag: { count: number }): { ok: true; message: string } | { ok: false; reason: string };
    drop(paths: string[], files: FileList): void | Promise<void>;
};
```

- **Armed** marks every pane in the window whose manifest has `fileDrop`, without calling `accept`.
- **Target or Blocked** comes from `accept` for the pane under the cursor.
- **The state reaches the DOM** as a `data-file-drop` attribute on that pane's `.pane-stack`. `PaneChrome` reads it from a per-window signal keyed by block id. Nothing outside the pane's own chrome renders the highlight.

### 6.3 Agent panes (phase 1)

`useAgentDropAttach` becomes the agent pane's `fileDrop` hook:

- **`accept`** returns one of:
  - `"Drop N files to attach"` when the tray takes files (attachments on, not a container agent);
  - `"Copy N files to <cwd>"` when the pane copies into its working folder (a container agent, or attachments off);
  - `{ ok: false, reason: "No working folder for this agent" }` when it would copy but has no `cmd:cwd`;
  - `{ ok: false, reason: "File drop is turned off (dnd:enabled)" }` when the setting is off.

  These branches are the same ones `onDrop` takes today (`useAgentDropAttach.ts:135`, `:161-189`).
- **`drop`** is today's `onDrop` body, from the `consumeDragPaths()` result onward (`:147-258`).
- **Removed:** the pane's own window listeners, `isDragOver`, `dropMessage` and its `<DragOverlay>`.

### 6.4 The broken overlay

Once no pane uses `DragOverlay` (`frontend/app/element/dragoverlay.tsx`), delete it. Until a pane migrates, fix it by reading `props.visible` instead of destructuring, so the terminal pane gets its overlay back in phase 1. That is a one-line change.

### 6.5 Floating panes on Windows

Goal 4 means a floater must not show Target until its drops work. Phase 1 therefore also stops `is_browser_pane` from meaning "no drag handler" for floater clients:

- **Preferred:** add a `drag_capture` flag, separate from `is_browser_pane`, and set it for floater and pane-pool clients (`floating_pane.rs:246-250`, `:468-472`).
- **Fallback:** if that has to wait, floaters report `accept → { ok: false, reason: "Drop into the main window for now" }`.

`client/navigation.rs:374-383` notes that the flag is inherited when clients are cloned; the change must keep that behaviour for real browser panes.

## 7. Edge cases

- **Internal drags** carry no `"Files"` and set `elementDragInFlight()`: ignored twice over.
- **Several pane tabs in one pane:** the drop goes to the active one, the same one the header shows.
- **Zoomed or maximised panes, and panes in a floater:** hit-testing uses the DOM, so no special case is needed.
- **A drag moving between AgentMux windows:** the first window clears on leave or the watchdog, and the second arms on enter. The Rust path stash is one process-wide slot (`drag_stash.rs:24`) that the new window's drag-enter overwrites, which is correct.
- **Dropping during processing:** unchanged. The tray already queues it.
- **Many files (more than 128):** `accept` could say "Drop 200 files (128 max)". It stays `ok`, because the tray already refuses the excess with a notice.
- **The drop lands in a pane that didn't accept:** the window guard swallows it, as today. No toast, since the no-drop cursor already said so.

## 8. Native browser panes

While the cursor is over a native browser pane, the renderer is blind, so nothing is highlighted and the watchdog clears the last target. That matches goal 3, because browser panes don't accept files.

Separately, whether an OS file dropped onto a browser page navigates that pane to `file://` depends on `is_disallowed_pane_nav_scheme` (`client/lifecycle.rs:837-858`). That is worth checking, but it is outside this spec.

## 9. Future panes

Each is one manifest capability plus a `fileDrop` hook. The controller and the look don't change.

| Pane | `accept` | `drop` |
|---|---|---|
| **Media** (`view: "media"`) | ok for one image, video or audio file ("Open in this pane"); blocked for anything else | point the pane at the file |
| **Editor** (`view: "editor"`) | ok for text files ("Open N files"); blocked for binaries | open the file(s) as editor tabs |
| **Terminal** (`view: "term"`) | "Copy N files to <cwd>" | today's copy (`term.tsx:400-429`) |

The terminal pane already accepts drops today but has no visible feedback. Moving it onto the controller is a small change and recommended soon after phase 1 (open question 1).

## 10. Tests

**Controller unit tests** (jsdom, synthetic `DragEvent`s with a stub `dataTransfer`):

- Over an agent pane: that pane becomes Target, the other agent panes become Armed, and `dropEffect` is `copy`.
- Over a sysinfo pane: nothing is Target, the agent panes stay Armed, and `dropEffect` is `none`.
- `accept → ok:false`: Blocked with the reason, no Target, and `dropEffect` is `none`.
- A drag whose types don't include `"Files"`, or one while `elementDragInFlight()` is true: nothing changes.
- `dragleave` with `relatedTarget: null`, `drop`, `dragend` and the 350 ms watchdog each clear all state.
- A drop on a Target calls `drop` once with the stashed paths. A drop on a Blocked or unsupported pane calls nothing.
- The target is recomputed only when the pane under the cursor changes: `accept` is called once for 20 `dragover`s over the same pane.

**Other tests:**

- **PaneChrome:** the `data-file-drop` attribute follows the signal. The CSS is checked in a dev build: 4px, visible in a pane that's alone in its tab, above the focused ring.
- **Agent `accept`:** tray vs. copy vs. no working folder vs. setting off.
- **`DragOverlay`:** a test that `visible` toggling shows and hides it, as a regression test for §6.4.
- **Manual on Windows:**
  - a main-window agent pane;
  - a floating agent pane, after §6.5;
  - two windows side by side;
  - Esc to cancel mid-drag;
  - dragging across a browser pane.

## 11. Rollout

1. **Phase 1:**
   - the controller, the `fileDrop` manifest capability and hook, and the PaneChrome attribute and CSS;
   - the agent pane moved onto the hook;
   - the `DragOverlay` fix (§6.4) and the floater drag handler (§6.5).
2. **Phase 2:** the terminal pane moved onto the hook, and `DragOverlay` deleted.
3. **Later:** media and editor panes (§9), each in its own PR with its own `accept`/`drop`.

## 12. Open questions

1. **Terminal in phase 1?** The ask says agent panes only for now. But terminals already take drops, and with no feedback they'd be the one silent drop target. Recommendation: phase 2, right after phase 1.
2. **Armed state?** It helps you find a drop target before you get there, but some may find it noisy with many panes. Recommendation: keep it, since it's faint and dashed, and drop it if it reads as clutter in the dev-build check.
3. **Border width.** Is 4px "very thick" enough at 100% and 125% scaling, or is 5–6px better? Settle it in the dev build; it's one variable.
