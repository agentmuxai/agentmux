# Spec: Media pane — live-updating image/video viewer for agent-generated files

**Status:** Implemented (PR #2299). Phase 1 + Phase 2 both shipped in one
pass rather than split across separate PRs as originally sketched below —
see "Post-implementation corrections" for what changed once real review
(ReAgent + Codex) exercised the code.
**Author:** Agent2
**Date:** 2026-07-26
**Related:** `docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md` (the
closest existing precedent — this design explicitly reuses and generalizes
its watcher pattern), `docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md`,
`docs/analysis/ANALYSIS_TOOL_PREVIEW_RUNNING_TO_COMPLETED_JERK_2026_07_05.md`
(a UI-transition pitfall this design should not repeat), `CLAUDE.md`'s
Widgets table (`agentmux-srv/src/config/widgets.json` is the canonical,
closed widget list — adding this pane means adding a real entry there, not
inventing one ad hoc), `docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md`
(confirmed while testing this pane: MP4/MOV playback fails on this app's
CEF build for any standard H.264/AAC file — a whole-app build limitation,
not a pane bug; see that report for the root cause and options).

## Motivation

An agent running a local generation pipeline (this was prompted by a
ComfyUI/Comfy Cloud video-generation workflow, but the need is general —
any agent producing images, video, or other binary artifacts into its
workspace) currently has no way to show that output inside AgentMux. The
only options today are: read the image via the agent's own multimodal
`Read` tool (shows it once, in the conversation, not a persistent/live
view), or open the OS file explorer. Neither updates live as new files
appear, and neither gives the human a dedicated place to watch generation
progress across a session.

Goal: a new **Media** pane — a real widget-bar entry, same tier as Editor/
Terminal/Browser — that displays an image or video file from the local
filesystem and **updates automatically when the file it's pointed at (or a
new file appearing in a watched directory) changes**, without a manual
reload.

## Non-goals

- **Not** true frame-by-frame streaming of an in-progress/partially-written
  video file. Generation pipelines like ComfyUI produce a complete file at
  the end of a job (`SaveWEBM`/`SaveImage` write once, atomically-ish, on
  completion) — there is no useful partial content to stream mid-encode.
  The realistic granularity is "a new/changed file appeared," which a
  watch-and-reload design (like the Editor live-reload spec) handles
  correctly; do not build the heavier `getFileSubject` binary-append-stream
  machinery (§3 below) for this — that's for genuinely incremental content
  like a growing terminal ring buffer, which this isn't.
- **Not** a general file browser or artifact manager. Scope is "point this
  pane at a path (file or directory) and view the media there," not
  building a new file-tree/browser UI. If a directory is watched, the pane
  shows the most recent matching file, not a gallery/grid (a gallery view
  is a reasonable future extension, out of scope for v1).
- **Not** solving arbitrary-path filesystem exposure generally. This design
  deliberately scopes what paths the new serving endpoint will read (see
  "Security: path scoping" below) rather than opening a fully general
  "serve any local path" capability.

## Existing architecture (research summary)

### Pane/widget registration

Two layers, both need a new entry:

1. **`agentmux-srv/src/config/widgets.json`** — the canonical, closed widget
   list (per `CLAUDE.md`: "do not invent or reference widgets that don't
   exist here"). Each entry is `defwidget@<name>` with `display:order`,
   `display:pinned`, `icon`, `color`, `label`, `description`, and a
   `blockdef.meta.view` string that becomes the pane's `viewType`.
2. **`frontend/app/block/block-registry.ts`** — the actual viewType →
   `ViewModel` class map (`blockViewRegistry.set("agent", AgentViewModel)`,
   etc.). Per the file's own header comment, adding a pane never requires
   touching `block.tsx` — only a `registerBlockView()` call here, plus a
   `ViewModel` implementation (`viewType`, `viewIcon`, `viewName`,
   `viewComponent`, `dispose` — `frontend/types/custom.d.ts:481-510`) and
   its Solid component.

Secondary, low-cost additions: `frontend/app/block/blockutil.tsx`'s
`blockViewToIcon`/`VIEW_LABELS` maps (fallback icon/label before the
ViewModel's own memo takes over), and a row in `CLAUDE.md`'s Widgets table
(repo documentation convention, not code).

### The tool-preview precedent — proves the plumbing, not the media handling

Tool-call previews (`frontend/app/view/agent/components/ToolBlock.tsx`,
`ToolOverlayLog.tsx`) already do "live-updating content tied to a running
process, driven by push events" — but only for **text**. The renderer
registry (`tool-renderers/registry.ts`) resolves to `DiffViewer`,
`BashOutputViewer`, `HighlightedCode`, `RecordTable`, `SearchResults`,
`WebFetchResult`, `Markdown`, or raw JSON — no image/binary path exists
today. This confirms the live-update wiring pattern is sound and battle-
tested, but a Media pane needs its own content-rendering path (`<img>`/
`<video>` elements), not a reusable renderer from this registry.

**One pitfall to explicitly avoid, already paid for elsewhere in the
codebase:** the still-open "tool-preview running→completed jerk" bug
(`docs/analysis/ANALYSIS_TOOL_PREVIEW_RUNNING_TO_COMPLETED_JERK_2026_07_05.md`)
is caused by two independent UI signals (a status leaving `"running"`, and
a content-shape change from log-tail to result-view) flipping in the same
render tick with no transition, producing a visible snap. A Media pane's
"waiting for file" → "file appeared" transition is the same shape of
problem (a status/visibility flag and a content swap changing together) —
the design should budget for a deliberate crossfade/height-transition from
the start rather than repeating the bug.

### Real-time update mechanism — three layers, one is close to a direct fit

Everything live flows over one shared per-tab WebSocket
(`frontend/app/store/ws.ts`), carrying:

1. **RPC request/response and server-push commands**
   (`frontend/app/store/rpc-client.ts` — `rpcCall`, plus a full-duplex
   `handleIncomingCommand` dispatch for server-initiated commands) and a
   **streaming-response generator**, `rpcStream()`, already used for file
   reads (`FileReadStreamCommand` etc., `rpc-api/file.ts`).
2. **WPS ("Wave Pub/Sub") — scoped push notifications, "wake signal, not
   payload."** `frontend/app/store/wps.ts`'s `waveEventSubscribe()`
   subscribes a component to `{eventType, scope}`; backend events are
   published via a `Broker`/`WaveEvent` primitive
   (`agentmux-srv/src/backend/wps.rs`) scoped as `"block:<blockId>"`. The
   already-shipped `EVENT_EDITOR_FILE_CHANGED`
   (`agentmux-srv/src/backend/editor_file_watcher.rs`) is the exact shape
   needed here: a `notify`-crate filesystem watcher, refcounted per watched
   path, publishing a content-free "this path changed" event scoped to the
   block(s) watching it; the frontend re-fetches on receipt rather than the
   event carrying payload. **This is the mechanism to reuse for the Media
   pane's live-update behavior** (see Design below) — generalized from
   "single explicitly-opened file" to "a watched directory, notify on
   new/changed matching files."
3. **`getFileSubject`/`WSFileEventData`** (`wps.ts:107-124`) — a base64
   append/truncate byte-stream subscription, currently used only by the
   terminal pane to stream PTY output live
   (`frontend/app/view/term/termwrap.ts`). This is the "genuinely
   incremental content" primitive explicitly ruled out by the Non-goals
   section above — noted here because it exists and is content-agnostic,
   in case a future genuinely-streaming use case (e.g. a live webcam/screen
   feed) wants it, but the wrong tool for "a finished file appeared."

### No existing artifact/media viewer — and one relevant gap already visible

- The Editor pane's file tree browses arbitrary directories on disk, but
  **explicitly refuses binary content** (`editor-model.ts`'s
  `sniffUnopenable` rejects any file with a NUL byte in its first 4KB) — by
  design, not a bug to route around.
- `EVENT_EDITOR_FILE_CHANGED` today is scoped to "files with an open editor
  tab, refcounted" — not a general "watch this directory" capability. The
  Media pane's watcher needs the same underlying `notify`-crate/debounce
  machinery generalized to directory-mode, not file-mode.
- **The frontend already contains dead code that assumes arbitrary-local-
  path media streaming exists**: `frontend/app/view/term/termsticker.tsx`
  builds `.../agentmux/stream-local-file?path=<rawAbsolutePath>` directly —
  exactly the endpoint and query shape this design implements, confirmed by
  reading the call site. **Implementing `stream-local-file` fixes this dead
  code for free.** `frontend/app/element/markdown-util.ts` /
  `frontend/util/waveutil.ts` are a *different*, larger scope: they hit
  `.../agentmux/stream-file?path=<uri>` where `<uri>` comes from
  `formatRemoteUri()` (`waveutil.ts:92-100`), which wraps the path as
  `wsh://<connection>/<path>` (or an `aws:...:s3://` form) — a
  connection-scoped URI format for a broader remote-file concept this spec
  doesn't touch. **Not fixed by this design** — `stream-file` (as opposed
  to `stream-local-file`) stays a 501 stub; wiring it up would mean parsing
  the `wsh://` URI scheme and routing by connection, which is real,
  separate scope belonging to whatever spec eventually covers remote/
  connection-based file access, not the Media pane.

### Backend file serving — one working blob store, one working static-file pattern, one stubbed general route

- **`GET /agentmux/file`** (`agentmux-srv/src/server/files.rs:26-99`) —
  works, but only for AgentMux's own internal zone-scoped blob store
  (SQLite-backed `FileStore`), not arbitrary filesystem paths. Not directly
  usable without first importing files into this store, which is more
  machinery than needed here.
- **`GET /agentmux/docsite/*path` / `GET /agentmux/schema/*path`**
  (`files.rs:101-155`) — working static-file serving with **already-correct
  image MIME type mapping** (`mime_from_path`, `files.rs:157-170`: png,
  jpg/jpeg, svg, fonts) and explicit path-normalization/traversal guards,
  but scoped to fixed, bundled app directories. The MIME-mapping and
  traversal-guard *pattern* here is exactly what the new route should copy
  (extended with video MIME types: webm, mp4).
- **`GET /agentmux/stream-file`, `/agentmux/stream-file/*path`,
  `GET /agentmux/stream-local-file`** — routed, but unimplemented
  (`stub_501`). This is the natural implementation target: the frontend
  already expects it to exist (see above), the route naming already
  signals "arbitrary local path," and it just needs a real handler.

## Design

### Phase 1 — static Media pane, on-demand path, no live updates yet

1. **Widget registration.** Add `defwidget@media` to `widgets.json`
   (`blockdef.meta.view: "media"`), plus the `CLAUDE.md` table row. Icon/
   color per existing convention (something distinct from Editor/Browser —
   e.g. a picture/film icon).
2. **`MediaViewModel` + `MediaView` component**
   (`frontend/app/view/media/`, mirroring the Editor/Terminal pane file
   layout). Pane state: a target path (file or directory), set via a
   header input or a "pick file" action (reuse the Editor pane's existing
   file-tree/picker component rather than building a new one, if it's
   reasonably decoupled from Editor-specific state). Renders `<img>` for
   image extensions, `<video controls>` for video extensions, based on
   `stream-local-file`'s reported MIME type or a simple extension check.
3. **Backend: implement `GET /agentmux/stream-local-file?path=...`**
   (`agentmux-srv/src/server/files.rs`, replacing its current
   `stub_501` mapping) — `std::fs::read`/streamed read of the given path,
   `Content-Type` via an extended `mime_from_path` (add `webm`→`video/webm`,
   `mp4`→`video/mp4`, `gif`→`image/gif`, `webp`→`image/webp`), reusing the
   docsite/schema route's path-normalization and traversal-guard pattern.
   **Security: path scoping (see below) — this is the one place this
   design deliberately narrows scope vs. what the stub's naming implies.**
4. This alone is already useful: point a Media pane at
   `clips/shot-06-charts-course.webm` and watch it inside AgentMux instead
   of shelling out to File Explorer. No watcher yet — manual "reload"
   button on the pane header covers the interim.

### Phase 2 — live updates via a generalized file watcher

1. **Generalize `EditorFileWatcher`** (`editor_file_watcher.rs`) from
   "refcounted set of explicitly-opened single files" to also support
   "watch a directory, notify on any create/modify of a file matching a
   media-extension filter." This can be a sibling struct
   (`MediaFileWatcher`) sharing the same `notify`-crate/debounce approach
   rather than overloading the editor-specific one, since the editor
   watcher's refcounting model (one entry per open tab) doesn't map cleanly
   onto "one entry per Media pane's watched directory."
2. **New WPS event**, e.g. `EVENT_MEDIA_FILE_CHANGED`, scoped
   `"block:<blockId>"` exactly like `EVENT_EDITOR_FILE_CHANGED` — same
   "wake signal, not payload" shape (just the changed path; the frontend
   re-fetches via the Phase 1 endpoint). When watching a directory, the
   handler picks "most recently modified matching file" as the new target
   unless the pane is pinned to a specific filename.
3. **Frontend subscribe/reload**, same shape as the Editor live-reload
   design: `waveEventSubscribe({eventType: EVENT_MEDIA_FILE_CHANGED, scope:
   makeORef("block", blockId)})` in `MediaViewModel`, dispatching a reload
   of the `<img>`/`<video>` `src`. **Apply the crossfade/transition lesson
   from the tool-preview jerk analysis here** — swapping `src` on a live
   `<video>` element already causes a visible flash/reset; wrap it in a
   deliberate transition (e.g. fade out old frame, load new, fade in)
   rather than an instant swap, and don't couple a "loading" spinner state
   change to the same tick as the content swap.
4. End state: point a Media pane at the steamboat-rescue project's `clips/`
   directory; the moment a new `.webm` is downloaded after a Comfy Cloud
   job completes, the pane updates on its own.

### Security: path scoping — corrected after checking `editor_handlers.rs` directly

**Revised from the original draft of this section.** That draft assumed the
Editor pane's `GetEditorRootsCommand`/`ListEditorDirCommand` root list is an
enforced allowlist and proposed reusing it to scope `stream-local-file`.
Checking `agentmux-srv/src/server/editor_handlers.rs:47-53` directly shows
that's wrong: the file's own comment states plainly that root list is **"root
scoping, not a sandbox"** — `listeditordir`/`readeditorfile` already "serve
any absolute path the frontend sends," with **macOS TCC as the actual gate**
for protected locations, not any in-app allowlist. There is no existing
enforced-root mechanism to reuse, so the original recommendation doesn't map
onto anything real in this codebase.

**Corrected design: match `readeditorfile`'s existing, already-shipped
posture exactly**, rather than inventing a new, stricter model for this one
route alone (a media-viewing route being more locked-down than the file
editor's own read path would be an inconsistent, confusing security
boundary, not a stronger one):

- `expand_home_dir_safe(&path)` (`agentmux-srv/src/backend/base.rs`, already
  used by every `editor_handlers.rs` path-taking command) to resolve `~`.
- A size guard, same *pattern* as `readeditorfile`
  (`editor_handlers.rs:259-263`) but a **larger ceiling**, not the same
  10MB number — `readeditorfile`'s 10MB cap was chosen for text files
  loaded into a code editor, and doesn't fit video: this session's own
  real output clips ranged 6-28MB for a few seconds of 1920x1080 footage,
  so 10MB would reject real content on day one. Use a generous cap sized
  for local video (e.g. 500MB) instead of copying the editor's number
  verbatim — the guard's purpose (don't let a client request an
  absurdly large file into memory) still applies, it just needs recalibrating
  for the content type this route actually serves.
- No additional allowlist. This is a single-user local desktop app; the
  existing `readeditorfile`/`writeeditorfile` endpoints already establish
  that "serve any absolute local path the frontend requests" is this
  codebase's accepted posture for local file access, gated by OS-level
  permissions rather than an in-app sandbox. `stream-local-file` should be
  consistent with that, not a stricter one-off.

## Open questions

1. **File-picker UX**: should the Media pane's target-path input reuse the
   Editor pane's file-tree component directly (import/share it), or is a
   simpler flat path-input + "browse" dialog enough for v1? Reusing the
   Editor's tree is more consistent but couples two panes' state; worth a
   quick spike before committing.
2. **Directory-mode "most recent file" heuristic**: mtime-based "most
   recently modified" is simple but can surprise a user who's mid-review of
   an older clip when a new one lands underneath them. Consider an explicit
   "pin to this file" toggle vs. "always follow latest in this directory"
   as two distinct modes, defaulting to pinned-to-a-specific-file (safer,
   less surprising) with directory-follow as an opt-in.
3. **Extension filter list**: what counts as "media" for directory-watch
   purposes — hardcode a fixed set (png/jpg/jpeg/webp/gif/webm/mp4), or
   make it pane-configurable? Lean toward a fixed sane default with no
   configurability for v1, matching the project's general preference for
   not building configurability nothing has asked for yet.
4. **Remote/networked agents**: does `stream-local-file` need to account
   for an agent whose workspace isn't on the same machine as the AgentMux
   frontend (per the `TRUST=network-claimed` jekt distinction documented in
   `CLAUDE.md`)? If so, this design's "local filesystem read" assumption
   doesn't hold and the media would need to come through the agent's own
   channel instead of a direct backend filesystem read — worth confirming
   scope (local-agent-only for v1?) before Phase 1 implementation starts.
5. **Video seek/scrub while a file is still being watched**: if a `<video>`
   element's `src` is swapped out from under an in-progress scrub/playback
   (Phase 2), what's the right UX — pause and let the user manually
   acknowledge, or auto-jump to the new file and reset playback position?
   Leans toward "don't yank playback out from under an actively-playing
   video; show a non-intrusive 'updated' indicator and let the user click
   to load the new version," consistent with the Editor live-reload spec's
   "never auto-clobber a dirty/active state" principle — but this needs a
   product decision, not just an engineering default.

## Implementation phases

**Phase 1 — static pane, backend route, no watcher.** Widget registration,
`MediaViewModel`/`MediaView`, `stream-local-file` handler with MIME
extension + path scoping. Independently useful and shippable; also fixes
the two pre-existing dead-code call sites (`termsticker.tsx`,
`markdown-util.ts`/`waveutil.ts`) that already assumed this route worked.

**Phase 2 — live directory/file watching.** `MediaFileWatcher`,
`EVENT_MEDIA_FILE_CHANGED`, frontend subscribe + transition-aware reload.

**Phase 3 (stretch, not scoped here)** — gallery/grid view for a watched
directory's full history rather than "most recent file only," if that
turns out to be wanted once Phase 1/2 are in real use.

## Post-implementation corrections

Real review (ReAgent + Codex, both against the initial PR #2299 commit)
caught two things this design got wrong that weren't visible from reading
the code alone — worth recording since they'd bite any similar
"stream a local file to an `<img>`/`<video>` element" design in this
codebase, not just this one:

1. **`<img src>`/`<video src>` cannot carry the `X-AuthKey` header
   `stream-local-file` requires.** This design's Phase 1 write-up above
   just says "renders `<img>` for image extensions... based on
   `stream-local-file`'s reported MIME type," implicitly assuming a direct
   URL assignment works — it doesn't. `stream-local-file` sits in
   `authed_routes`, and the query-string `?authkey=` fallback is
   deliberately restricted to the `/ws` upgrade route only (2026-05-11
   security audit, C3 — see `auth_middleware`'s comment in
   `agentmux-srv/src/server/mod.rs`). Every direct-URL `<img>`/`<video>`
   request would 401 silently. **Fix:** fetch the bytes in JS with the
   header (same pattern `fetchWaveFile` in `wave-file.ts` already uses),
   and hand the element a `URL.createObjectURL(blob)` instead of the raw
   endpoint URL. This also retroactively explains why `termsticker.tsx`'s
   pre-existing `stream-local-file` call site was dead code in the first
   place — it has the identical bug and was never exercised, so nothing
   ever surfaced it.
2. **`std::fs::read` inside an async Axum handler blocks a shared Tokio
   worker thread and fully materializes the file in memory** — a real
   problem at this route's size ceiling (500MB) and given the pane's
   live-update re-fetch pattern. Fixed by switching to `tokio::fs::File` +
   `tokio_util::io::ReaderStream` + `axum::body::Body::from_stream` (needed
   adding tokio-util's `io` feature). No change to the route's external
   contract, purely an internal fix.

Also fixed from the same review pass, smaller: the live-update WPS handler
didn't clear a stale "no files yet" error message when a real file arrived
(both render branches gated on `!errorMsg()`, so the message would stick
forever), and setting `displayPath` to a value equal to its current value
is a no-op for a Solid signal — a pipeline that overwrites a *stable*
filename in place would never visibly refresh. Fixed with an explicit
`revision` counter bumped on every change event, decoupled from whether
the path string itself changed.

## Files

| File | Relevance |
|------|-----------|
| `agentmux-srv/src/config/widgets.json` | Canonical widget list — new `defwidget@media` entry |
| `frontend/app/block/block-registry.ts` | viewType → ViewModel registration point |
| `frontend/app/block/blockutil.tsx` | Fallback icon/label maps |
| `frontend/types/custom.d.ts:481-510` | `ViewModel` interface new `MediaViewModel` implements |
| `agentmux-srv/src/server/mod.rs:268-271` | Route table — `stream-local-file`'s current `stub_501` mapping to replace |
| `agentmux-srv/src/server/files.rs:26-170` | `handle_wave_file`, `handle_docsite`/`handle_schema`, and `mime_from_path` — the patterns the new handler copies (MIME mapping, traversal guards) |
| `agentmux-srv/src/backend/editor_file_watcher.rs` | Watcher pattern to generalize into `MediaFileWatcher` |
| `frontend/app/store/wps.ts` | `waveEventSubscribe`, `getFileSubject` — the two push mechanisms considered (subscribe-and-reload chosen; byte-stream explicitly not used, see Non-goals) |
| `frontend/app/view/term/termsticker.tsx`, `frontend/app/element/markdown-util.ts`, `frontend/util/waveutil.ts` | Existing dead code already assuming `stream-local-file`/`stream-file` work — fixed as a side effect of Phase 1 |
| `frontend/app/view/agent/components/ToolBlock.tsx`, `ToolOverlayLog.tsx` | Tool-preview precedent for live-updating pane content; also the source of the running→completed jerk lesson to avoid repeating |
| `docs/analysis/ANALYSIS_TOOL_PREVIEW_RUNNING_TO_COMPLETED_JERK_2026_07_05.md` | UI-transition pitfall this design should design around from the start |
| `docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md` | Closest existing precedent; Phase 2 directly generalizes its watcher/event/subscribe pattern |
