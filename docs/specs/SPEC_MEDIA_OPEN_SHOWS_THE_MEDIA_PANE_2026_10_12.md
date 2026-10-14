# Spec: Clicking an image shows the Media pane it lands in

**Date:** 2026-10-12
**Author:** Maricon@charlie
**Type:** Spec (one small change, one shared helper that already exists)
**Status:** implemented (#4852)
**Decisions applied:** D1 no focus move, D2 prefer a visible pane, D3 also find a stack's background Media tab; D4 (the agent's `OpenMedia`) stays out of scope.
**Requested by:** the operator: *"when clicking an image that would normally open an image pane, if an image pane already exists but is minimized, restore the pane so it is viewable and load the image as another document tab. I notice the images are already loaded properly as a document tab but the pane remains minimized, so the user doesn't realize the image was actually loaded into the minimized pane."*
**Related:** `SPEC_EDITOR_REUSE_RESTORES_MINIMIZED_PANE_2026_09_30.md` (the same fix for the Editor, shipped in #4063; this spec reuses its helper), `SPEC_DOCUMENT_TABS_2026_10_02.md` §5.7 (opening media as a tab of the Media pane on screen), `SPEC_MEDIA_PANE_2026_07_26.md`.

## 1. The problem

When the user clicks an image (or a video or audio file), the app adds it as a document tab of a Media pane that is already on screen, instead of opening a new pane. That part works. But if that Media pane is **minimized**, the tab is added inside a pane that is only a header chip. Nothing visible happens, so the user thinks the click did nothing, and finds the image later only by expanding the chip by hand.

This is the same failure the Editor had before #4063, on the user-click path rather than the agent path.

## 2. How it works today

All user clicks on a media file go through one function, `openInMediaPaneOnScreen(path)` (`frontend/app/view/media/media-open.ts`):

1. `mediaPaneOnScreen()` picks the Media pane in the window tab on screen: **the focused pane if it is a Media pane, else the first Media pane in the layout's leaf order.** It does not look at whether that pane is minimized, hidden behind a maximized pane, or a background tab of a pane stack.
2. It appends `{ id, path }` to that block's `media:open` meta (a queue). The pane's effect (`media-pane.tsx`) drains the queue and calls `pane.open(path)`, which adds the file as a document tab and shows it.
3. It returns `true`, and the caller does nothing more. If there is no Media pane it returns `false`, and the caller opens a new one.

Callers (all user clicks):

| Click | Code |
|---|---|
| An image, video or audio card in an agent's reply, or a markdown image | `openInMediaPane` in `frontend/app/element/markdown-media.tsx` |
| An image in a tool result | `ResultImages.tsx`, through `openInMediaPane` |
| Opening a media file from the Files pane | `openInPane` in `frontend/app/view/files/files-open.ts` |

Nothing in this path changes the pane's visibility. The agent's `OpenMedia` tool is a different path: it always opens a **new** Media pane (`/api/v1/pane/open`), so a minimized pane is not involved, and it is unchanged by this spec.

## 3. The behaviour

When a click adds a file to a Media pane, that pane is made visible **in its tab**, exactly as `OpenEditor` does for the Editor:

- **If the pane is minimized, it is restored** to the size it had before it was minimized.
- **If it is a background tab of a multi-tab pane (a pane stack), that pane switches to the Media pane.**
- **If another pane in the layout is maximized, that pane is un-maximized**, because a maximized pane hides every other tile.

The new file is the Media pane's active document tab, as today.

What it deliberately does **not** do (the same limits as the Editor, for the same reasons):

- **No window-tab switch.** The pane being "on screen" already means it is in the window tab the user is looking at.
- **No layout focus and no keyboard caret move.** The user clicked somewhere else on purpose (decision D1 below asks whether the Media pane should take focus).
- **An already-visible pane is untouched.** Its size, position and other tabs do not change.
- **The user hiding it again afterwards sticks**, until the next click that adds a file.

### 3.1 Choosing among several Media panes

`mediaPaneOnScreen()` should prefer a pane the user can already see, and fall back to one they cannot:

1. the focused pane, if it is a Media pane (as today);
2. else the first Media pane that is **visible**: not minimized, and the front tab of its stack;
3. else the first Media pane of any kind, which the reveal then restores.

Today's step 3 is its only step after the focused check, so with one expanded and one minimized Media pane, a click can pick the minimized one and leave the visible one alone. With this order it picks the visible one, and nothing is restored.

## 4. Design

The helper already exists. `showBlockWithoutFocus(blockId)` in `frontend/app/util/reveal-block.ts` finds the block's tab in this window and calls `showBlockInPane`, which restores a minimized pane, un-maximizes another pane, and switches a stack to the block, without moving focus. The Editor's model calls it when it processes an agent's open; the agent pane's "open in editor" button calls it too.

**Where to call it: in `openInMediaPaneOnScreen`, right after the open request is written**, so that the three callers above all get it from one place:

```ts
export async function openInMediaPaneOnScreen(path: string): Promise<boolean> {
    const blockId = mediaPaneOnScreen();
    if (!blockId) return false;
    // …append { id, path } to media:open, as today…
    await setBlockMeta(blockId, { [META_OPEN]: [...list, request] } as MetaType);
    // Show the pane the file landed in (restore if minimized, un-maximize another pane, switch a stack
    // to it) without moving focus or the window tab. Awaited so the tab is shown when the click resolves.
    await showBlockWithoutFocus(blockId);
    return true;
}
```

Why here and not in the Media pane's queue consumer (as the Editor does):

- The Editor shows its pane from the consumer because its opens also arrive from agents through the backend, which has no other place to do it. Every Media open that this spec covers starts as a click in this renderer, through this one function.
- A consumer-side call would also fire when the queue is drained after a reload or a window re-creation, restoring a pane the user had minimized on purpose. Calling it at the click does not.

If `showBlockWithoutFocus` returns false (the block is not in a built layout in this window), the file is still queued and the click still counts as handled, as today.

### 4.1 `mediaPaneOnScreen()`

Replace the second loop with the order in §3.1, using the same checks the layout already exposes: a leaf's `minimized` flag, and `activeBlockId` for a stack. It also gains a small fix: it should consider the blocks in a stack, not only `leaf.data.blockId`, so a Media pane that is a background tab of a stack is found instead of a second Media pane being opened. That is a behaviour change on its own (decision D3).

### 4.2 Nothing else changes

- No new meta key, no backend change, no change to what `media:open` carries or how the pane drains it.
- `OpenMedia` (agent) is unchanged.
- The helper's own behaviour is already tested (`reveal-block.test.ts`); this spec adds tests at the call site.

## 5. Tests

1. **Minimized:** with a Media pane that is minimized, `openInMediaPaneOnScreen(path)` returns true, queues the request, and the pane is no longer minimized (`model.minimizedNodeIds()` does not hold it).
2. **Visible pane untouched:** with an expanded Media pane, the request is queued and `minimizeNodeToggle` is not called; sizes and the other tabs are unchanged.
3. **Maximized neighbour:** with another pane maximized, the maximize is ended and the Media pane is visible.
4. **Stack member:** with the Media pane a background tab of a stack, the stack switches to it.
5. **Selection order (§3.1):** with one expanded and one minimized Media pane and neither focused, the expanded one is chosen and the minimized one stays minimized; with only a minimized one, it is chosen and restored; with the minimized one focused, it is chosen and restored.
6. **No pane:** returns false, and no layout change.
7. **Focus:** no focus request and no window-tab change in any of the above.
8. **Callers:** the markdown image card, `ResultImages` and the Files pane each reach the same function (a spy on `showBlockWithoutFocus` per caller, or one test through each).

Live check on a real build: minimize the Media pane, click an image in an agent's reply; the pane expands with the image as its front tab and the agent pane keeps the caret. Repeat with a video, with the Files pane's "open", with the Media pane in a stack, and with another pane maximized.

## 6. Decisions for the owner

| | Question | Recommended |
|---|---|---|
| D1 | After the click, should layout focus move to the Media pane? | no, as for the Editor: the user may be reading or typing in the pane they clicked from |
| D2 | When several Media panes exist, prefer a visible one over a minimized one (§3.1)? | yes |
| D3 | Also find a Media pane that is a background tab of a stack (§4.1), instead of opening a new pane? | yes, but it is a separate behaviour change; say if you want it left out |
| D4 | Should the agent's `OpenMedia` reuse an existing Media pane too (it always opens a new one now)? | out of scope here |

## 7. What this spec does not cover

- The agent's `OpenMedia` path (always a new pane).
- Media on a remote connection (`openInPane` already refuses it).
- Any change to how the Media pane loads and shows a file.
- I did not run the app for this spec: the call path and its callers come from reading the code, and the first thing to confirm when implementing is that `showBlockWithoutFocus` resolves the block's tab for a pane in the tab on screen without a window-tab switch.
