# Spec: Media pane v3 — persistent browser + custom playback/scrub UI

**Status:** Proposed
**Author:** AgentY
**Date:** 2026-07-29
**Related:** `docs/specs/SPEC_MEDIA_PANE_2026_07_26.md` (v1 — implemented,
PR #2299, commit `bd7c20609`),
`docs/specs/SPEC_MEDIA_PANE_V2_AGENT_WORKFLOW_GAPS_2026_07_28.md` (v2 —
merged, PR #2344; this spec supersedes v2 §3's interaction model
specifically, see below), `agentmux-srv/src/config/widgets.json`,
`frontend/app/view/media/`.

## Motivation

Explicit ask: a "media browser" — rich file browsing plus playback, no
editing tools (no trim/cut/timeline authoring; that's real scope for a
different tool entirely). Two concrete requirements beyond what v1/v2
cover:

1. **Rich browsing**, not a one-file-at-a-time picker or a "grid that
   becomes a single view when you click something." Browsing and playback
   should coexist on screen at once — this is a real change from v2 §3's
   proposal, not just an addition to it (see "Supersedes v2 §3" below).
2. **A playback/scrub area that isn't the native `<video controls>`
   element** — a purpose-built transport bar the pane owns and themes
   itself, not whatever the browser engine bolts onto a video tag.

## Research: how established tools do this

Two real, converging patterns from actual professional/consumer media
tools, not invented from scratch:

**Layout — a persistent library + preview split**, not a modal/switching
one. Adobe Bridge, Unreal Engine's Content Browser, Blender's Asset
Browser, and DaVinci Resolve's Media Pool all use the same shape: a
folder/library navigator, a thumbnail grid as the main browsing surface,
and a dedicated preview area with its own transport controls — all
visible together, not one replacing another. ([Unreal Content Browser
docs](https://docs.unrealengine.com/4.27/en-US/Basics/ContentBrowser/UI),
[Blender Asset Browser
docs](https://docs.blender.org/manual/en/latest/editors/asset_browser.html),
[Adobe Bridge preview/playback
docs](https://helpx.adobe.com/si/bridge/using/preview-dynamic-media-files-adobe.html))

**Hover-scrub on thumbnails** — Adobe Bridge specifically: hovering a
video thumbnail in the grid shows a silent, fast preview scrubbing through
the clip's frames, reverting to the static thumbnail on mouse-out, so you
can triage many candidates without opening any of them. A DaVinci Resolve
forum thread independently confirms editors consider hover-scrub "the
fastest way" to evaluate source clips before committing to one. ([Adobe
Bridge
docs](https://helpx.adobe.com/si/bridge/using/preview-dynamic-media-files-adobe.html),
[Blackmagic Forum
thread](https://forum.blackmagicdesign.com/viewtopic.php?t=104902&p=580242))
— we take the triage/preview half of this pattern, explicitly not the
"drag the scrubbed range onto a timeline" half, which is an editing
workflow out of scope here.

**Custom scrubber mechanics** — the consistent recipe across multiple
implementation write-ups: a draggable slider synced via
`requestAnimationFrame` during playback (not a `timeupdate`-driven
re-render, which is choppier), pointer-events for drag (not separate
mouse/touch handlers), and a hover/drag preview (thumbnail or waveform)
for scrub feedback. Styling is fully custom CSS, matching the host app's
theme rather than the browser's native player chrome. ([general custom
video player UI
guide](https://www.vidzflow.com/blog/designing-a-custom-video-player-ui-tips-for-performance-and-accessibility),
[Adobe AEM scrubber skinning
docs](https://experienceleague.adobe.com/en/docs/dynamic-media-developer-resources/library/viewers-aem-assets-dmc/video/customizing-video-viewer/r-html5-viewer-20-customize-videoscrubber),
[timeline scrubbing implementation
notes](https://palospublishing.com/incorporating-timeline-scrubbing-in-a-custom-editor/))

## Supersedes v2 §3's interaction model

v2 proposed gallery view as a **picker**: click a thumbnail, the pane
swaps to normal single-media view. Real-tool research above doesn't
support that model — every reference tool keeps the browser and the
preview visible **together**. This spec replaces that part of v2's
design; v2's other proposals (thumbnail generation approach, pin/follow
modes, EDL preview, codec transcode) are unaffected and still stand as
written.

## Non-goals (unchanged, restated because this spec touches UI surface)

No trim/cut/split tools, no timeline authoring, no drag-to-arrange, no
in/out-point-to-sequence workflow. This is a **browser + viewer**, not an
editor — everything here is about finding and watching media faster, not
assembling it into something new. If in-pane editing is ever wanted,
that's real new scope for its own spec, not an extension of this one.

## Design

### Layout — persistent split, not a picker/viewer toggle

```
+----------------------------------------------------------------+
| [Media pane header: path breadcrumb, view toggle, filter]      |
+---------------------------+------------------------------------+
|                           |                                    |
|   THUMBNAIL GRID          |         PREVIEW AREA                |
|   (scrollable, resizable  |   (selected item's image/video,     |
|   divider between panes)  |    letterboxed to fit)              |
|                           |                                    |
|   [th][th][th][th]        |                                    |
|   [th][▓▓][th][th]  <-----|-- currently selected                |
|   [th][th][th][th]        |                                    |
|   [th][th][th][th]        |                                    |
|                           |                                    |
|   (scrolls independently) +------------------------------------+
|                           | ▶  ●──────────○───────────  1:23/3:45 |
|                           | [Loop] [0.5x▾] [🔊──] [⛶]           |
+---------------------------+------------------------------------+
```

- Two resizable panes within the Media pane's own body (a divider,
  draggable, with a sane default split — ~35/65 grid/preview leaning
  toward preview since that's usually the larger content). Both panes
  visible simultaneously; no view-switching state to manage.
- Grid pane: reuses v2 §3's thumbnail generation/caching design
  unchanged. Selecting a thumbnail (click) loads it into the preview pane
  — does not navigate away from the grid.
- Preview pane: renders the selected item (image, or video/audio with the
  custom transport bar below it, §"Custom transport bar").
- For non-directory/single-file mode (v1's original "point at one file"
  usage, and Pinned mode from v2 §2) — the grid pane can collapse/hide
  when there's nothing to browse (a single pinned file, no directory
  context), reverting to a preview-only layout. The split is additive to
  existing modes, not a replacement that forces browsing UI onto every use
  case.

### Custom transport bar (video/audio, replaces native `<video controls>`)

A single shared component (`PlaybackTransport`, used under both the image/
video preview area and directly for audio-only files), built per the
research recipe:

- **Play/pause** button (also spacebar when the pane has focus).
- **Scrub slider**: custom draggable element, not `<input type="range">`
  styled — position updates via `requestAnimationFrame` while playing
  (smooth, not tied to the `timeupdate` event's coarser firing rate), and
  responds to pointer drag for seeking. Drag preview: while dragging,
  update the preview area's video frame live (`video.currentTime =
  <dragged position>` on a muted/paused clone or the same element scrubbed
  directly) rather than only showing a tooltip — matches what the research
  calls out as the more useful feedback mode for actual seeking, not just
  a time readout.
- **Timecode display**: `current / duration`, monospace, updates with the
  scrub position.
- **Loop toggle** — directly useful for exactly the kind of short-clip
  repeated review this pane exists for (reviewing a few seconds of agent
  output over and over while judging it).
- **Playback rate** (0.5x / 1x / 1.5x / 2x) — cheap to add given the
  native `<video>` element already supports `playbackRate`, and
  particularly useful for reviewing fast-cut content frame-by-frame-ish
  without needing real frame-stepping.
- **Volume** slider + mute toggle.
- **Fullscreen/expand** toggle — since the preview area is one pane among
  several in a tiled layout, a one-click "make this pane's preview fill
  the window" affordance matters more here than in a dedicated media app.
- Explicitly not included: in/out point markers, trim handles, speed ramp
  curves, or anything that implies building an edit — those are editing
  tools, out of scope per this spec's Non-goals.

### Hover-scrub on grid thumbnails

On mouse-hover over a video thumbnail (not click — click selects it into
the preview pane, a separate action), map horizontal mouse position within
the thumbnail to a playback position and show that frame, reverting to the
static (midpoint) thumbnail on mouse-out. Two implementation options,
in increasing cost/accuracy order:

1. **Sampled-frame swap** (cheap): server generates N frames per video
   (e.g. 8-12, evenly spaced) at thumbnail-generation time (extends v2
   §3's single-midpoint-frame generation to a small frame strip), client
   swaps the displayed `<img>` based on hover x-position — no video decode
   in the browser at all, just image swaps.
2. **Live muted scrub** (more accurate, heavier): an actual `<video>`
   element in the grid cell, muted, seeking on hover-move — real frame
   accuracy but a decode cost per hovered thumbnail.

Recommend starting with (1) — matches Adobe Bridge's own approach closely
enough for triage purposes, and reuses infrastructure v2 already proposed
rather than adding new decode-on-hover cost to every grid cell.

## Open questions

1. **Divider position persistence** — should the grid/preview split ratio
   be remembered per-block (like `media:path`) or always reset to the
   default? Leans toward persisting (matches the project's general
   pattern of persisting pane-specific state via `block.meta`), but a
   product call, not purely technical.
2. **Grid item size / density control** — fixed thumbnail size, or a
   size slider like several reference tools have (Blender's adjustable
   preview size, Unreal's Tiles/List/Columns view switch)? Lean toward a
   single fixed size for v1 of this feature — matches the project's
   general preference against building configurability nothing has asked
   for yet — with a size control as a natural, easy follow-up if it turns
   out to matter.
3. **Selection model** — does selecting a grid item for preview also
   update `media:path` (so a reload/relaunch reopens on the same
   selection), or is grid selection ephemeral/session-only? Leans toward
   persisting, consistent with existing Pinned-mode behavior, but worth
   confirming since it changes what "the pane's saved state" means once
   browsing is a first-class mode alongside single-file pinning.

## Files (anticipated — this spec does not implement)

| File | Relevance |
|------|-----------|
| `frontend/app/view/media/media.tsx` | Split-pane layout, replaces v2 §3's picker/viewer-toggle sketch |
| `frontend/app/view/media/` (new) | `PlaybackTransport` component (shared video/audio transport bar), grid component with hover-scrub, resizable-divider component (check for an existing shared one in this codebase before building new — panes elsewhere likely already need resizable splits) |
| `agentmux-srv/src/server/files.rs` | Extend v2 §3's thumbnail route to optionally return a frame strip (N timestamps) for hover-scrub, not just one midpoint frame |
| `docs/specs/SPEC_MEDIA_PANE_V2_AGENT_WORKFLOW_GAPS_2026_07_28.md` | §3's picker/viewer-toggle interaction model is superseded by this spec's persistent split; §1/§2/§4/§5/§6 unaffected |
