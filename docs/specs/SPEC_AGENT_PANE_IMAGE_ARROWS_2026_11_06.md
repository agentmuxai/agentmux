# Spec: an image opened from an agent message steps through that message's other images with the arrow keys

Task #23512 (spec), build #23513. Author: Agent2-5@narko, 2026-11-06. Written against origin/main `5bc5d2d8`.
Status: draft for the operator's OK. Not committed (a spec ships with the build PR).

## 1. What it is today

Three places show an image in the agent pane, and they behave differently:

| Surface | Code | Click does | Arrows do |
|---|---|---|---|
| An image or video in the **agent's own message** (`![alt](path)`) | `LocalImage` in `frontend/app/element/markdown-media.tsx` | `openMediaInEditor(path)`: a tab of the Editor pane on screen, else a new Editor pane, with "follow folder" on | In the Editor's image viewer arrows **pan** (`image-viewer.tsx:209`); `[` / `]` step through the *folder* (§4.4.2 of SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS), not the message |
| A **user-sent attachment** in the transcript | `AttachmentStrip` → `AttachmentLightbox` (`attachments/`) | a pane-scoped modal | **← / → page through that message's attachments**, wrapping, `n / total` counter, Esc closes |
| An image a **tool returned** (`ResultImages`) | `ResultImages.tsx` | Editor if it has a file, else fit/full toggle | n/a |

So the behaviour the operator wants already exists for what the *user* sends. For what the *agent* shows, a click leaves the pane for the Editor, and the Editor knows nothing about "the other images in that message".

## 2. Recommendation in one paragraph

When a message has **two or more** local images, clicking one opens a **pane-scoped lightbox** (same `Modal scope="pane"` as `AttachmentLightbox`) on that image. **← / →** step through the message's images in reading order, wrapping; **Esc** closes. The bar has the `n / total` counter, previous/next buttons, **Open in editor** (today's behaviour, for the current image) and Close. A message with one image behaves exactly as today (straight to the Editor, which has zoom and pan the lightbox won't). This is a new small component that copies `AttachmentLightbox`'s shell; it does not stretch `AttachmentLightbox` itself, whose items are attachment ids with a downscaled "send copy" and a Remove button.

## 3. Behaviour

### 3.1 Where the list comes from
- **"A message" = one rendered markdown block** (`MarkdownBlock`, one assistant text node). A turn that alternates text and tool calls has several blocks; images do not step across them. (Open question Q1.)
- **Read the list from the DOM at click time**, not from a registry and not by re-parsing the text. Each `LocalImage` that has an image to show carries `data-am-media-path` (and its `alt`). On click: `el.closest(".agent-markdown-block")` (the block root, one new class or attribute on `MarkdownBlock`'s wrapper), then `querySelectorAll("[data-am-media-path]")` in document order. This is always in reading order, includes images that arrived while streaming, costs nothing while scrolling, and the virtual list needs no change (a block's images mount and unmount together).
- **Which images count:** every `LocalImage` in the block that is not `missing` or `too-large`. One still `waiting` (the inline image loads lazily near the viewport) is in the list, and the lightbox loads it when stepped to (3.4). Excluded: `missing` ones, `too-large` ones (a card that opens the Editor), remote `https://` images (a chip until clicked; never auto-loaded, so never loaded by a keypress either) and raster `data:` URIs (no click action today; out of v1).
- **Video and audio are not in the list.** They are inline players with their own controls; arrow keys on a focused player seek, and stealing them would break that. A video between two images is skipped, and a message with one image and one video behaves as a one-image message (Editor). (Q2.)
- Only the **agent's own messages**: `MarkdownMediaOpts` gets one new optional field, `stepThrough?: boolean`, set only by `AgentMediaProvider`. Kanban, the Editor's markdown preview and any other `<Markdown media>` user keep today's click.

### 3.2 Keys
| Key | Does |
|---|---|
| ← / → | previous / next image, **wrapping** (same as the attachment lightbox); `preventDefault` so nothing scrolls |
| Esc | close; `stopPropagation` so it doesn't also close something under it (as `AttachmentLightbox` does) |
| Home / End | first / last (cheap, free for the keyboard) |
| Enter on a focused image in the message | opens the lightbox (the image gets `tabindex=0` and `role=button`; today it is mouse-only) |
| Enter, or the button, in the lightbox | none; **Open in editor** is a button (and `E`), which closes the lightbox and opens that image |
| ↑ / ↓ | nothing |

Wrap is the recommendation because it matches the attachment lightbox the person already knows; the counter makes the position clear. (Q3.)

### 3.3 Focus
The dialog takes focus on open (`initialFocus`, as `AttachmentLightbox` does) and focus goes back to the image that opened it on close (the build checks that `Modal` does this, and does it itself if not). While open, the arrows belong to the dialog, so they do not move the transcript's selection or the composer's caret.

### 3.4 Loading
- The current image loads through `fetchMediaBlob(path, { maxBytes: INLINE_IMAGE_MAX_BYTES })` (the same authed route and 25 MB cap as the inline image), as the **original file**, not a downscaled copy: the person is looking for detail, and it is the same bytes the Editor would show. It is shown fitted to the dialog.
- The next and previous images are fetched after the current one settles, so stepping is instant in the usual case. Blob URLs are revoked when the lightbox moves two steps away and on close.
- A file that fails or is over the cap shows the same inline text as the attachment lightbox (`This image is no longer available.`), with Open in editor still offered when over the cap. Stepping past it still works.
- The bar shows the file name, `W×H · size` (from the decoded image and the blob), and the alt text when it differs from the name.

### 3.5 Screen readers
- The dialog's `aria-label` is `Image 2 of 5: <alt or name>`; it changes on each step and a visually hidden `aria-live="polite"` line says the same, so a step is announced once.
- The `<img>` has `alt` = the agent's alt text, else the file name (an empty alt would be skipped).
- Previous/next/close/Open in editor are real buttons with labels (`Previous image`, `Next image`, ...), not only keys, as in `AttachmentLightbox`.
- In the message, the image becomes a focusable button named `Open image: <alt or name>, 2 of 5` so a keyboard user can reach it at all.

### 3.6 The jekt side panel (#23219 / #23220)
Incoming jekts are another agent's words, and `<Markdown media>` is switched on only for the agent's own messages (RICH_OUTPUT spec §4.1), so a jekt in the panel never shows an inline image and has no image list. Nothing to build there. If #23220 later shows a jekt's **attached files**, it should reuse `AttachmentLightbox` (it already steps). The lightbox is a *pane* modal, so it covers the pane including the side panel; Esc is stopped at the dialog, so it does not also collapse the panel.

### 3.7 Mobile viewer
The agentmux-mobile viewer is a separate repo that I have not read for this spec, so I am not designing it. The same semantics (one message's images, in reading order, previous/next, close) should carry over, with swipe in place of arrows. Recommendation: a follow-up task for QTest once this lands, naming this spec. (Q5.)

### 3.8 Tool-result images
`ResultImages` can show several images from one tool result. Out of v1 (the ask is messages). The new component takes items as `{ path | url, name }`, so it can take them later without change. (Q4.)

## 4. Build outline (task #23513)

1. `markdown-media.tsx`: `MarkdownMediaOpts.stepThrough`; `LocalImage` adds `data-am-media-path`, and when `stepThrough` and the block has 2+ images, the click opens the lightbox, else `openMediaInEditor` as now; `tabindex`, `role`, Enter/Space.
2. `MarkdownBlock.tsx`: a root class on the block wrapper (the DOM query's anchor). `agent-media.tsx`: `stepThrough: true`.
3. New `frontend/app/view/agent/components/MessageImageLightbox.tsx` (+ styles beside `_attachments.scss`, reusing the lightbox's tokens): `Modal scope="pane"`, items, index, the keys in 3.2, load/prefetch in 3.4, a11y in 3.5. It mounts through the agent pane (a small signal in `agent-media.tsx`'s context) so one lightbox exists per pane, not one per image.
4. Tests (vitest, beside the code): list order and exclusions from a rendered block (missing, too-large, video, remote); ← / → wrap and Esc; one-image message goes to the Editor; Kanban's `MarkdownImg` still opens the Editor; the label and live region. No Rust. No RPC types change.
5. Visual: a look check on macOS and Linux (workspace rule): the dialog fitted at small and large pane sizes, dark and light themes, with a tall, a wide and a tiny image. Changeset: `minor`.

One PR, one push after the light checks (`tsc` with the CI config, the vitest files next to the change).

## 5. Open questions (each with a recommendation)

| # | Question | Recommendation |
|---|---|---|
| Q1 | Should stepping cross the blocks of one turn (text, a tool call, more text)? | **No.** "That message" is one block; crossing would step to images the person wasn't looking at, and needs a turn-wide list from the virtual list. |
| Q2 | Include video and audio in the list? | **No.** Arrows seek in a focused player. Video and audio keep their inline players and the Editor for a full view. |
| Q3 | Wrap at the ends? | **Yes**, like the attachment lightbox; the counter shows where you are. |
| Q4 | Tool-result images (`ResultImages`)? | **Not in v1**; the component is built so it can take them next. |
| Q5 | Mobile viewer? | **Separate follow-up** for QTest; same semantics, swipe for arrows. |
| Q6 | A message with exactly one image: lightbox or Editor? | **Editor, as today.** A one-image lightbox would drop the Editor's zoom and pan for no gain. The cost: a message with one image and one with two click differently; the lightbox's Open in editor button softens it. If the operator prefers one rule, make every message image open the lightbox. |
| Q7 | Should the Editor's own arrows step through a message's list instead (leave the pane, keep zoom)? | **No.** Arrows pan there (M8), `[` / `]` already step the folder, and the Editor has no notion of the message. Changing that is a bigger and worse trade. |

## 6. Not doing
Zoom and pan in the lightbox (the Editor has them: Open in editor), a filmstrip, download or copy buttons, remote images, SVG inline, any change to the Editor, srv or RPC.
