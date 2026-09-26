# SPEC: Paste and drop images into the agent pane input

**Date:** 2026-09-26
**Status:** proposed — nothing in this spec is implemented. Written against `main` @ `be7e05af5`. Every file:line citation was read on that commit; spot-verify before trusting.
**Author:** Korp@narko
**Related:** `docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md` (today's drop → copy-to-cwd → `@name` flow, which this spec narrows for images), `docs/specs/SPEC_UNIFIED_CLIPBOARD_2026_05_18.md` (text-only clipboard bridge; §6 puts images out of scope), `docs/retro/retro-md-drop-window-hijack-and-55-6-relaunch-failure-2026-08-16.md` (why drop handling is guarded at window level).

---

## 0. The ask

> we want to introduce image paste/dnd .. when user has an image in the clipboard and presses paste (keyboard or rightclick men) into the agent pane input, a thumbnail of the image shows in line (use best practice for how the UX is designed) The image is copied into a good place (find best practice) we essentially want the standard experience. when dragging an image, or set of images onto the agent pane, it shows as thumbnails in the input box. We support up to 128 files/1GB (group into a truncated visual form once it grows a certain size) in one prompt, with heavy processing including a progress bar. also include information about size in a good place.. research best practices, write spec to file .

## 1. What happens today

Nothing in the app handles images. The prompt is one plain string all the way from the text box to each CLI.

| Piece | Today | Where |
|---|---|---|
| Composer | A plain uncontrolled `<textarea class="agent-input">`. Handlers: keydown, input, composition, context menu. **No `onPaste`, no `onDrop`.** Drafts are strings in a per-pane `composerDrafts` Map. | `frontend/app/view/agent/components/AgentFooter.tsx:1140-1165`, `:55` |
| Send | `handleSend` reads the textarea value → `onSendMessage(message)` → `useAgentCommands.sendMessage` → `RpcApi.AgentInputCommand({blockid, message, message_id})`. | `AgentFooter.tsx:855-895`, `agent-view.tsx:1846-1894`, `hooks/useAgentCommands.ts:945, 1535-1539` |
| RPC payload | `CommandAgentInputData` has `message: string`, `message_id?`, `hidden?`. **No attachments field.** | `frontend/types/rpc/CommandAgentInputData.ts`, `agentmux-srv/src/backend/rpc_types/block.rs:367` |
| Ctrl+V with an image | Browser default text paste. **An image does nothing.** | — |
| Right-click → Paste | Menu item is `{label:"Paste", role:"paste"}` with no click handler. Nothing in the JS menu path handles `role`, so **Paste from the menu appears to do nothing, even for text** (read from code; confirm live). | `frontend/app/store/contextmenu.ts:151`, `frontend/util/cef-api.ts:410-413` |
| Clipboard bridge | Text only on every OS (`CF_UNICODETEXT`, `pbpaste`, `wl-paste`/`xclip`). | `agentmux-cef/src/commands/clipboard.rs` |
| Drop onto an agent pane | CEF `on_drag_enter` stashes OS paths (single slot, 5 s TTL). On drop, the pane copies every file into the agent's `cmd:cwd` and types `@basename` into the box. No size cap, no progress, never reads file bytes. | `hooks/useAgentDropAttach.ts:109-210`, `agentmux-cef/src/client/handlers.rs:162-178`, `agentmux-cef/src/drag_stash.rs:22`, `agentmux-cef/src/commands/providers.rs:720-755` |
| Sent message in the transcript | `UserMessageBlock` renders `<pre><LinkifiedText/></pre>`. No image slot. On reload, `replayedUserMessage` drops non-string user content, so image blocks would vanish. | `UserMessageBlock.tsx:197`, `providers/translator.ts:48-54`, `claude-translator.ts:303-315` |
| Showing a local image | `GET /agentmux/stream-local-file?path=` works (500 MB cap, needs `X-AuthKey`); the Media pane fetches a blob and uses an object URL. | `agentmux-srv/src/server/files.rs:37, 199-264`, `frontend/app/view/media/media.tsx:81-103` |
| Moving bytes frontend → backend | No upload RPC. The only binary POST is voice (`POST /api/v1/voice/transcribe`). The repo never sets `DefaultBodyLimit`, so axum's **2 MB** default applies to srv routes and CEF `/ipc`. | `agentmux-srv/src/server/voice.rs:42-50` |
| Image decoding in Rust | **No `image` crate** in any `Cargo.toml`. `sha2` is already a srv dependency. | `agentmux-srv/Cargo.toml:69` |
| Nearest storage precedent | UI screenshots in `<data>/tmp/ui-screenshots/<uuid>.png`, pruned after 1 h. | `agentmux-srv/src/server/ui_handlers.rs:170-276` |
| Progress / size UI | No determinate progress bar anywhere. Five private byte formatters (`formatBytes`, `formatSize`, `formatFileSize`). Toasts via `pushNotification` support update-in-place by `id`. | `shell-drawer-menu.ts:31`, `MemoryFileCard.tsx:45`, `flash-notifications.ts:29-33` |

What each provider accepts at the stdin/protocol layer today, and what it could accept:

| Provider path | Sends today | Image input available |
|---|---|---|
| Claude, persistent (`--input-format stream-json`) | `{"type":"user","message":{"content":<string>}}` (`persistent/queue.rs:695-702`) | `content` may be an array with `{"type":"image","source":{"type":"base64",…}}` blocks. The file path can also go in the text; Claude Code's Read tool opens and resizes images. |
| Codex, subprocess (`exec --json … -`) | text on stdin (`subprocess/host_spawn.rs:243-273`) | `-i/--image <path>` argv flag (`subprocess/argv.rs` never adds it). |
| Codex, App Server | `turn/start` with a single `text` input (`app_server_protocol.rs:261-265`) | The pinned 0.154.0 schema already has `LocalImageUserInput {type:"localImage", path}` (`schema/providers/codex/app-server/0.154.0/ClientRequest.json` ~5585-5645). |
| ACP (openclaw, pi, copilot) | one `{type:"text"}` block (`acp.rs:784-795`) | ACP `image` content blocks when the agent advertises `promptCapabilities.image`. |
| Gemini / Qwen / Antigravity / Kimi | text on stdin (`providers.rs:359, 395, 438-445, 675`) | Path references in text; per-CLI support to verify (§12). |

## 2. What other products do (research digest)

- **Where thumbnails go.** Claude.ai, ChatGPT, Cursor and Copilot Chat put a row of thumbnail cards **inside the composer frame, above the text**, each with a hover × . Claude Code's TUI instead inserts an `[Image #N]` token at the cursor. Slack lets you **drag to reorder** and add alt text before sending.
- **Limits elsewhere.** claude.ai: 20 files per chat, 10 MB per image. ChatGPT: 20 MB per image. Slack: 10 files per message. **128 per prompt is well above any consumer chat app**, so the grouped view in §5.2 is not optional.
- **Model limits that force a pipeline.** Anthropic API: JPEG/PNG/GIF/WebP only; 8000 px max, but **2000 px max on both sides once a request has more than 20 images**; 10 MB per image, 32 MB per request; 100 images per request on 200k-context models. Claude Code shipped exactly this bug: images pasted mid-turn skipped its downscale, and the 21st image broke the session. OpenAI: PNG/JPEG/WebP/static GIF. Gemini also takes HEIC. EXIF is ignored by the models.
- **Chromium mechanics.** Read the `paste` event's `clipboardData.files` (no permission prompt); `navigator.clipboard.read()` needs a permission and exposes only `image/png`. Files copied in Explorer/Finder appear in `clipboardData.files` (Chrome 91+) but as bytes, not paths. Chrome cannot decode TIFF or HEIC. `CefDragData::GetFilePaths()` gives real paths on drop. Blob data lives in Chromium's browser process and can page to disk, but anything copied into an ArrayBuffer sits in the ~4 GB V8 heap.
- **Storage.** Claude Code moved pasted images **out of `~/.claude` and out of the project** into a per-session directory with a 30-day retention sweep. A Claude Code transcript that inlined base64 images grew to 3.3 GB and crashed on start. Never write into the user's working tree (accidental `git add`).
- **Accessibility.** Real `<ul>` of attachments; each remove button named "Remove *filename*"; focus moves to the neighbor after removal; one polite live-region announcement per batch, not per progress tick; undo toast instead of a confirm dialog, pausing on hover/focus (WCAG 2.2.1).

Sources (checked 2026-09-26):
- Anthropic vision limits: https://platform.claude.com/docs/en/build-with-claude/vision
- Claude Code image storage and retention: https://code.claude.com/docs/en/claude-directory ; paste keys: https://code.claude.com/docs/en/interactive-mode
- Claude Code many-image failure: https://github.com/pingdotgg/t3code/issues/13647 ; 3.3 GB transcript: https://github.com/anthropics/claude-code/issues/20470
- Stream-json image input: https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode
- Codex `localImage` / `--image`: https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md , https://developers.openai.com/codex/cli/reference
- OpenAI image input: https://developers.openai.com/api/docs/guides/images-vision ; Gemini: https://ai.google.dev/gemini-api/docs/image-understanding
- claude.ai limits: https://support.claude.com/en/articles/8241126 ; Slack reorder/alt text: https://slack.com/help/articles/201330736-Add-files-to-Slack
- Clipboard files in Chrome 91+: https://chromestatus.com/feature/5671807392677888 ; async clipboard: https://web.dev/articles/async-clipboard
- CEF drag paths: https://cef-builds.spotifycdn.com/docs/120.1/classCefDragData.html
- Accessible uploaders: https://carbondesignsystem.com/components/file-uploader/accessibility/ ; undo over confirm: https://www.nngroup.com/articles/confirmation-dialog/

Not verified from an official source: exact thumbnail sizes and drop-overlay designs in Claude.ai, ChatGPT and Cursor (third-party descriptions only), and ChatGPT's per-message image count.

## 3. Goals

1. **Ctrl+V / Cmd+V and right-click → Paste** of a clipboard image (screenshot, copied image, or files copied in Explorer/Finder) into the agent composer adds it as an attachment with a thumbnail.
2. **Dropping one or many image files (or a folder)** onto the agent pane adds them as attachments with thumbnails.
3. **Up to 128 files and 1 GB (originals) per prompt.** Past a threshold the tray collapses into a grouped "+N" form; it expands into a scrollable grid.
4. **Processing happens off the UI thread** with per-file and aggregate progress, and a cancel.
5. **Size is always visible**: running total against the limit in the tray, per-file size on each item.
6. Files are stored **outside the user's repo** in a content-addressed store with retention.
7. The agent actually receives the images in the best form its provider supports, and the sent message **shows thumbnails in the transcript, including after reload**.

## 4. Non-goals

- Non-image attachments (PDF, text, archives). Non-image files keep today's copy-to-cwd + `@name` behavior. The store and pipeline in §6 are built so PDFs can join later.
- Image editing (crop, annotate) before sending.
- Inline images inside the text itself. The composer stays a `<textarea>`; see §12 Q1.
- Rich clipboard *copy* out of AgentMux.
- Sending attachments over WAN/jekt or to messaging bridges.

## 5. UX design

### 5.1 The attachment tray

The tray lives **inside the composer frame, above the textarea**, matching Claude.ai / ChatGPT / Copilot. It appears only when there is at least one attachment.

```
┌──────────────────────────────────────────────────────────────────┐
│ [▣1][▣2][▣3][▣4][▣5][▣6][▣7][ +23 ]     30 images · 214 MB / 1 GB │
│ Can you compare the layout in 3 and 5?                            │
│                                                        [ Send ▸ ] │
└──────────────────────────────────────────────────────────────────┘
```

- **Tile:** 64×64 px, 6 px radius, `object-fit: cover`, 1 px border from the theme token for subtle borders. A small index badge (1, 2, 3…) in the top-left corner, so the text can say "image 3" and mean the same thing the agent sees (§6.6 numbers images the same way).
- **Hover / focus:** × button in the top-right; tooltip with filename, dimensions and size ("screenshot.png · 2560×1440 · 1.8 MB").
- **Click (or Enter on a focused tile):** opens a lightbox over the pane with the full image, filename, original size and dimensions, the send-copy size and dimensions ("sent as 2000×1125, 640 KB"), ←/→ to page through, Esc to close, and a Remove button.
- **Reorder:** drag a tile within the tray; Alt+←/→ on a focused tile. Order is the order the agent receives.
- **Remove all:** a "Clear" text button appears in the tray summary on hover/focus once there are 2+ attachments.
- **Undo:** removing one or all shows a toast "Removed 1 image · Undo" that pauses while hovered or focused.

### 5.2 Grouping when it grows

- **Collapsed (default):** as many tiles as fit in one row of the current pane width, minus one slot. If the count exceeds that, the last slot becomes a **"+N" tile** showing a 2×2 mosaic of the next four thumbnails dimmed under the "+N" text. The row never wraps and the composer never grows past one tray row in collapsed state.
- **Expanded:** clicking "+N" (or the summary text) expands the tray into a grid, max-height 40% of the pane, scrolling internally, with a "Show less" control. Tiles grow to 88 px and show the filename and size under each thumbnail. Tiles are rendered with `content-visibility: auto` and `contain-intrinsic-size`; thumbnails load lazily as they scroll in. 128 tiles is small enough that this avoids a virtualization library.
- The expanded/collapsed choice is per pane and remembered while the draft exists.

### 5.3 Size information

- **Summary, right-aligned in the tray header:** "30 images · 214 MB / 1 GB". Muted text normally; warning color at ≥ 80% of either limit ("118 / 128 images"); error color over the limit.
- **Per file:** in the tooltip, the expanded grid caption, and the lightbox.
- **The limit counts originals**, since that is what the user dragged. The lightbox also shows the send-copy size so the user can see what the model gets.
- One shared `formatBytes` (1024-based, one decimal from MB up: "812 KB", "1.8 MB", "1.0 GB") replaces the five private copies (§1).

### 5.4 Processing and progress

Every attachment goes through the pipeline in §6.4 (hash, decode, thumbnail, send-copy). A screenshot takes milliseconds; 128 camera photos take many seconds.

- **Per tile:** until its thumbnail is ready, a tile shows a neutral placeholder with a thin circular progress ring (determinate while bytes are copied or uploaded, indeterminate while decoding). Filename initials fill the placeholder so the user can tell tiles apart.
- **Aggregate:** while anything is processing, the tray header shows a **determinate bar** under the summary: "Processing 37 of 128 · 412 MB of 1 GB" with a **Cancel** button. Progress is by bytes, not by file count, so one huge file doesn't stall the bar at 99%.
- **Send while processing:** Send is disabled with the tooltip "Waiting for 12 images to finish processing". Enter shows the same text as a transient hint under the tray. The user can keep typing, add more, or remove items.
- **Screen readers:** one polite announcement when a batch is accepted ("30 images added, 2 skipped") and one when processing ends. No per-tick announcements. The bar has `role="progressbar"` with `aria-valuenow`.

### 5.5 Dropping

- The whole agent pane is the drop target (today's `useAgentDropAttach` target). On drag-enter the overlay reads the stashed OS paths (CEF has them at drag-enter, §1) and asks the backend for a cheap `stat` preview, so the overlay can say something specific **before** the drop:
  - "Attach 12 images (48.2 MB)"
  - "Attach 12 images · 3 other files will be copied to the working folder" (mixed drop; non-images keep today's behavior)
  - "Too many: 140 images. You can attach 116 more." (drop still allowed; the first 116 by name order are taken, the rest listed in a toast)
- Folders are expanded recursively, images only, capped by the limits.
- A drop onto a pane whose agent is **busy** still attaches to the draft; nothing is sent.

### 5.6 Pasting

- **Ctrl/Cmd+V in the composer:** if the clipboard holds image data or a file list, attach them and paste any text normally. Image and text in one paste (e.g. copied from a web page) gives both.
- **Right-click → Paste:** runs the same code path through the native bridge (§6.2), since a menu click has no `paste` event. This also fixes text paste from the menu, which appears broken today (§1).
- **Pasted screenshots get a readable name:** `Pasted image 2026-09-26 14.03.12.png`.
- **Nothing pasteable:** if Ctrl+V finds no text and no image, show "Nothing to paste. The clipboard is empty or holds a format AgentMux can't read." Windows screenshot tools are the known source of this.

### 5.7 Errors and limits

| Case | Behavior |
|---|---|
| Unsupported type (not an image) in a paste | Skip it; toast "2 files skipped: not images". |
| HEIC / HEIF | Phase 1: tile in error state, "HEIC isn't supported yet — export as JPEG". Phase 3: converted (§10). |
| TIFF / BMP | Accepted and converted to PNG for sending. |
| SVG | Rasterized to PNG by the backend; never inserted into the DOM as SVG markup. |
| Animated GIF | Accepted; tile shows the first frame; the send-copy is the first frame as PNG (what every model uses anyway). Badge "GIF · first frame". |
| Corrupt or undecodable | Tile in error state with the decoder's message and a Remove button; it doesn't block Send (it is excluded, and the summary says "1 failed"). |
| Over 128 files or 1 GB | Accept in order up to the limit; the rest are listed in a toast. Never silently drop. |
| Image larger than 16384 px on a side or 200 megapixels | Rejected before full decode (decompression-bomb guard, §8). |
| Duplicate (same SHA-256 already in this draft) | Not added again; the existing tile flashes. |
| Provider can't take images | Tray header says "This agent can't see images — they'll be sent as file paths" (§6.6). |

### 5.8 In the transcript

- The sent user message shows the same thumbnail strip above its text, read-only, collapsed the same way ("+N"), clicking opens the lightbox.
- Thumbnails are served from the attachment store (§6.5), not from base64 in the message, so a 128-image message doesn't bloat the document or the block file.
- After reload, replay reads attachment references from AgentMux's own record (§6.7), not from the provider's transcript.

### 5.9 Keyboard

- Tab moves from the textarea into the tray; ←/→ between tiles; Delete/Backspace removes the focused tile and focuses its neighbor (or the textarea when none remain); Enter opens the lightbox.
- In the textarea, **Backspace at position 0 with no selection** focuses the last tile (Slack's pattern) instead of deleting it.

## 6. Architecture

### 6.1 Principle: bytes stay out of the renderer

A 1 GB batch must not pass through the V8 heap, the JSON RPC channel (2 MB axum default, 5 s handler timeout), or `/ipc`. Every ingestion path ends as **"backend reads a file from a path"** except one fallback that streams a Blob as an HTTP request body (Chromium streams Blob bodies from the browser process, never copying them into V8).

### 6.2 Ingestion paths

| Source | How | Bytes cross the renderer? |
|---|---|---|
| Drop (CEF) | Existing `consume_drag_paths` → new srv RPC `attachments.ingest` with `{paths}` | No |
| Ctrl+V, right-click Paste (CEF) | New CEF IPC `read_clipboard_attachments` → returns `{paths}` for a file list (Windows `CF_HDROP`, macOS `NSFilenamesPboardType`, Linux `text/uri-list`), or writes clipboard image data (Windows PNG/`CF_DIBV5`, macOS PNG/TIFF, Linux `image/png` via `wl-paste`/`xclip -t`) to `<store>/incoming/<uuid>.png` and returns that path → `attachments.ingest` | No |
| Ctrl+V fallback (native read returned nothing but the `paste` event has `clipboardData.files`), and the non-CEF dev host | `fetch(POST /api/v1/attachments/upload, body: file)` per file, streamed | Blob only, never an ArrayBuffer |

For Ctrl+V the composer's `onPaste` handler reads `clipboardData.files` / image items **synchronously** and **never calls `preventDefault()`**. A textarea's default paste only ever inserts text, so letting it run is exactly right for the text part: the browser inserts it with its own selection handling and native undo, and nothing has to re-insert it (no `execCommand`). The handler only adds what the default paste ignores: it hands the image files (or, for a copied file list, the native reader's paths) to the attachment pipeline. A paste with images and no text inserts nothing into the textarea, which is also the default.

The upload route is the only new HTTP body path. It reads the raw body as a stream (axum's `DefaultBodyLimit` only governs buffering extractors, so it does not cap this), **counts bytes as it writes**, and aborts and deletes the partial file the moment the count passes the per-file cap. It hashes while writing and requires `X-AuthKey` like `stream-local-file`.

### 6.3 Storage

Location: `get_mux_data_dir()/attachments/` (per channel, like other srv data; `agentmux-srv/src/backend/base.rs:83-93`). Not the agent's `cmd:cwd`, never the user's repo.

```
attachments/
  blobs/ab/abcdef…(sha256).png        original bytes, content-addressed, read-only
  derived/ab/abcdef….<fp>.thumb.jpg|png   256 px long edge, for tiles and transcript
  derived/ab/abcdef….<fp>.send.png|jpg    what the agent gets (§6.4)
  incoming/<uuid>.<ext>               in-flight paste/upload, renamed into blobs/ when hashed
  derived/ab/abcdef….<fp>.json            dimensions, formats and sizes of the three files
```

- **Content addressing:** duplicates within and across prompts cost nothing, and the ID in every API is the hex SHA-256. IDs are validated as 64 hex chars before any path join.
- **Permissions:** directory is user-only (0700 on Unix; default user-profile ACL on Windows under `~/.agentmux`).
- **Retention is by last use, not by reference tracking.** Ingesting or sending an attachment refreshes the modification time of its files, and sending also writes a small `<id>.sent` marker next to its derived files. A sweep on srv start and every 6 h deletes an attachment's files when they haven't been touched for `attachments:retentiondays` (default 30, the same rule Claude Code uses) if it was ever sent, or for **7 days** if it never was. There is no reference table, so nothing can pin a blob forever, and an abandoned draft (lost on restart) is gone within a week instead of lingering for the full window. A draft left unsent in an open pane for more than 7 days loses its images; its tiles then show "no longer available". The cost is that a sent message older than the retention window shows "image no longer available" in place of its thumbnails. `incoming/` files older than 1 h are deleted (screenshot-dir precedent).

### 6.4 Processing pipeline (srv, Rust)

Runs on blocking threads. **Decode memory is capped** by a 2 GB budget: before decoding, each job reads the image header and reserves width × height × 8 bytes (the decoded RGBA frame plus one working copy). The aggregate of all running decodes therefore never exceeds 2 GB; a 200-megapixel image reserves ~1.6 GB and runs alone, and a job bigger than the whole budget reserves all of it. Separately, at most `max(1, cores/2)` jobs run at once, so 128 files don't starve the server of CPU.

**Derived files are keyed by a transform fingerprint** `<fp>` = pipeline version + send edge (e.g. `v1-e2000`), not by the original's hash alone, because their bytes depend on `attachments:sendmaxedge` and the encoder. Changing the setting makes the old derived files unused (the sweep removes them); the next use of that attachment re-derives from the stored original.

1. **Copy + hash:** stream the source into `incoming/`, computing SHA-256 on the fly (`sha2`, already a dependency). If the blob already exists, skip the copy. Progress events by bytes.
2. **Sniff by magic bytes**, not extension (catches HEIC named `.jpg`).
3. **Header-only dimension check** before decode; reject past 16384 px per side / 200 MP.
4. **Decode** with the `image` crate (new dependency: PNG, JPEG, GIF, WebP, BMP, TIFF) and apply EXIF orientation. SVG via `resvg` (new dependency) at 2000 px long edge.
5. **Thumbnail:** 256 px long edge, WebP.
6. **Send-copy:** long edge ≤ **2000 px** (fits Anthropic's many-image rule, OpenAI, Gemini), metadata stripped (EXIF, including GPS, never leaves the machine). PNG when the source has alpha or looks like a screenshot (PNG source, ≤ 256 distinct colors in a sample); otherwise JPEG quality 85. Re-encode down to ≤ **5 MB** if needed (Bedrock/Vertex cap). If the original already satisfies every rule and has no metadata, the send-copy is a hard link to it.
7. Emit `attachment:ready` with `{id, name, mime, bytes, width, height, send_bytes, send_width, send_height, thumb_url}`.

Cancel sets a per-batch flag that **running jobs check too**: between copy chunks (the copy stops and its `incoming/` file is closed, then deleted, so Windows can remove it), before waiting on the memory budget, and before decoding. A decode already in progress finishes (it can't be interrupted) but its result is reported as cancelled, not ready. Queued jobs report cancelled without starting. Blobs already stored stay (another draft may use the same bytes) and the sweep handles them.

### 6.5 Events and serving

- Progress travels on the existing event bus as `attachment:progress` `{batch_id, id?, done_bytes, total_bytes, done_files, total_files}`, throttled to ~10 per second per batch, and `attachment:ready` / `attachment:failed`.
- Thumbnails and send-copies are served by a new `GET /api/v1/attachments/<id>/(thumb|send|original)` with `X-AuthKey`, returning `image/*` with `Cache-Control: no-cache` and an `ETag` of `<id>-<fp>`: the original is immutable by hash, but thumb and send-copy change when the fingerprint does, so they must not be cached forever under the same URL. The frontend fetches to a blob URL (the Media pane's pattern) and revokes it when the tile unmounts.

### 6.6 Protocol and provider delivery

`CommandAgentInputData` gains:

```ts
attachments?: { id: string; name: string }[]   // in tray order
```

The backend resolves each ID to its send-copy path, then per provider:

| Provider | Delivery |
|---|---|
| Claude (persistent stream-json) | **Hybrid.** The first images whose base64 fits a budget (default: up to 20 images and 20 MB base64) go as `image` content blocks, in order, before the text block. **All** images are also listed in a text preamble with their paths (`Attached images (read with the Read tool if not shown above): 1. /…/abc….png (screenshot.png) …`). This keeps the request under the 32 MB API cap and keeps Claude Code's own JSONL from ballooning, while every image stays reachable. |
| Claude (container mode) | Send-copies are copied into the container at `/tmp/agentmux-attachments/<id>.<ext>` with the existing tar upload (`backend/container.rs:730-775`); paths in the preamble use those. |
| Codex App Server | One `localImage {path}` input per image, then the text input. |
| Codex subprocess (`exec`) | `-i <path>` per image on the turn's argv, plus the numbered path list in text. |
| ACP | `image` blocks if `promptCapabilities.image`, otherwise `resource_link` blocks; plus the path list. |
| Gemini, Qwen, Antigravity, Kimi | Numbered path list in text (`@path` syntax where the CLI supports it). |

The numbering in every preamble matches the tile badges (§5.1).

**Stdin-size note:** AgentMux's own persistent controller stores the exact stdin line in the block file and resume retry batch (`persistent/queue.rs:660-675`, `persistent/input.rs:97-102`). With inline images that line is up to ~20 MB. The stored copy must **replace each base64 `data` field with the attachment ID** before persisting; the retry path re-inflates from the store.

### 6.7 Recording and replay

- `UserMessageNode` gains `attachments?: AttachmentRef[]` (`frontend/app/view/agent/types.ts:378`).
- AgentMux's own user-message record for the block stores the refs (not base64), so replay and `replayedUserMessage` (`providers/translator.ts:48-54`) rebuild the strip without parsing the provider's transcript. Claude's translator must also stop dropping array-form user content that contains image blocks (`claude-translator.ts:303-315`), rendering it as refs when the data matches a stored blob hash and as a generic "image" chip otherwise.
- Drafts: `composerDrafts` holds `{text, attachments}` instead of a string (`AgentFooter.tsx:55`). Drafts are in memory, so an app restart drops unsent attachments from the tray. No startup reconciliation is needed: nothing records draft references on disk, and never-sent attachments age out after 7 days (§6.3).

## 7. Settings

| Key | Default | Meaning |
|---|---|---|
| `attachments:enabled` | `true` | Master switch; `false` restores today's behavior exactly. |
| `attachments:maxfiles` | `128` | Per prompt. |
| `attachments:maxtotalmb` | `1024` | Per prompt, originals. Per-file cap equals this. |
| `attachments:sendmaxedge` | `2000` | Long edge of the send-copy. |
| `attachments:claudeinlinemax` | `20` | Max images sent as inline blocks to Claude (0 = paths only). |
| `attachments:retentiondays` | `30` | Days since last use before an attachment's files are deleted. |

Existing `dnd:enabled` still gates drop as a whole; `dnd:agentinserttoken` now applies only to non-image files.

## 8. Security

- **IDs are hashes, validated as 64 lowercase hex** before any filesystem use; the serving route never takes a path.
- **Decompression bombs:** header dimension check before decode (§6.4 step 3); decode runs with the `image` crate's memory limits set.
- **SVG** is rasterized in the backend and never placed in the DOM as markup.
- **Metadata:** send-copies strip EXIF/XMP/ICC text chunks. A phone photo's GPS location is not sent to a model provider.
- **Auth:** upload and serving routes require `X-AuthKey`, same as `stream-local-file`.
- **Clipboard file lists** may name paths the user can read but the agent's sandbox couldn't. That is the point of attaching; it is the same trust as today's drop copy. Paths are never followed as symlinks out of a dropped folder.
- **Storage** is outside the working tree, so attachments can't be committed by the agent's `git add -A`.

## 9. Tests

- **Rust, srv:** pipeline unit tests over fixture images (PNG with alpha, EXIF-rotated JPEG, CMYK JPEG, animated GIF, 16-bit TIFF, BMP, SVG, a HEIC renamed `.jpg`, a truncated PNG, a 20000×20000 header). Assert: orientation applied, long edge ≤ 2000, ≤ 5 MB, no EXIF in output, bomb rejected before decode, HEIC reported not crashed. Dedup: same bytes twice → one blob. Retention sweep deletes files older than the window and keeps recently touched ones. Upload route: body larger than the cap is rejected mid-stream; missing auth → 401.
- **Rust, delivery:** per provider, the exact stdin line / argv / JSON-RPC params for 0, 1, 25 attachments; the Claude hybrid budget split; the persisted Claude line contains IDs, not base64.
- **Vitest:** tray grouping math (how many tiles fit per width; "+N" count); summary text and warn/error thresholds; keyboard removal focus rules; draft save/restore with attachments; `formatBytes`.
- **CEF:** `read_clipboard_attachments` on Windows for a PNG screenshot, a `CF_DIBV5` bitmap, and an Explorer file list.
- **UI screenshot script** (`scripts/ui-screenshots/`): tray with 1, 7, 30 and 128 items; processing state; error tile; transcript strip after reload.

## 10. Phases

1. **Store, pipeline, drop.** §6.3–6.5, drop ingestion, the tray with thumbnails, size summary, progress and limits, Claude and Codex (both controllers) delivery, transcript strip and replay. Right-click Paste text fix.
2. **Paste.** `read_clipboard_attachments` (Windows first, then macOS, Linux), `onPaste`, the upload fallback route, right-click Paste for images.
3. **Breadth and polish.** ACP and remaining providers, container-mode copy, HEIC (Windows WIC / macOS ImageIO, or libheif), reorder by drag, lightbox paging.

## 11. Files touched (expected)

- `frontend/app/view/agent/components/AgentFooter.tsx` — `onPaste`, tray mount, drafts shape, Send gating.
- `frontend/app/view/agent/components/AttachmentTray.tsx` (new), `AttachmentLightbox.tsx` (new), `attachment-tray.scss` (new).
- `frontend/app/view/agent/hooks/useAgentDropAttach.ts` — split images from other files; overlay preview.
- `frontend/app/view/agent/hooks/useAgentCommands.ts` — carry attachments through `sendMessage` and the pending queue.
- `frontend/app/view/agent/components/UserMessageBlock.tsx`, `types.ts`, `providers/translator.ts`, `providers/claude-translator.ts`.
- `frontend/app/store/contextmenu.ts` — Paste handler.
- `frontend/util/format-bytes.ts` (new; replaces five private formatters).
- `agentmux-srv/src/backend/attachments/` (new: store, pipeline, refs, sweep), `server/attachments.rs` (new routes), `rpc_types/block.rs`, `persistent/queue.rs`, `persistent/input.rs`, `subprocess/argv.rs`, `app_server_protocol.rs`, `acp.rs`, `container.rs`.
- `agentmux-cef/src/commands/clipboard.rs`, `agentmux-cef/src/ipc.rs`.
- `agentmux-srv/Cargo.toml` — `image`, `resvg`.
- `schema/settings.json` — §7 keys.

## 12. Open questions

Questions 1–3 were answered by the repo owner on 2026-09-26 by accepting this spec's defaults: tray above the text, non-image files keep today's behavior, and the limit counts originals. They are kept below for the reasoning.

1. **"Thumbnail shows in line."** This spec reads it as "in the input box" and uses a tray above the text, the pattern every major chat app uses. The alternative is Claude Code's TUI style: an inline `[Image 3]` token at the cursor, with the thumbnail in the tray. True inline images in the text need a contenteditable composer, a much larger change to a component that IME and undo handling depend on. The tile index badges plus the numbered preamble give most of the benefit. Confirm which was meant.
2. **Non-image files dropped together with images:** keep today's copy-to-cwd + `@name` for them (this spec), or treat everything dropped as an attachment?
3. **Should the 1 GB limit count originals (this spec) or send-copies?** Originals match what the user sees in Explorer; send-copies are usually 10–50× smaller.
4. **Claude inline budget.** 20 images inline is a guess balancing "the model sees them immediately" against transcript size and the 32 MB cap. Paths-only is the conservative alternative.
5. **Verify against pinned CLI versions** (claude 2.1.280, codex 0.154.0): stream-json image blocks on stdin; `codex exec --json -i` (a hang with that combination was reported in Oct 2025, openai/codex#5773); Gemini/Qwen/Kimi `@path` image support.
6. **`DragOverlay` reactivity:** `frontend/app/element/dragoverlay.tsx:11` destructures props, which in SolidJS reads them once. The drop overlay may never show today. Check before building §5.5 on top of it.
