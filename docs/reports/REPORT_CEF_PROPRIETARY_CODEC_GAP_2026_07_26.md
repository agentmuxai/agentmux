# Report: CEF build has no H.264/AAC support — a whole-app gap, not a Media-pane bug

**Status:** Confirmed root cause, no fix implemented. Filed as a tracked
follow-up so it doesn't get rediscovered from scratch next time a pane
needs to play video.
**Author:** Agent2
**Date:** 2026-07-26
**Related:** `docs/specs/SPEC_MEDIA_PANE_2026_07_26.md` (where this was
discovered while testing MP4 playback in the new Media pane).

## Finding

Playing `C:\Users\asafe\Videos\yey.mp4` (a normal H.264/AAC MP4) in the
Media pane failed with:

```
PipelineStatus::DEMUXER_ERROR_NO_SUPPORTED_STREAMS: FFmpegDemuxer: no supported streams
```

This is Chromium's media pipeline saying the container was read fine but
none of its streams have a registered decoder in this build. This is the
signature of a CEF/Chromium build compiled **without**
`ffmpeg_branding=Chrome proprietary_codecs=true` — the standard GN flags
needed to enable H.264/AAC decoding, which official/stock CEF binary
distributions ship **without**, by default, for licensing reasons.

**Confirmed this isn't dev-environment-specific.** Searched the whole repo
(Cargo configs, Taskfile, CI workflows, build scripts) for
`proprietary_codecs` / `ffmpeg_branding` / `GN_DEFINES` — zero matches
anywhere. The `cef` crate dependency (`agentmux-cef/Cargo.toml`) pulls the
standard binary distribution with no override. **This means the actual
released AgentMux app has this exact same gap** — not just this local dev
build. Any pane that can end up rendering a `<video>` element pointed at
an H.264/AAC MP4 (Browser pane included, not just Media) will hit the same
`DEMUXER_ERROR_NO_SUPPORTED_STREAMS`.

## What still works

- **WebM (VP9/Opus)** — no licensing gate, works today. This project's own
  ComfyUI-generated clips are WebM, which is why those played fine in the
  Media pane during earlier testing.
- **WAV (PCM)** — also unencumbered, no gate.
- Image formats — unaffected (not part of the media *codec* pipeline).

## What doesn't

- **MP4/MOV with standard H.264 video and/or AAC audio** — the overwhelming
  majority of real-world MP4 files. Confirmed failing.
- **MKV** — was already expected to fail regardless of codec support, since
  Chromium's `<video>` element doesn't reliably accept the Matroska
  container itself for direct playback (see the Media pane spec's
  Post-implementation corrections). The codec gap is a second, independent
  reason it wouldn't have worked anyway.

## Options considered

1. **Rebuild the `agentmuxai/cef` fork with `proprietary_codecs=true`.**
   The complete, "real" fix — playback would work the same way it does in
   Chrome itself. Cost: this repo's only existing custom-CEF build process
   (`docs/cef-build/build-patched-libcef.md`) is Linux-only, for an
   unrelated window-drag/transparency patch — a Windows build pipeline for
   this doesn't exist yet and would need to be built from scratch. Also a
   multi-hour (3-6h), ~100GB-disk Chromium compile per the existing Linux
   doc's own numbers, plus H.264 licensing terms to be aware of (per CEF
   forum discussion, free for the first 100,000 installations, fees beyond
   that). **Per explicit user direction: treat this as the last-resort
   option, not the first thing to reach for.**
2. **Server-side transcode-on-serve** (not implemented, floated as a
   middle ground) — have `stream-local-file` (or a variant) invoke a local
   `ffmpeg` binary to remux/transcode an incompatible file to WebM before
   serving it, so CEF's media pipeline never has to decode H.264 itself.
   Meaningfully lighter than option 1 (no CEF rebuild, no multi-hour
   compile) but still real, non-trivial scope: requires `ffmpeg` to be
   present on the end user's machine (or bundled — a real distribution/
   licensing question of its own, since a redistributed ffmpeg build with
   H.264 decode enabled carries the same patent-licensing consideration as
   option 1, just via a different binary), a transcode step with caching
   (transcoding is slow — can't happen synchronously on every request),
   and error handling for when `ffmpeg` isn't available. Not scoped or
   estimated in detail here — a candidate for its own spec if this
   priority rises.
3. **Do nothing beyond documenting it** (what this report does). The Media
   pane's own code is correct — MP4/MOV support isn't a pane bug, it's an
   environment/build limitation that would resolve itself if the CEF
   binary were ever rebuilt with the right flags. The pane's `onError`
   handler now surfaces the real `MediaError` code/message
   (`frontend/app/view/media/media.tsx`'s `describeMediaError`) instead of
   a generic "failed" message, so this failure mode is at least legible
   in-app rather than a silent blank pane.

## Recommendation

No action taken beyond this report — deferring the actual fix decision.
Option 3 (document + leave MP4/MOV in the Media pane's supported list, since
the code is correct and would just start working if the underlying build
ever changes) is the reasonable default until/unless video playback becomes
a priority significant enough to justify option 1 or 2's real cost.
