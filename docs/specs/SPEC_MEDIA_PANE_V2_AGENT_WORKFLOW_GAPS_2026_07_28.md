# Spec: Media pane v2 — gaps found running a real agent video-editing workflow through it

**Status:** Proposed
**Author:** AgentY
**Date:** 2026-07-28
**Related:** `docs/specs/SPEC_MEDIA_PANE_2026_07_26.md` (v1 — implemented, PR #2299;
this spec extends it and resolves several of its own "Open questions" rather
than re-deriving the architecture), `docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md`
(directly hit by the case study below, not just theoretical), `docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md`.

## Motivation — a concrete case study, not a speculative wishlist

This spec is seeded by an actual multi-hour agent workflow run outside
AgentMux (an openFrameworks/Ableton visual-sync project, then a "cut a
video to a song's transients" pipeline over six historical films), used
here as a stand-in for the general shape of "agent produces/edits media,
human reviews it" work the Media pane exists to support. Every feature
below is motivated by something that workflow actually needed and didn't
have, not a guess at what might be nice:

- **The agent's own final output didn't play.** The end deliverable was an
  H.264/AAC `.mp4`. Per the codec-gap report, that's exactly the format
  this app's CEF build can't play — so the single most important file in
  the whole session (the actual finished video) would show a decode error
  in the Media pane as shipped today. This isn't a hypothetical edge case;
  it's the median output format for "agent renders a video" work generally
  (ffmpeg's default, most encoders' default, what gets uploaded anywhere).
- **The same output file was re-rendered in place, repeatedly, while being
  reviewed.** A bug was found and fixed mid-session by re-running the
  render script against the *same output path* four times. Each time, a
  human watching that file in a live-updating pane would have had
  playback yanked out from under them (v1's open question #5, now with a
  concrete repro scenario instead of a hypothetical one).
- **A gallery of hundreds of candidate clips was reviewed before picking
  final shots.** 284 candidate shots across three source films were each
  reduced to a representative thumbnail and visually reviewed (by
  subagents, in that session) before selection. A one-file-at-a-time
  picker with no thumbnail/gallery view is the wrong shape for this kind
  of "pick from many candidates" task — v1 explicitly scoped this out as
  Phase 3; this spec argues it's now a validated, not speculative, need.
- **A structured, timestamped manifest was hand-built** (a JSON catalog +
  a Markdown breakdown mapping every cut to a timestamp, source shot, and
  lyric line) purely so a human could understand "what's playing when"
  without scrubbing the whole video by hand. That's exactly the kind of
  annotation a media pane could surface natively if it knew how to find
  the manifest.
- **Multiple full test renders were produced just to preview a cut
  before committing to it** — because there was no way to preview "these
  five trimmed segments from three different source files, in this order"
  without first paying the cost of a real ffmpeg render. An agent that
  can describe an edit as data (a list of `{source, t_start, t_end}`
  segments) but can't get a human's eyes on it without a full render is
  doing unnecessary work every iteration.

None of this requires re-architecting v1 — the watcher/WPS/`stream-local-file`
foundation is sound and this spec builds on it directly. It's gap-filling
against v1's own "Open questions" section, plus two new capabilities
(gallery view, EDL preview) that fell out of actually using this shape of
pane for a real job.

## Non-goals

Same posture as v1: this is still not a general file browser, not a video
*editor* (no in-pane trim/cut UI — EDL preview below is read-only playback
of an externally-authored edit list, not an authoring tool), and still not
solving remote/networked-agent file access (v1's open question #4 stays
open; nothing in the case study above touched a remote agent, so there's
no new evidence to resolve it one way or the other — leave it deferred).

## Design

### 1. Resolve the CEF codec gap — now with a forcing example

The codec-gap report weighed three options (CEF rebuild with
`proprietary_codecs=true`, server-side ffmpeg transcode-on-demand, or
document-and-defer) without picking one, reasonably, absent a concrete
case forcing the question. This spec provides one: **an agent's own
video-render output is unwatchable in the pane it exists for.**

Recommend **server-side transcode-on-demand** over a CEF rebuild:

- A CEF rebuild changes a whole-app, cross-platform build flag with
  licensing implications (H.264 patent licensing is why CEF ships without
  `proprietary_codecs` by default) — a much bigger decision than this
  pane's scope, and the report already treats it as the heavyweight
  option.
- ffmpeg is already a hard dependency of this exact workflow shape (any
  agent producing video is almost certainly already invoking ffmpeg to
  produce it), and this session's own pipeline already leaned on it
  extensively for exactly this kind of format massaging.
- Concretely: `stream-local-file`, on detecting an MP4/MOV request (by
  extension or a quick `ffprobe`), transcodes to a WebM (VP9/Opus, which
  the report confirms already plays) into a cache directory keyed by
  source path + mtime, and streams the cached copy. First view of a given
  file pays a real transcode cost (seconds, not instant); every
  subsequent view of the same unchanged file is a cache hit. This keeps
  the existing "serve any local path" security posture unchanged — it's
  a transform of the response body, not a new access surface.
- Cache invalidation is free to get right: key = `(path, mtime, size)`,
  same signal the v1 watcher already tracks for "did this file change."

### 2. Pin vs. follow mode, and non-destructive live-update (resolves open questions #2 and #5)

The case study's repeated-re-render-of-the-same-path scenario makes both
of v1's deferred product decisions concrete enough to just decide:

- **Two explicit modes**, not one auto-behavior: **Pinned** (default —
  shows exactly the file/path the user picked, full stop, matches v1's
  existing single-file mode exactly) and **Follow latest** (opt-in,
  directory-watch mode, shows most-recently-modified matching file — this
  is v1's existing directory-watch behavior, just given a name and made
  switchable rather than being the only directory-mode behavior).
- **Within either mode, a content change to the *currently displayed*
  file (same path, new mtime — the re-render-in-place case) never yanks
  an active playback.** Instead: show a small, non-blocking "Updated —
  reload" affordance in the pane header (same visual language as a
  browser tab's "this page has updates" pattern), and only swap `src`
  when the user clicks it, or immediately if the video is paused/at rest
  (not actively playing). This is strictly a refinement of v1's Phase 2
  reload logic — same WPS event, same re-fetch call, just gated on
  playback state before applying it.
- Mode is per-block state, alongside the existing `media:path` meta key —
  add `media:mode` (`"pinned" | "follow"`), defaulting to `"pinned"` per
  v1's own recommendation in its open question #2.

### 3. Gallery/grid view (v1's Phase 3, now scoped concretely)

For a **watched directory** in Follow mode, add a toggle between the
existing single-media view and a grid of thumbnails for every matching
file in the directory (not just the latest) — directly modeled on the
284-thumbnail review workflow from the case study, which was done by
hand with ffmpeg because no in-app equivalent existed.

- **Thumbnail generation, server-side, on demand.** New backend
  capability (there is currently none anywhere in the codebase, per the
  v1 research): for a video file, extract a single frame via ffmpeg at
  either a fixed offset or the file's midpoint (the case study used
  midpoint — a more representative frame than 0:00, which is often a
  black/title frame); for an image file, a resized copy. Cache keyed the
  same way as §1's transcode cache (`path, mtime, size`). Served through
  a new route, e.g. `GET /agentmux/media-thumbnail?path=...`, same auth/
  path-scoping posture as `stream-local-file`.
- **Grid interaction:** clicking a thumbnail switches the pane to normal
  single-media (Pinned) view for that file — the gallery is a picker, not
  a permanent multi-up display.
- Explicitly *not* proposing thumbnail generation as a blocking part of
  the directory watcher's existing change-detection (§ of v1) — keep
  those concerns separate; the watcher still just says "this path
  changed," thumbnail generation is a pull-based, cacheable, lazy
  operation triggered by the gallery view actually being open.

### 4. Audio waveform preview

`<audio controls>` alone gives no visual sense of where content is in a
long file — the case study manually generated waveform PNGs via ffmpeg's
`showwavespic` filter multiple times purely to sanity-check that two
audio files were meaningfully different before trusting a mux step.

- Same pattern as thumbnails (§3): a cached, on-demand server-side
  render via ffmpeg (`showwavespic`), served as an image, shown above the
  native `<audio>` element rather than replacing it (no custom playback/
  scrub UI — that's real scope creep against this pane's non-goals; a
  static waveform image purely as visual context is not).

### 5. EDL / segment-list preview mode

The genuinely new capability, not present in any form in v1: let an agent
hand the Media pane a small JSON document describing an ordered sequence
of trimmed segments — `[{source: <path>, t_start: <sec>, t_end: <sec>}, ...]`
— pointed to via a `.json` file with a recognized shape, and have the pane
**play them back in sequence without a pre-render step.**

- Motivation, directly from the case study: the same 11-second candidate
  cut was rendered to a temporary `.mp4` and reviewed *repeatedly* while
  iterating on a shot-selection algorithm, purely because there was no
  way to preview "these five trimmed segments in this order" without
  paying a real ffmpeg-encode cost every time. A pure-playback preview
  (seek source A to `t_start`, play until `t_end`, cut to source B at its
  `t_start`, ...) is strictly cheaper and is standard `<video>`-element
  behavior (`currentTime` + a `timeupdate`/`ended`-driven advance), not a
  new rendering capability.
- Explicitly **not** proposing this as a general editing timeline UI (no
  drag-to-trim, no add/remove segment controls in-pane) — the EDL is
  authored externally (by an agent, or by hand as JSON) and the pane's
  job is strictly "play this list back accurately," matching the
  project's general preference against building configurability/authoring
  UI nothing has asked for yet. If in-pane authoring turns out to be
  wanted later, that's real new scope for its own spec.
- Detection: a `.json` path handed to the Media pane whose top-level shape
  matches `{segments: [...]}` (or a bare array of the same shape) is
  treated as an EDL rather than attempting to render it as an image/video
  directly. A malformed/non-matching JSON falls through to today's
  existing unsupported-extension error path, unchanged.

### 6. Media-scoped lightweight file picker (resolves open question #1)

v1 left this genuinely undecided between "reuse Editor's file-tree" and
"stay dialog-only." The case study's evidence: the relevant files were
scattered across four different directories (`Downloads/HarlemRenaissance`,
`Documents/<project>/pipeline`, `Music/AUDIO/<project>/Samples/Imported`,
plus generated thumbnails/manifests alongside), and a one-shot OS file
dialog per pick, with no memory of "the other candidate files were right
next to this one," made repeated comparisons across files clunky.

Recommend a **new, small, media-scoped tree component** — not a reuse of
Editor's file-tree (which explicitly refuses binary content by design,
per v1's own research, so it's the wrong component to extend) — showing
only media-matching extensions within the current directory + a "up one
level" affordance, opened from the same header button that today opens
the OS picker (OS picker stays available too, not replaced). This is
intentionally the smallest version of "browse near where I already am"
that helps the multi-file-comparison case, not a full filesystem browser.

## Open questions

1. **Thumbnail/transcode cache location and eviction.** Both §1 and §3
   propose an on-disk cache keyed by `(path, mtime, size)`. Where does it
   live (a subdirectory under the existing data dir?), and does it need
   eviction beyond "stays until the app's normal data-dir cleanup," given
   video transcodes specifically could be sizable? Worth sizing against
   real usage before over-building an eviction policy nothing has asked
   for yet.
2. **EDL source-file mismatch handling.** If an EDL references a source
   file that's been moved/deleted since the EDL was authored, or a
   `t_start`/`t_end` outside the file's actual duration — fail the whole
   preview with one clear error, or skip just that segment and continue?
   Leans toward "skip and show a non-blocking per-segment warning,"
   consistent with the project's general preference for graceful
   degradation over hard-failing on partial bad input, but worth a
   product decision rather than assuming.
3. **Does §1's transcode change what `stream-local-file` reports as
   Content-Type/size to the frontend** — i.e., does the frontend need to
   know "this is a transcoded copy, not the original bytes" for any
   reason (e.g. a "download original" affordance), or is transparent
   substitution fine? Leans toward transparent (the pane is for viewing,
   not archival/export), but flagging since it's a real behavior change
   from today's byte-exact passthrough.

## Files (anticipated — this spec does not implement)

| File | Relevance |
|------|-----------|
| `agentmux-srv/src/server/files.rs` | `stream-local-file` — add transcode-on-demand (§1) and a new `media-thumbnail`/waveform route (§3, §4) |
| `agentmux-srv/src/backend/media_file_watcher.rs` | No structural change — §2's pin/follow split is a frontend-state and reload-gating change, not a watcher change |
| `frontend/app/view/media/media.tsx` | Pin/follow toggle + non-destructive update affordance (§2), gallery grid view (§3), waveform display (§4), EDL playback mode (§5), picker entry point for §6 |
| `frontend/app/view/media/` (new files) | New media-scoped tree component (§6), gallery grid component (§3), EDL playback controller (§5) |
| `docs/specs/SPEC_MEDIA_PANE_2026_07_26.md` | v1 — this spec's open questions #2, #5, #1(picker), and Phase 3 are the items being resolved here |
| `docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md` | The codec gap §1 proposes resolving, now with a concrete forcing example |
