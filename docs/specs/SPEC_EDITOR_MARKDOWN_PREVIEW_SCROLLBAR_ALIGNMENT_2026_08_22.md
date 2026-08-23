# SPEC: Editor pane — align markdown preview's scrollbar with source mode

**Date:** 2026-08-22
**Status:** Draft
**Author:** Korp
**Repo touched:** `agentmux` (`frontend`)
**Related:** `frontend/app/view/editor/editor-view.tsx`, `frontend/app/view/editor/editor-view.scss`,
`frontend/app/element/markdown.tsx`, `frontend/app/element/markdown.scss`,
`docs/specs/SPEC_TOOL_PREVIEW_SCROLLBAR_EDGE_PADDING_2026_08_08.md` (the
established house precedent for this exact failure mode — see §2)

## 1. Problem

In the Editor pane's markdown Source mode, the scrollbar sits flush against
the pane's right edge (CodeMirror's native scrollbar, no ancestor padding).
In Preview mode, the scrollbar looks misplaced — it renders inset from the
true edge, floating inside a padding gutter instead of hugging the pane
border the way Source mode's does.

## 2. Root cause

Source mode (`editor-view.tsx:890-896`, `editor-view.scss:557-561`):
`.editor-codemirror` has no padding anywhere in its ancestor chain.
CodeMirror's own `.cm-scroller` is the sole scroll container, so its native
scrollbar paints flush against the pane's true right edge.

Preview mode (`editor-view.tsx:905-916`) nests two scroll containers instead
of one:

- `.editor-preview-content` (`editor-view.scss:581-589`) — `overflow-y: auto`
  **and** `padding: 16px 24px`, wrapping...
- `<Markdown>`'s own `.markdown > .content` (`markdown.tsx:317-343`,
  `markdown.scss:19-38`) — `overflow: scroll`, converted at mount
  (`markdown.tsx:299-304`) into an OverlayScrollbars-managed viewport (the
  library that renders the visible track/thumb).

Because the 16px/24px padding lives on the **outer** `.editor-preview-content`
wrapper rather than on the element OverlayScrollbars actually manages, the
entire `.markdown` box — and with it the visible OverlayScrollbars track —
is inset 24px from the pane's true right edge. The outer `overflow-y: auto`
is also redundant: `.content` already owns real scrolling, so the outer
wrapper is a second, unnecessary scroll container that exists only to hold
the padding.

Track width/color are not the issue — `--os-size: 7px` (`app.scss:142`)
already matches the native `*::-webkit-scrollbar { width: 7px }`
(`app.scss:87-90`), and both pull thumb colors from the same
`--scrollbar-thumb-color` tokens. This is purely a positioning bug, not a
styling mismatch.

This is the same failure mode `SPEC_TOOL_PREVIEW_SCROLLBAR_EDGE_PADDING_2026_08_08.md`
already diagnosed and fixed for the agent pane's tool-call previews: a
scrollbar always renders at its own owning element's border box, unaffected
by that element's own padding — so any padding placed on an *ancestor* of
the scroll-owning box pushes the scrollbar (track and all) inboard of the
visible surface's true edge. That spec's fix was to zero the ancestor's
padding and re-home the inset onto the scroll-owning element itself
(`.agent-tool-panel` → `.agent-tool-overlay-log`). The same technique
applies here regardless of OverlayScrollbars vs. a native `::-webkit-scrollbar`
— it's ordinary CSS box-model behavior, not a library-specific quirk.

## 3. Fix

Move the padding off the outer wrapper and onto the element OverlayScrollbars
actually hosts, using the `contentClass` prop `<Markdown>` already exposes
for exactly this kind of per-consumer scoping (`markdown.tsx:72,99,322,327`
— currently declared but unused by any caller). OverlayScrollbars reads
padding declared on its host element and renders it as inner content inset
while keeping the scrollbar track anchored to the host's true border box —
so applying the padding this way keeps the same visual text inset while
letting the scrollbar sit flush, matching Source mode.

Changes:

1. **`editor-view.scss:581-589`** (`.editor-preview-content`) — drop
   `padding: 16px 24px` and the now-redundant `overflow-y: auto`. It becomes
   a plain flex sizing wrapper (`flex: 1 1 auto`), no longer a scroll
   container or padding holder.
2. **`editor-view.tsx:914`** — pass a new `contentClass="editor-preview-markdown-content"`
   prop to `<Markdown textAtom={() => liveDoc()} />`, so the padding lands
   on `.content` (the OverlayScrollbars host) scoped to this one call site
   only — not on `.markdown-content-inner` or any other global class shared
   by every other `<Markdown>` consumer (agent chat, tool-call output, etc.).
3. **`editor-view.scss`** — add:
   ```scss
   .editor-preview-markdown-content {
       padding: 16px 24px;
   }
   ```

No changes needed to `markdown.tsx`/`markdown.scss` — `contentClass` is
already wired through to `.content` for this exact purpose.

## 4. Why this is scoped correctly

- `contentClass` only ever applies to the one `.content` div OverlayScrollbars
  manages for the instance it's passed to (`cn("content", contentClassName)`)
  — every other `<Markdown>` render site in the app (none of which currently
  pass `contentClass`) is untouched.
- `.editor-preview-content` is used at exactly one call site
  (`editor-view.tsx:913`), so narrowing its CSS to a plain flex wrapper has
  no other blast radius.
- Track/thumb size and color are untouched — this only changes where the
  padding lives, not `--os-size` or any scrollbar-color token.

## 5. Testing plan

- Manual: open a markdown file in the Editor pane, switch to Preview and
  Split modes, confirm the scrollbar sits flush against the pane's right
  edge in both, matching Source mode's CodeMirror scrollbar position.
- Manual: confirm the rendered markdown text still has its original
  16px/24px visual inset (no regression to reading comfort/line length).
- Manual: confirm scrolling still works (drag thumb, wheel, keyboard) with
  the padding relocated.
- Manual: spot-check another `<Markdown>` consumer (e.g. an agent chat
  message) to confirm its scrollbar/padding is unaffected, since
  `.editor-preview-markdown-content` is scoped to the editor's call site only.

## 6. Follow-up: native scrollbar, not just repositioned (same day)

Repositioning (§3) put the OverlayScrollbars track at the right spot, but it's
still visually a different scrollbar technology than Source mode's plain
native CodeMirror scrollbar — a JS-rendered overlay track with `autoHide:
"leave"` fade behavior, thinner-looking than a real webkit scrollbar even at
the same declared `--os-size`/`width` (7px both). Requested follow-up: make
Preview use the exact same scrollbar as Source, not just a same-sized one in
the same place.

**Fix:** `<Markdown>` gains a new `nativeScrollbar` prop
(`frontend/app/element/markdown.tsx`). When set, `onMount` skips calling
`OverlayScrollbars(contentsEl, ...)` entirely, leaving `.content`'s plain CSS
`overflow` to render a real native/webkit scrollbar — styled by the exact
same universal `*::-webkit-scrollbar` rule (`app.scss:87-90`) CodeMirror's
`.cm-scroller` already uses. The heading-anchor-scroll effect (`focusedHeading`)
was updated to fall back to `contentsEl` directly as the scroll viewport when
`contentsOs` is null, so in-page anchor links (`[text](#heading)`) still work
in native mode.

Editor preview passes `nativeScrollbar` at its one call site
(`editor-view.tsx`). `editor-view.scss`'s `.content.editor-preview-markdown-content`
override switches `overflow-y` from the base `.content { overflow: scroll }`
to `auto` (matching CodeMirror's `.cm-scroller` default — no reserved
scrollbar gutter when content already fits).

No other `<Markdown>` consumer passes `nativeScrollbar` — everyone else keeps
OverlayScrollbars unchanged (default `false`).

Verified live via `task dev`: Source and Preview scrollbars are now visually
identical (same width, same native browser rendering, no auto-hide fade
behavior difference).
