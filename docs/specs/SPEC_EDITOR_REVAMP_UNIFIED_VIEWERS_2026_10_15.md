# SPEC: Editor revamp — one pane for every file (PDF, images, audio/video, binary, large files), and the Media pane folded into it

**Date:** 2026-10-15
**Status:** active — phase 1 built in #5016, phase 2 (PDF) in #5025 (see "Phase 1 as built" and "Phase 2 as built" at the end); phases 3–5 to follow. Written against `main` @ `521daa2ad`.
**Author:** Korp@narko
**Requested by:** the repo owner:
> *"does agentmux editor support pdf? lets write a quick spec to support pdfs and images in the editor. we need to get the editor wide-ranging editor access, including binary if it detects it, images, etc. we want wide coverage, research latest best practices."*
>
> *"we dont need a media pane .. everything can be done in the editor. Any sort of special controls can be loaded on context, a separate media is pointless. lets include that in an editor feature revamp."*
>
> *"the playback and image features we added for media should just go to editor."*

**Builds on (and does not contradict):**
- [`SPEC_DOCUMENT_TABS_2026_10_02.md`](SPEC_DOCUMENT_TABS_2026_10_02.md) §6.1 and §6.3, and [`PLAN_EDITOR_ON_SHARED_DOC_TABS_2026_10_10.md`](PLAN_EDITOR_ON_SHARED_DOC_TABS_2026_10_10.md). Editor and Media already share the document-tab layer. This spec makes the Editor the only document pane.
- [`SPEC_MEDIA_PANE_2026_07_26.md`](SPEC_MEDIA_PANE_2026_07_26.md), [`SPEC_MEDIA_PANE_V4_MCP_OPEN_TOOL_2026_08_03.md`](SPEC_MEDIA_PANE_V4_MCP_OPEN_TOOL_2026_08_03.md), [`SPEC_MEDIA_OPEN_SHOWS_THE_MEDIA_PANE_2026_10_12.md`](SPEC_MEDIA_OPEN_SHOWS_THE_MEDIA_PANE_2026_10_12.md), [`SPEC_EDITOR_MEDIA_SPLIT_OPENS_EMPTY_2026_10_10.md`](SPEC_EDITOR_MEDIA_SPLIT_OPENS_EMPTY_2026_10_10.md), and [`../reports/REPORT_MEDIA_VIEWER_ZOOM_AND_FEATURES_2026_10_09.md`](../reports/REPORT_MEDIA_VIEWER_ZOOM_AND_FEATURES_2026_10_09.md). Every behaviour these specs shipped carries over (§4.12, §4.13).
- [`SPEC_EDITOR_FILE_ENCODINGS_2026_06_17.md`](SPEC_EDITOR_FILE_ENCODINGS_2026_06_17.md): the text path and its encodings are unchanged; binary detection now runs before them.
- [`SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md`](SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md): its watcher is reused for every viewer (§4.9).
- [`SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md`](SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md) §5: its content-first file classifier is reused for viewer selection (§4.2).
- [`SPEC_EDITOR_MD_PREVIEW_PANEL_2026_06_21.md`](SPEC_EDITOR_MD_PREVIEW_PANEL_2026_06_21.md), [`SPEC_EDITOR_FILE_TREE_OPEN_ACTIONS_2026_07_12.md`](SPEC_EDITOR_FILE_TREE_OPEN_ACTIONS_2026_07_12.md), and [`SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md`](SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md).

---

## 1. Summary

**Does the Editor support PDF today? No.**
- The Editor sends every file to CodeMirror 6 as text.
- A PDF is either refused as binary (there is a NUL byte in its first 4 KB) or shown as garbled text. Which one happens depends on the file's first bytes (§2.6).
- No pane in the app renders a PDF except the **Browser** pane. Chromium's built-in viewer was enabled there on 2026-10-14 (#4938).
- The Media pane refuses PDFs when you drop them or pick them. If an agent opens one with `OpenMedia`, the pane goes blank.
- From the Files pane, a PDF opens in the OS's default app.

Images, video and audio work only in the separate **Media** pane. The Editor refuses them as binary.

**What this spec proposes:**
1. **One pane for every file.**
   - The Editor opens any file. It picks a **viewer** for it from a registry, using the file's content first, then its extension, then the user's choice:
     - text (CodeMirror, as today)
     - markdown preview (as today)
     - image
     - audio/video
     - PDF
     - hex
     - large-text
     - an info card for anything else
   - Each viewer brings its own controls into the Editor's chrome while its tab is in front: zoom and fit, page navigation, playback, the hex inspector.
   - "Reopen with…" switches the viewer for one tab, like VS Code's "Reopen Editor With…".
2. **The Media pane is retired.**
   - Its image viewer (zoom about the cursor, pan, fit and actual size, pixelated rendering, keys, keeping the zoom across renders) and its playback (video, audio, error reporting) **move into the Editor unchanged** as the image and audio/video viewers. Nothing is rebuilt.
   - Every way into the Media pane (inline images in an agent reply, tool-result images, the Files pane, drops, `OpenMedia`, `muxsh view`, layout files) now leads to the Editor.
   - Saved `media` blocks reopen as Editors through the pane registry's existing `aliases` mechanism.
3. **PDF with pdf.js, bundled in the frontend** (`pdfjs-dist` ≥ 6.2.108; 6.4.x is current). Scripting is off. Large files load over Range requests.
   - The Chromium viewer stays the Browser pane's.
   - Reasons (§5.1): pdf.js renders inside the Editor's own layout, so it works with themes, the Editor's find bar, page commands for agents, split panes and floating panes. It also doesn't depend on a GN flag that the size-reduction spec proposes to turn off.
4. **A hex viewer we build ourselves.** It is virtualized, reads over Range requests and is read-only. The design copies Microsoft's Hex Editor. Every React hex library is unmaintained (§5.2).
5. **Binary detection on srv, before anything is read into the Editor.**
   - A cheap `fileprobe` RPC sniffs the first 8 KB. It reuses the attachment classifier and adds a UTF-16 check.
   - So the Editor no longer downloads 10 MB to find out a file is binary.

**Phase 1 is the smallest useful slice** (§6):
- the viewer registry and the probe
- the Media pane's image and playback code moved into the Editor
- an info card with "Open as text anyway / hex / with the system app"
- every Media entry point re-routed to the Editor
- the `media` alias for saved blocks

PDF is phase 2, hex is phase 3, and large files are phase 4.

---

## 2. Current state

### 2.1 The Editor
- **Engine: CodeMirror 6.** The `@codemirror/*` packages are in `package.json:81-94`, and `"codemirror"` is at `:106`. There is no Monaco. Languages are lazy-loaded in `frontend/app/view/editor/editor-view.tsx:50-88`.
- **Nothing chooses a viewer.** Every file opens in CodeMirror. Opening goes from `editor-model.ts:113` → `openFile`/`openFilePreview` (`:535-544`) → `_openFileWithMode` (`:555-626`). The extension only selects a language (`EXTENSION_MAP`, `editor-model.ts:1466-1498`; `.svg` maps to `"html"`). The fallback is `"text"` (`:1500-1513`).
- **Binary and wide-line refusal happens after the whole file is read.**
  - `sniffUnopenable` (`editor-model.ts:1437-1457`, called at `:692`) looks for U+0000 in the first 4096 characters ("This file looks binary (contains NUL bytes) and can't be displayed in the editor."), and for a first line longer than 100,000 characters.
  - The refusal shows as the full-pane "Couldn't open file" panel (`editor-view.tsx:1046-1062`). It offers only **Close tab**. There is no "open anyway" and no other viewer.
- **Markdown preview** (`editor-view.tsx:1020-1034`, `:1141-1158`) renders `<Markdown>` with no `media` options. An image inside a previewed `.md` file therefore shows as the literal text `[img:src]` (`frontend/app/element/markdown-media.tsx:487-493`).
- **File tree** (`editor-view.tsx:840-856`; actions at `editor-model.ts:1199-1303`): Open, Open to the Side, Open in New Tab, Copy Path, Copy Relative Path, Reveal in Explorer, Rename and Delete. Single-click opens a preview tab and double-click pins it (`editor-view.tsx:964-965`). There is no "Open with the system app".
- **Drop** (`frontend/app/view/editor/editor-drop.ts:13-70`): image, PDF, Office, archive, audio and video names are refused while hovering ("The editor opens text files; … isn't one"). A pathless drop is capped at 10 MB and checked for NUL.
- **Encodings** (`crates/srv/src/backend/text_encoding.rs:36-76`): BOM, then UTF-8, then `chardetng`, then `encoding_rs`. Phases 1–2 of the encodings spec are built. Phase 3 (the encoding chip and "Reopen with Encoding") is not. Saving is blocked when the decode needed replacement characters (`editor-model.ts:886-899`).
- **Live reload** (`crates/srv/src/backend/editor_file_watcher.rs:38-40`, `editor:file_changed`, 300 ms debounce; RPCs at `crates/srv/src/server/editor_handlers.rs:157-198`; frontend at `editor-model.ts:217-224`, `:281-285`, `:737-788`): clean tabs reload and dirty tabs never do. SSH files are not watched.

### 2.2 The Media pane: full inventory

Code: `frontend/app/view/media/`, about 1,940 lines including tests.

| # | What it does | Where |
|---|---|---|
| M1 | Registered pane type `media`: label "Media", icon `photo-film`, hue 120; a split opens an empty Media pane | `media.tsx:37-54`, `:19-21`; registered at `frontend/app/block/block-registry.ts:19,52` |
| M2 | Widget-bar entry `defwidget@media` (order 14, not pinned) | `crates/srv/src/config/widgets.json:228-239` |
| M3 | Document tabs on the shared layer: payload `{path, blank?, file?}`, "New Tab" empty tab, strip always shown, Ctrl+T, drag between Media panes, `media:path` follows the front tab | `media-pane.tsx:47-170`, `:254-301`; `DOC_TAB_HOSTS = ["editor","media"]` at `frontend/app/keybindings/registry.ts:24` |
| M4 | The `media:open` queue: clicks from elsewhere add tabs, drained once per request id | `media-pane.tsx:195-225`; writer `media-open.ts:50-64` |
| M5 | Picks the Media pane on screen (focused, else visible, else any, including a background tab of a stack) and reveals it without moving focus | `media-open.ts:26-44`, `:61` (`showBlockWithoutFocus`) |
| M6 | Kind by extension: images `png jpg jpeg gif webp svg`, video `webm mp4 mov`, audio `wav` | `frontend/app/element/local-media.ts:18-27`; `media-view.tsx:372-378` |
| M7 | **Image:** `<img>` from a blob URL, `object-fit: contain`, centred; decode error shows "Failed to display media (unsupported format or corrupt file)." | `media-view.tsx:426-457` |
| M8 | **Zoom/pan:** plain wheel zooms about the cursor (Ctrl+wheel is left to pane zoom); drag pans when zoomed; double-click toggles fit and actual size (or 2×); `+`/`=`, `-`/`_`, `0` fit, `1` actual size, arrows pan a tenth of the view; pan is clamped; a resize re-clamps; maths is in CSS pixels so a zoomed pane stays correct | `media-view.tsx:87-183`, `:271-283`; maths `media-zoom.ts` (unit-tested, `media-zoom.test.ts`) |
| M9 | Range from fit up to `max(4, 32·natural/fit)`; `image-rendering: pixelated` past 2 screen px per image px; cursors zoom-in, grab and grabbing; no `will-change` (it blurs) | `media-zoom.ts:30-37`; `media-view.scss:9-30` |
| M10 | "250% · Fit" badge while zoomed | `media-view.tsx:458-468`; `media-view.scss:32-38` |
| M11 | A newer render with the same dimensions keeps zoom and pan; a different size goes back to fit | `media-view.tsx:93-95`, `:178-183` |
| M12 | **Video:** `<video controls>`, fades in on `loadeddata`; errors read the `MediaError` code ("MEDIA_ERR_SRC_NOT_SUPPORTED (likely a missing/disabled codec…)") | `media-view.tsx:469-478`; `describeMediaError` `local-media.ts:107-118` |
| M13 | **Audio:** `<audio controls>`, 80% width, the same error path | `media-view.tsx:479-487` |
| M14 | Fetches with the `X-AuthKey` header into a Blob object URL (an `<img src>` can't send the header); a fetch token drops superseded loads; the URL is revoked on swap and unmount | `media-view.tsx:328-370`; `fetchMediaBlob` `local-media.ts:120-169` |
| M15 | **Follow the folder:** watches the file's directory, filtered to media extensions; a newer matching file replaces the tab's file, and its title follows; a revision counter forces a reload on an in-place overwrite | `media-view.tsx:185-236`; RPC `watchmediadir`/`unwatchmediadir` `crates/srv/src/server/editor_handlers.rs:200-235`; `crates/srv/src/backend/media_file_watcher.rs:37-39` (`media:file_changed`, 300 ms debounce) |
| M16 | Only the front tab is mounted, so a video behind it stops; switching back fetches again | `media-pane.tsx:264-297` |
| M17 | Native "open file" picker filtered to media types (svg is missing from the filter); "Pick a different file" button; click the empty state to pick | `crates/cef/src/commands/platform.rs:1037-1062`; `media-view.tsx:246-253`, `:488-518` |
| M18 | Drop: exactly one file, by name or MIME type; a pathless drop is shown from its bytes (no live updates, not restored after restart) | `media-drop.ts:36-69`; `media-view.tsx:255-269`, `:284-297`; `media-pane.tsx:155-164` |
| M19 | Empty state with the faint logo and supported-types text | `media-view.tsx:488-498`; `media-pane.tsx:265-276`; `SPEC_MEDIA_PANE_EMPTY_LOGO_2026_10_12.md` |
| M20 | MuxAPI `RunCommand` runs doc-tab actions | `media-pane.tsx:232-240` |
| M21 | Shortcut tip "media:image" in the Help pane | `frontend/app/keybindings/tips.ts:322-330` |
| M22 | Layout files export and import `media` with a `path` | `crates/srv/src/backend/layout_file.rs:512-516`, `:937-945` |
| M23 | `pane.open` builds `media:path`; refuses media on an SSH host | `crates/srv/src/server/app_api/pane.rs:271-276`, `:314-315` |
| M24 | **`OpenMedia` MCP tool:** `file`, `title`, `split`, `floating`; always a **new** pane (no reuse), no extension check | schema `crates/mcp/src/tool_schemas.rs:853-866`; handler `crates/mcp/src/tools/panes.rs:98-~170` |
| M25 | Other tools point agents at `OpenMedia` ("…or OpenMedia to show it to the user"): `UIScreenshot`, `CaptureWindow` | `crates/mcp/src/tool_schemas.rs:362`, `:692`; `crates/mcp/src/tools/ui.rs:32`; `crates/mcp/src/window_capture.rs:636` |
| M26 | Operator Config text names the Media pane (rich output: "Clicking an image opens it in a Media pane"; MuxAPI tool list) | `crates/srv/operator-config-seed.json:7`, `:17` |

**Related features that are not the Media pane but open it**:
- Inline media in agent replies (`markdown-media.tsx`). `openInMediaPane` is at `:67-72` and is called from the image card (`:211`) and the media card (`:240`).
- Tool-result images (`frontend/app/view/agent/components/ResultImages.tsx:52`).
- The Files pane (`frontend/app/view/files/files-open.ts:38-68`).

The inline player (`LocalAV`, `markdown-media.tsx:262-440`) is where the **poster frame**, the **muted start** and the **200 MB inline cap** live. Its poster comes from a 2 MB Range read through `fetchMediaRange` (`local-media.ts:171-199`). It stays in the agent pane; this spec changes only where its "open" click goes (§4.13). Its Range-read pattern is also what the Editor's video viewer adopts (§4.4.3).

### 2.3 How srv serves bytes
- **`readeditorfile`** (`crates/srv/src/server/editor_handlers.rs:466-525`) reads the whole file and decodes it. It has no Range support and no binary check. It refuses files over 10 MB from the file's metadata (`:500`, "File too large (>10MB)"). The same cap applies over SSH (`crates/srv/src/backend/fs_ops/remote.rs:310-325`, `EDITOR_MAX`). `writeeditorfile` keeps the 10 MB cap and writes only under the home directory.
- **`GET /agentmux/stream-local-file`** (route `crates/srv/src/server/routes.rs:207`; handler `crates/srv/src/server/files.rs:204-305`) streams the file asynchronously. The cap is `STREAM_LOCAL_FILE_MAX_BYTES = 500_000_000` (`files.rs:38`), and anything larger gets a 413. A **single `Range` gets a 206** (or a 416), via `parse_byte_range` (`:322-357`). CORS exposes `Content-Range`. The route needs `X-AuthKey`, so an element can't point at it directly (`SPEC_MEDIA_PANE_2026_07_26.md`, Post-implementation corrections).
- **MIME** is a hand-written extension table (`mime_from_path`, `files.rs:171-190`): html, css, js, json, png, jpg, svg, gif, webp, webm, mp4, mov, wav and woff. **Anything else, including pdf, avif and mp3, is sent as `application/octet-stream`.**
- A **content classifier already exists** but only the attachment pipeline uses it:
  - `crates/srv/src/backend/attachments/kind.rs:222-300` (`classify`/`classify_bytes`) reads the first 8 KB and recognises images by magic bytes, HEIC/AVIF via `ftyp`, SVG, `%PDF-`, ZIP/OOXML/ODF, OLE2, RTF, the archive family, audio, video and MP4.
  - Its text test is "valid UTF-8 and no NUL" (`kind.rs:176-185`).
  - The image sniff is in `attachments/process.rs:42-76`.
  - srv already depends on `image` (with TIFF), `lopdf`, `zip`, `encoding_rs`, `chardetng` and `rusqlite` (`crates/srv/Cargo.toml:39,82-103`).

### 2.4 CEF and PDF
- The GN args (`scripts/cef-build/args.gn`, `args-windows.gn`, `args-darwin.gn`) **do not set `enable_pdf`**, so Chromium's default (on, for desktop) applies. All three set `proprietary_codecs=true` and `ffmpeg_branding="Chrome"` (`args.gn:78-79`, `args-windows.gn:94-95`, `args-darwin.gn:25-26`), so H.264/AAC MP4 should play. **Not verified at runtime here.**
- `docs/specs/cef-size-reduction.md:196,242,310` *proposes* `enable_pdf=false` to save 5–10 MB. Its status is "proposed".
- **Browser panes render PDFs** with Chromium's viewer since #4938 (2026-10-14). `PDF_VIEWER_ORIGIN = "chrome-extension://mhjfbmdgcfjbbpaeojofohoefgiehjai/"` is allowlisted for navigation (`crates/cef/src/commands/platform.rs:702-708`, checked at `:740`). `file:` is not an allowed scheme (`:687-700`).
- Every browser in the app is **Alloy style**: the main window (`crates/cef/src/ui_tasks/window.rs:1731-1758`), browser panes (`crates/cef/src/browser_pane/creation.rs:225`) and floating panes (`crates/cef/src/floating_pane.rs:696`, `:930`). The app frontend is a page at `http://127.0.0.1:<port>`. srv is a different origin and is reached through CORS (`crates/cef/src/ipc.rs:66-100`).
- No custom URL schemes are registered. **The main app has no CSP.** `index.html` has no CSP meta tag; the only CSP in the app is `WIDGET_CSP`, for sandboxed widgets (`crates/srv/src/backend/widget_packages.rs:1113-1115`).
- **Not verified:** whether an `<iframe>` showing a `blob:` PDF inside the *main app page* would load Chromium's viewer. #4938 covered navigation in browser panes only.

### 2.5 Every way a file reaches a pane today

| Entry | Decides by | Editor | Media | OS app |
|---|---|---|---|---|
| Files pane double-click/Enter (`files-open.ts:38-44`, `files-view.tsx:373-396`) | extension | everything else (incl. `.avif`, `.csv`, `.log`) | png jpg jpeg gif webp svg webm mp4 mov wav | pdf, Office, archives, dll/so/db/sqlite, bmp/tif/heic, other audio/video, fonts; programs ask first |
| Agent reply image/AV card, tool-result image | extension (`local-media.ts:56-61`) | — | yes (`openInMediaPane`) | — |
| Editor file tree | none | everything (binary refused after reading) | — | — |
| Drop on Editor / Media | name, then MIME | text only | one media file | — |
| `OpenEditor` (MCP) | none | yes, reuses the Editor pane (`reuse_editor_pane: true`, `panes.rs:69`; `pane.rs:396`) | — | — |
| `OpenMedia` (MCP) | none | — | yes, always a new pane | — |
| `muxsh view` (`crates/srv/src/backend/shellintegration/muxsh.mjs:82-87`, `:152-158`) | its own list, including **`.pdf .bmp .ico .avi .mp3 .ogg .flac`** | everything else | yes, **but the pane can't show those seven** | — |
| Layout file import (`layout_file.rs:937`) | view in file | `editor` | `media` | — |

### 2.6 What happens today, per file type

The binary formats in this table have a NUL in their first bytes, by their standard headers. That is inferred from the headers, not tested on files.

| File | Editor | Media pane (via `OpenMedia`) | Files pane |
|---|---|---|---|
| `.pdf` | Refused as binary if a NUL is in the first 4 KB (usual, since a compressed stream or binary comment follows the header); otherwise **garbled editable text**. >10 MB: "File too large" | Downloads the whole file, then a **blank pane** (kind `none` has no branch, `media-view.tsx:372-378`, `:488`) | OS app |
| `.png .jpg .gif .webp` | Refused (binary) | `<img>` with zoom/pan | Media |
| `.svg` | Editable source, html highlighting, **no render** | Renders via `<img>` (safe) | Media |
| `.avif` | Refused | Blank (Chromium decodes AVIF; the pane's list omits it) | **Editor** (missing from the OS list) → refused |
| `.heic .tiff .bmp .ico` | Refused | Blank | OS app |
| `.mp4 .mov .webm` | Refused | `<video>`; the whole file (≤ 500 MB) is downloaded first, no streaming | Media |
| `.wav` | Refused | `<audio>` | Media |
| `.mp3 .ogg .flac .m4a` | Refused | Blank (Chromium plays them; the list omits them) | OS app |
| `.zip`, `.exe`, `.so`, `.db` (SQLite) | Refused | Blank | OS app (`.exe` asks) |
| 200 MB `.log` | "File too large (>10MB)" | Downloads 200 MB, then blank | Editor → too large |
| Invalid UTF-8 | Opens; `chardetng` guesses (often windows-1252); save blocked only if replacements were needed; no encoding shown | Blank | Editor |

### 2.7 Inconsistencies found along the way
1. `OpenMedia` and `muxsh view` send files the Media pane can't show, and the pane then goes blank silently.
2. The Editor downloads up to 10 MB before discovering that a file is binary.
3. Markdown preview shows `[img:…]` instead of images.
4. The media picker's filter leaves out SVG, which the pane supports.
5. Four extension lists that each say what "media" means have drifted apart: `local-media.ts`, `files-open.ts`, `muxsh.mjs` and the CEF picker. srv's `mime_from_path` is a fifth.

---

## 3. Goals and non-goals

### Goals
1. **The Editor opens any local file.** There is never a dead end: the worst case is an info card offering hex, text-anyway, the system app and reveal.
2. **The viewer is chosen by content, not just by name.** The user can override it per tab, and the choice persists with the tab.
3. **Type-specific controls appear only when relevant:** zoom for images, pages for PDFs, playback for media, the inspector for hex.
4. **The Media pane is retired with no regressions.** Every row of §2.2 has a home (§4.12). Saved layouts and blocks keep working.
5. **The Media pane's image and playback code is moved, not rewritten** (§4.13).
6. **Agents** get one tool (`OpenEditor`) that opens anything. `OpenMedia` keeps working as an alias.
7. **Untrusted files are safe.** PDF JavaScript never runs, SVG scripts never run, and nothing is inlined into the app DOM.
8. **Large files don't freeze the app.** Size tiers, a read-only range viewer for huge text, and streamed media.

### Non-goals
- Editing images or PDFs (annotation, form filling, crop). These are view-only. Hex editing is a later, opt-in phase (§4.4.5).
- Office document rendering (docx/xlsx/pptx). The info card offers the system app. The attachment pipeline's text extraction could back a "View text" later (§7).
- Remote/SSH files for the non-text viewers in phase 1. `pane.open` already refuses media on SSH (`pane.rs:314`). §4.6 lists the srv work needed to lift that.
- Changing the agent pane's inline media (poster, muted start, inline caps). Only its open-click target changes.
- Changing the Browser pane's PDF handling.

---

## 4. Design

### 4.1 The model: a document is a file plus a viewer
- An Editor tab today is a buffer (`EditorBuffer`: path, language, load state, scratch identity; `SPEC_DOCUMENT_TABS_2026_10_02.md` §6.1). It gains:
  - **`viewer: ViewerId`**, chosen at open (§4.2) and saved in the tab's `doctabs` record.
  - **`viewerOverride?: ViewerId`**, set by "Reopen with…".
  - **`probe`**: what srv reported (§4.2). Not saved; it is re-probed on restore.
- **Only the text viewer has a CodeMirror buffer, a dirty state, an encoding or LSP.** The other viewers are read-only, so the dirty-close confirmation, save and the encoding hooks skip them.
- The doc-tab host's move hooks (`editor-doc-tabs.ts`) carry `viewer` and the viewer's own state: zoom, page, playback time, hex offset.
- **Keep alive.** The Editor pane stays keep-alive (`block-registry.ts:39`). Within it, only the front tab's viewer is mounted. This is what Media does (M16), and it means a video in a background tab stops.
  - The Editor already caches CodeMirror state per tab. Non-text viewers keep a small serialisable state object per tab instead (zoom, page, time, offset) and re-fetch bytes when shown again, as Media does.

### 4.2 Choosing the viewer

**srv gets one new RPC, `fileprobe {path, connection?}`.** It stats the file, reads the first 8 KB and returns:
```jsonc
{
  "size": 1234567, "mtime": 1760000000,
  "kind": "image" | "image_file" | "svg" | "pdf" | "audio" | "video" | "archive" | "word" | "excel" | "powerpoint" | "sqlite" | "font" | "text" | "other",
  "mime": "image/png",            // from content first, extension second
  "text": { "encoding": "utf-16le", "bom": "utf-16le" } | null,   // null = binary
  "binary_reason": "nul" | "control_bytes" | null
}
```

**How it decides:**
1. **Magic bytes first.** It runs `attachments::kind::classify_bytes` (`kind.rs:230`). That classifier already gets "a `.txt` that is really a ZIP" right. It needs three small additions:
   - SQLite (`SQLite format 3\0`)
   - fonts (`wOFF`, `wOF2`, `OTTO`, `\0\1\0\0`)
   - a JPEG XL signature, for the info card
2. **Text vs binary**, in this order:
   1. A BOM means text in that encoding.
   2. **The UTF-16 parity test** (what VS Code does): in the first 512 bytes, NULs only at odd offsets means UTF-16LE, and only at even offsets means UTF-16BE.
   3. Any NUL means binary (git's test, over 8 KB here rather than git's 8000 bytes).
   4. Otherwise, more than a small share of WHATWG "binary data bytes" (0x00–0x08, 0x0B, 0x0E–0x1A, 0x1C–0x1F) means binary.
   5. Otherwise it is text, and the existing `text_encoding.rs` pipeline decodes it on read.

   The attachment classifier's stricter "must be valid UTF-8" test is wrong for the Editor: a CP1252 `.ini` file is text. So the Editor gets this function, and the attachment pipeline keeps its own.
3. **The extension** settles only ambiguous content: for example `ftyp` audio vs video, an OLE2 file's Office type, or text bytes that are `.csv` vs `.ts`.

**No new crate is needed.** `infer` (MIT) or `file-format` are options if the magic table grows (§5.3).

**From probe to viewer** (the frontend registry, §4.3, ranks the candidates):

| Probe | Default viewer | Also offered ("Reopen with…") |
|---|---|---|
| text, ≤ 10 MB (the current cap; §4.4.6 tiers) | **Text** (CodeMirror); markdown keeps preview/source/split | Hex |
| text, > 10 MB | **Large text** (read-only, range-backed) | Hex; "Load into editor anyway" up to a hard cap |
| `svg` | **Image** (rendered, via `<img>`) | Text (source), Hex |
| `image` (png jpeg gif webp avif bmp ico apng) | **Image** | Hex |
| `image_file` (heic heif tiff jxl) | **Image** if decodable (§4.4.2: srv converts TIFF; HEIC/JXL depend on build), else **Info** | Hex |
| `video`, `audio` | **Media** (audio/video) | Hex |
| `pdf` | **PDF** | Hex, Text (raw) |
| everything else binary | **Hex** if ≤ the hex cap, else **Info** | Info, Text ("Open as text anyway") |

**The user's choice wins.** "Reopen with…" (tab menu, command palette, `Ctrl+Shift+P` → "Reopen Editor With…") lists every viewer whose `canOpen` is at least `option`. It sets `viewerOverride` on that tab only.
- A setting, `editor:viewer.associations` (glob → viewer id, like VS Code's `workbench.editorAssociations`), makes a choice stick for a file pattern. This is phase 3.

**When to probe:** every open (tree, tab restore, drop, `OpenEditor`) probes before reading. The probe is one stat and an 8 KB read, so it costs about the same as today's `readeditorfile` prologue.
- The text path then calls `readeditorfile` as today.
- Every other viewer reads through `stream-local-file`, with Range where it helps.
- `sniffUnopenable` stays as a second check on decoded text: the 100K-character first line still routes to Large text instead of freezing CodeMirror.

### 4.3 The viewer registry (frontend)

```ts
// frontend/app/view/editor/viewers/registry.ts
interface ViewerDef {
    id: "text" | "markdown" | "image" | "media" | "pdf" | "hex" | "largetext" | "info" | string;
    label: string;                 // "Image Preview", "Hex", …
    icon: string;
    /** "default" opens automatically; "option" only via Reopen with…; null never. */
    canOpen(probe: FileProbe, path: string): "default" | "option" | null;
    priority?: number;             // tie-break among "default"s
    component: (p: ViewerProps) => JSX.Element;      // mounted for the front tab only
    /** Contextual controls, shown in the Editor's toolbar row while this tab is in front. */
    toolbar?: (ctx: ViewerCtx) => JSX.Element;
    /** Items for the Editor's footer/status chip area (dims, zoom %, page x/y, offset, encoding). */
    status?: (ctx: ViewerCtx) => JSX.Element;
    /** Keys the viewer handles when its body has focus (never global). */
    keymap?: ViewerKey[];
    /** Hooks the Editor's find bar (SPEC_EDITOR_AND_APP_FIND_2026_06_17.md) into the viewer. */
    find?: FindProvider;
    /** What a change on disk does (§4.9). */
    reload: "buffer" | "in-place" | "follow-folder";
    readOnly: boolean;
    /** Serializable per-tab state carried across tab moves and restarts. */
    state?: { save(): unknown; restore(s: unknown): void };
}
```

- It follows the pane-tab registry's pattern (`frontend/app/block/pane-tab-registry.ts`): built-ins register at module load, and a viewer could later come from a widget package. Widget-provided viewers are an open question (§7) and are not in this spec's phases.
- **Where the contextual controls go.** Editor chrome today:
  - the doc-tab strip, with the tree toggle in front
  - a toolbar row: the markdown Preview/Source/Split switch (`editor-view.tsx:1020-1034`)
  - the body
  - footer chips: LSP, and encoding later

  Viewers contribute to the **toolbar row** and the **footer chips**. That makes the markdown mode switch just the markdown viewer's toolbar, so it is a generalisation, not a new surface.

### 4.4 The viewers

#### 4.4.1 Text and markdown (unchanged)
- CodeMirror, encodings, LSP, live reload and markdown preview all stay exactly as they are.
- **One fix comes with the revamp:** pass `media` options to the preview's `<Markdown>` so local images render, using `LocalImage` from `markdown-media.tsx` with the file's folder as `baseDir`. This replaces the `[img:src]` placeholder (§2.7.3).
- An SVG opened with "Reopen with → Text" is today's behaviour.

#### 4.4.2 Image (moved from Media, §4.13)
- Everything in M7–M11 and M14, unchanged: zoom about the cursor, pan, double-click fit/actual, the `+ - 0 1` and arrow keys, the 32× ceiling, pixelated past 2×, the badge, keeping zoom across same-size renders, blob URL loading and the fade-in.
- **Added**, all from `REPORT_MEDIA_VIEWER_ZOOM_AND_FEATURES_2026_10_09.md` §4 (its "low effort" rows):
  - **Toolbar:** Fit, 100%, zoom % readout, a background toggle (dark / light / checkerboard; checkerboard is the default for images with alpha), Copy image, Reveal in folder, and "Follow newer files in this folder" (§4.9).
  - **Status:** `W × H`, file size, format.
  - **Previous/next image in the folder** (`[` / `]`). Arrows stay pan, per M8.
- **SVG** stays an `<img>` from a blob forced to `image/svg+xml` (as `files-preview.tsx:62` does). That is secure animated mode: no scripts and no external loads (§4.7). It is never inlined into the DOM.
- **Formats:**
  - Chromium decodes PNG, JPEG, GIF (animated), WebP (animated), APNG, AVIF (incl. animated), BMP and ICO. Add `avif bmp ico apng` to the image list.
  - TIFF: srv converts it to PNG on demand with the `image` crate it already ships with TIFF enabled (`Cargo.toml:89`), behind the same 500 MB cap, cached by `(path, mtime)`.
  - HEIC/HEIF needs libheif (LGPL, plus HEVC patents) or a young pure-Rust decoder. **Phase 5, and an open question** (§7).
  - JPEG XL is on by default from Chromium 155 (behind a flag since 145). It renders natively once the CEF milestone is at or past 155; until then the info card says so.
- **Very large images.** Decoded memory is about W×H×4, and GPU textures tend to top out around 16384 px. Above 16384 px on a side, or 100 MP, decode a downscaled bitmap with `createImageBitmap(blob, {resizeWidth})` for fit and low zooms. Show a "Showing a reduced preview" chip. At high zoom, show a 1:1 crop of the visible region. Tiling (OpenSeadragon-style) only if someone needs gigapixel images.
- EXIF orientation is applied by default (`image-orientation: from-image`). Chromium colour-manages ICC-tagged images.

#### 4.4.3 Audio and video (moved from Media, §4.13)
- Everything in M12, M13 and M16: native `<video controls>` and `<audio controls>`, fade-in on `loadeddata`, the `describeMediaError` messages, and a background tab stops playing.
- **Changed: streaming instead of a full download.**
  - Today the whole file, up to 500 MB, is fetched into a Blob before playing (M14). For files over about 32 MB, the viewer instead plays through **plain HTTP Range streaming** (no MediaSource), using an auth-free, short-lived, single-file URL (below), so `<video>` issues its own Range requests and srv answers 206 (`files.rs:254-298`).
  - Small files keep the Blob path, which is also what makes in-place re-render swaps flash-free.
- **The auth-free URL.** `<video src>` can't send `X-AuthKey` (M14's reason), and the `?authkey=` query fallback is deliberately limited to `/ws` (`SPEC_MEDIA_PANE_2026_07_26.md` correction 1). So:
  - srv mints a **per-file capability token**: `POST /agentmux/stream-token {path}` (authed) returns an opaque random token, valid for 10 minutes, bound to that one canonical path and to GET/HEAD.
  - `GET /agentmux/stream-local-file?t=<token>` serves it with Range.
  - This lets no one read any file they couldn't already read with the auth key, keeps keys out of URLs, and works for `<video>`, `<audio>` and pdf.js alike. **Open question for the security owner (§7)**; until it is decided, the Blob path remains, as today.
- **Kept from inline AV:** no autoplay when a tab opens. A tab opened from a video card (§4.13) opens paused at 0.
- **Added** (report §4 row 7, medium effort, phase 5): a loop toggle for short clips, `<`/`>` speed, and `,`/`.` frame step while paused.
- **Formats:** add `mp3 ogg oga opus flac m4a aac` (audio) and `m4v` (video) to the media lists. Chromium plays them, and H.264/AAC is enabled in the GN args (§2.4). MKV stays out (`local-media.ts:20-26`).

#### 4.4.4 PDF (new; engine choice in §5.1)
- **Engine:** `pdfjs-dist` (Apache-2.0), **pinned ≥ 6.2.108**. 6.4.299 (2026-10-03) is current. Use its `PDFViewer` component (`pdfjs-dist/web/pdf_viewer.mjs`): virtualized pages, text layer (selectable, copyable) and annotation layer (links only).
- **Options:**
  - `getDocument({ data | range transport, enableXfa: false, useWasm: true, wasmUrl, cMapUrl, standardFontDataUrl, iccUrl })`, with all side files **bundled** under `frontend/public/pdfjs/` and loaded from the app's own origin.
  - Viewer `enableScripting: false`, set explicitly.
  - The worker comes from `GlobalWorkerOptions.workerPort = new Worker(new URL("pdfjs-dist/build/pdf.worker.min.mjs", import.meta.url), { type: "module" })`. This way a worker that fails to load is an error, not a silent main-thread fallback.
- **Loading:**
  - Files up to 32 MB load as bytes through `fetchMediaBlob`.
  - Larger files use `PDFDataRangeTransport` over `fetchMediaRange` (64 KB chunks, pdf.js's default `rangeChunkSize`), so a 300 MB scan opens at page 1 without downloading everything. Once §4.4.3's token exists, pdf.js can be handed `url` directly.
- **Toolbar:**
  - page `n / N` input and prev/next (`PgUp`/`PgDn`, `Home`/`End`)
  - zoom: Fit width, Fit page, presets, `Ctrl+=`/`Ctrl+-` inside the viewer only
  - rotate, sidebar thumbnails (phase 2b) and outline (bookmarks)
  - Open in browser pane (the Chromium viewer, for print and forms)
  - Open with system app, Reveal
- **Find:** the Editor's find bar (`Ctrl+F`) drives `PDFFindController` through the `eventBus` (`find`, `updatefindmatchescount`).
- **Theme:** the page stays white, which is the content. The surround and toolbar use theme variables. An optional "invert pages in dark themes" toggle (CSS filter), off by default.
- **Links:** internal links navigate inside the viewer. External links open in a Browser pane, never inside the Editor, and never `javascript:`.
- **Errors:** encrypted PDFs prompt for a password. Malformed PDFs show the pdf.js error and offer Hex or Open with system app.
- **Agents:** `OpenEditor {file, page}` (§4.11).

#### 4.4.5 Hex (new; build, §5.2)
- **Layout:** `offset | 16 bytes hex | decoded text`. The row width is configurable (8, 16 or 32). The decoded column is ASCII or Latin-1 with `.` for non-printable bytes.
- **Virtualized rows over Range reads:**
  - 64 KB aligned chunks from `fetchMediaRange`, held in an LRU of about 64 chunks.
  - Element heights are capped by the browser, so above about 16 M rows the scrollbar maps to a segment (virtual scroll space with a scaled thumb) instead of one huge spacer.
- **Navigation:** `Ctrl+G` go to offset (decimal or `0x`), PgUp/PgDn, Home/End, and a click or drag selection mirrored between the hex and text columns.
- **Data inspector** (side panel, toggled): the bytes at the cursor as u8/i8 through u64/i64, f16/f32/f64, ULEB128, UTF-8 code point, binary and octal, plus a little/big-endian toggle. This is Microsoft's Hex Editor inspector, minus the exotic types.
- **Find:** hex pattern (`DE AD ?? EF`) or text (UTF-8, or UTF-16LE). The search runs on srv over the file (a new `filesearchbytes {path, pattern, from}` RPC) so it can cover files bigger than memory.
- **Copy** as hex, as a C array, or as base64. Copy offset.
- **Read-only.** The hex cap is 4 GB (offsets are u64; the hex view never loads the whole file). Editing (overwrite only, never insert, saved through a temp file and an atomic rename) is a **later, opt-in** phase behind a deliberate "Enable editing" toggle. It is not in this spec's phases.

#### 4.4.6 Large text (new)

Tiers, from editor precedent (VS Code turns tokenization off above 20 MB or 300K lines; JetBrains' intellisense limit is 2.5 MB and its open limit is 20 MB):

| Size | Viewer | Notes |
|---|---|---|
| ≤ 10 MB | Text (CodeMirror), as today | `readeditorfile`'s cap is unchanged in phase 1–3 |
| 10–50 MB | Text, **read-only**, highlighting and LSP off, word-wrap off | phase 4 raises `readeditorfile`'s cap to 50 MB for read-only loads only; `writeeditorfile` stays 10 MB |
| > 50 MB, or any first line > 100K chars | **Large text** | read-only, range-backed |

- **Large text design:**
  - srv builds a **line index** (`fileindexlines {path}` → offsets of every N-th line, streamed and cached per `(path, mtime)`).
  - The viewer virtualizes lines and fetches a byte range around the viewport.
  - Lines over about 10K characters are soft-truncated with "…(+N chars)" and a click-to-expand.
  - `Ctrl+G` goes to a line. `Ctrl+F` searches on srv (a regex over the stream, the same RPC family as hex find).
  - **Tail mode** (follow appended lines) for logs, driven by the existing watcher.
- Decoding uses the encoding detected by the probe, chunk-aligned on character boundaries via `encoding_rs`'s streaming decoder.

#### 4.4.7 Info card (the fallback, never a dead end)
- Shows the icon and kind (from the attachment tiles, `SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md` §5), name, size, MIME, modified time and, for archives, the entry count.
- **Buttons:** Open as hex, Open as text anyway, Open with system app (programs ask first, as in `files-open.ts:36`), Reveal in folder, Copy path.
- This replaces today's "Couldn't open file / Close tab" for binary files. Real errors (missing file, permission denied) keep the error panel.

### 4.5 Keyboard shortcuts
- Viewer keys are active **only while that viewer's body has focus**. CodeMirror's keymap never sees them, and they never shadow the doc-tab keys (`Ctrl+T`, `Ctrl+W`, `Ctrl+Tab`, `Ctrl+Shift+T`, handled first by `handleDocTabKey`).
- **Image:** `+ = - _ 0 1`, arrows (M8, unchanged), `[ ]` previous/next file, `B` background.
- **Media:** Space play/pause, `←/→` seek 5 s, `M` mute, `F` fullscreen, `< > , .` (phase 5).
- **PDF:** PgUp/PgDn, Home/End, `Ctrl+= / Ctrl+-` zoom, `Ctrl+G` page.
- **Hex:** arrows move the cursor, `Ctrl+G` offset, `Tab` switches between the hex and text columns, `I` toggles the inspector.
- **All viewers:** `Ctrl+F` find (where supported), `Ctrl+Shift+P` → "Reopen Editor With…", `Ctrl+Alt+R` reveal.
- **Ctrl+wheel stays pane zoom everywhere** (M8's rule, kept). Plain wheel zooms images only (the owner's earlier decision, `REPORT_MEDIA_VIEWER_ZOOM_AND_FEATURES_2026_10_09.md` §1). PDF and hex scroll on the wheel, as document tools do.
- Each key gets a row in `frontend/app/keybindings/tips.ts`. The `media:image` tip (M21) moves to `editor:image`.

### 4.6 How srv serves bytes

| Need | Route | Change |
|---|---|---|
| Probe | new RPC `fileprobe` | §4.2; reuses `kind.rs` |
| Text | `readeditorfile` | unchanged (phase 4: a read-only 50 MB tier) |
| Image, AV, PDF, hex, large text | `GET /agentmux/stream-local-file` | **extend `mime_from_path`** (`files.rs:171-190`) with avif, bmp, ico, apng, pdf, mp3, ogg, oga, opus, flac, m4a, aac, m4v, wasm and mjs. Better: take the MIME from the probe's sniff, so a mislabelled file still gets the right type. Range already works |
| Streaming without a header | new `POST /agentmux/stream-token` + `?t=` | §4.4.3; open question |
| TIFF → PNG | new `GET /agentmux/image-convert?path=` (authed) | `image` crate; cached by `(path, mtime)`; same 500 MB input cap |
| Line index, byte search | new RPCs `fileindexlines`, `filesearchbytes` | phases 3–4; run off the async workers (`spawn_blocking`) |

- **Caps:** `stream-local-file` keeps its 500 MB cap for whole-file reads. **Range reads of any size are allowed**, because each range is bounded. Phase 3 lifts the cap for Range requests only (the hex viewer and large-text viewer need it). The cap was meant to stop one request pulling a huge file into memory, and the handler already streams (`ReaderStream`).
- **SSH:** `fs_ops/remote.rs` already reads files over SSH for the text editor. A remote `fileprobe` and a remote range read are the srv work that lets the non-text viewers open SSH files. That is phase 5. Until then, the info card for a remote binary file offers "Copy to this computer and open".

### 4.7 Security
- **The threat model:** a file the user opens may be hostile. An agent may have downloaded it, or a prompt-injected agent may ask the user to open it. Nothing in a file may run script in the app's origin. The app origin holds the srv auth key and every MuxAPI capability.
- **PDF:**
  - pdf.js ≥ 6.2.108. **CVE-2026-16633** (published 2026-07-28 on the pdf.js advisories page) is arbitrary JS execution in ≥ 5.6.83 when scripting is on; it is fixed only in 6.2.108. **CVE-2024-4367** (≤ 4.1.392) is the earlier one.
  - `enableScripting: false`, `enableXfa: false`.
  - The version pin is enforced by a unit test that reads `pdfjs-dist/package.json`, so a downgrade fails CI.
  - Links are handled by our link service: internal links stay, external links open a Browser pane, and other schemes are dropped.
- **Defence in depth: the PDF viewer runs in a sandboxed iframe.**
  - It is served from the frontend's own static files: `frontend/public/pdfjs/viewer.html`, with `sandbox="allow-scripts"` and no `allow-same-origin`, so it gets an opaque origin.
  - It has its own CSP: `default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob:; img-src blob: data:; style-src 'self' 'unsafe-inline'; connect-src 'none'`. pdf.js needs `'wasm-unsafe-eval'` for its JPX, JBIG2 and ICC decoders.
  - The parent fetches the bytes (it holds the auth key) and posts an `ArrayBuffer` in. For Range loading, it answers `{start, end}` range requests from the iframe over `postMessage`.
  - So even a future pdf.js escape lands in an origin with no key, no network and no MuxAPI.
  - The same frame isolation could later host other parsers of untrusted formats.
- **SVG:** only as `<img>` from a blob (secure animated mode, no scripts, no external loads). The current code already follows this (`local-media.ts:14-17`). It is never inserted as markup.
- **Images, AV:** native decoders inside Chromium's renderer sandbox. Media errors are reported, never retried in a loop.
- **Converters on srv** (TIFF, later HEIC): run in the `__extract` child process the attachment pipeline already uses for untrusted parsers. That gives a 30 s timeout and keeps panics and memory spikes out of srv (`SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md` §6).
- **Hex / large text:** bytes are only ever text nodes, never HTML.
- **Paths:** the same posture as `readeditorfile`. Any absolute path the frontend asks for is served, gated by the OS. UNC/network paths are refused for anything loaded without a click, as inline media already does (`isNetworkPath`, `local-media.ts:89-95`).
- **The app-wide CSP gap** (`index.html` has none) is out of scope here. It is noted as a follow-up: the sandboxed-iframe design above does not depend on it.

### 4.8 Performance
- **Probe before read.** No viewer reads more than it shows: images and small media read the whole file; PDF, hex, large text and big media read ranges.
- **Lazy-load the heavy viewers.** pdf.js (about 1 MB of JS plus about 2 MB of side files) and the hex viewer are dynamic `import()`s, so the Editor's startup cost doesn't change for text users.
- **One mounted viewer per pane.** Blob URLs are revoked on tab switch and unmount (M14).
- **Images:** decode off-screen (`img.decode()`) before swapping a re-render, to remove the flash (report §5). Use `createImageBitmap` downscaling above the size limits.
- **PDF:** pdf.js virtualizes pages. Cap rendered canvas area (`maxCanvasPixels`) so a huge page can't allocate GBs.
- **Memory budget:** `REPORT_MEMORY_FOOTPRINT_500MB_BUDGET_2026_10_12.md` applies. The pdf.js worker is created per open PDF tab and terminated when the tab unmounts.

### 4.9 Live reload, per viewer

| Viewer | On a change on disk | Mechanism |
|---|---|---|
| Text / markdown | as today: clean tabs reload, dirty tabs don't | `editor_file_watcher` |
| Image, Media | reload in place: zoom is kept for a same-size image (M11); video keeps its time when the duration is unchanged and it is not playing; a playing video shows an "Updated, reload" chip instead of jumping (`SPEC_MEDIA_PANE_2026_07_26.md` open question 5) | `editor_file_watcher` (the file) |
| Image, Media with **Follow folder** on | a newer matching file in the folder replaces the tab's file, and the title follows (M15) | `media_file_watcher` (`watchmediadir`), unchanged on srv |
| PDF | reload in place, keeping the page and zoom | `editor_file_watcher` |
| Hex, large text | the chunk cache is invalidated; large text in tail mode appends | `editor_file_watcher` |

- **Follow folder is the Media pane's behaviour,** and it is the point of watching an agent's render folder. It becomes a per-tab toggle in the image and media toolbars.
  - **Default: on** for tabs that came from a former Media entry point (an agent reply card, a tool-result image, `OpenMedia`, a migrated `media` block).
  - **Default: off** for files opened from the tree or by `OpenEditor`. A user browsing a folder doesn't expect the tab to jump.
  - This keeps M15 with no regression where people rely on it. Whether to make it on everywhere is §7.

### 4.10 Accessibility
- Every toolbar control is a real button with a label and the shortcut in its title. Zoom, page and offset changes are announced in a `role="status"` region (WCAG 4.1.3).
- **Image:** keyboard zoom and pan already exist (WCAG 2.1.1). Add a single-pointer alternative to drag-pan: click-to-centre while zoomed (WCAG 2.5.7, report §5).
- **PDF:** the text layer makes text selectable and readable by screen readers. Page thumbnails get `alt="Page n"`.
- **Hex:** the grid is `role="grid"` with row and column headers. The cursor cell is `aria-selected`, and the inspector values are plain text.
- **Media:** native controls, which are already accessible. The custom keys duplicate them and don't replace them.
- Nothing animates beyond the existing quick fade, so `prefers-reduced-motion` needs nothing new.

### 4.11 Agents and MuxAPI

**`OpenEditor`** (`crates/mcp/src/tool_schemas.rs:836-851`) gains optional parameters:
- **`viewer`**: `"auto"` (default) or `"text" | "image" | "media" | "pdf" | "hex"`. "auto" lets the probe decide; `"text"` is today's behaviour.
- **`page`** (PDF) and **`offset`** (hex), next to the existing line support.
- **`follow_folder`** (image/media).

The description changes to: *"Open any file (code, text, markdown, images, video, audio, PDF, binary) in the AgentMux editor next to this conversation; the editor picks the right viewer."*

**`OpenMedia`** stays registered as an **alias**:
- Same schema. The handler calls the `OpenEditor` path with `viewer: "auto"`, `follow_folder: true` and `reuse_editor_pane: true`.
- Its description says: *"Deprecated alias of OpenEditor (kept for compatibility)."*
- It is removed from the Operator Config MuxAPI list (M26) and from the "…or OpenMedia to show it to the user" hints (M25); those now say `OpenEditor`.
- It is **not removed for two releases** (§4.12.3), because agents' prompts and transcripts carry the name.
- Side effect: `OpenMedia` starts reusing the Editor pane on screen. That closes `SPEC_MEDIA_OPEN_SHOWS_THE_MEDIA_PANE_2026_10_12.md` decision D4, which had been left out of scope.

**Other agent surfaces:**
- `pane.open` with `view: "media"` (MuxAPI, `muxsh`, older clients) is mapped by `build_pane_meta` (`pane.rs:271-276`) to `view: "editor"`, with the file queued through `editor:pending_open_files` (`pane.rs:367`) and the follow-folder flag.
- **`muxsh view`** (`muxsh.mjs:152-158`) stops guessing between media and editor: every file goes to `editor`, URLs to `browser`, folders to `files`. `MEDIA_EXTENSIONS` (`:82-87`) is deleted. That also removes one of the drifting lists (§2.7.5). `muxsh edit` keeps forcing the text viewer (`viewer: "text"`).
- `UIScreenshot` and `CaptureWindow` results suggest `OpenEditor`.

### 4.12 Retiring the Media pane

#### 4.12.1 Where each Media feature goes

| §2.2 row | Destination |
|---|---|
| M1 pane type `media` | the Editor's manifest gains `aliases: ["media"]` (`pane-tab-registry.ts:148-149,196,217`); `mediaPaneTab` is unregistered |
| M2 widget-bar entry | removed from `widgets.json`. The Editor's entry description becomes "Open and view any file: code, text, images, video, PDF, binary". Any user whose `widgets` config pinned `defwidget@media` gets the Editor (a widgets.json alias, or a one-line config migration) |
| M3 doc tabs | already shared; the Editor's tabs |
| M4 `media:open` queue | the Editor reads a legacy `media:open` on migrated blocks once (§4.12.2); new opens use `editor:pending_open_files` |
| M5 pick-pane-on-screen + reveal | `openInEditorPaneOnScreen(path, opts)`: same order (focused, visible, any, including stack tabs), same `showBlockWithoutFocus`. It is `mediaPaneOnScreen` with `isMedia` replaced by `isEditor`, plus the agent path's existing `maybe_reuse_editor_pane` |
| M6 kind lists | replaced by the probe; `local-media.ts` keeps its lists only for inline media in agent replies, extended with the formats in §4.4.2–4.4.3 |
| M7–M11, M14 image | Image viewer, moved (§4.13) |
| M12, M13 video/audio | Media viewer, moved (§4.13) |
| M15 follow folder | Image/Media viewers' "Follow folder" toggle; `watchmediadir` and `media_file_watcher` stay on srv unchanged |
| M16 front tab only | the Editor mounts only the front viewer (§4.1) |
| M17 picker | the Editor's "Open File…" uses the native dialog **without a type filter** (all files). The media filter (`platform.rs:1037-1062`) is removed. The empty Editor keeps its scratch buffer default (`SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md`) |
| M18 drop | `editor-drop.ts` accepts **any** file: the binary-name refusal (`:13`, `:27-36`) goes; the probe picks the viewer. A pathless drop of media bytes is shown from its Blob, as M18 does; a pathless binary over 10 MB opens in Hex from the Blob |
| M19 empty state | the Editor's own empty state (scratch). The faint-logo empty state is deleted |
| M20 RunCommand | already supported by the Editor |
| M21 tip | becomes `editor:image` (§4.5) |
| M22 layout files | import: `"media"` with `path` builds an Editor with that file and follow-folder on (`layout_file.rs:937-945`). Export: never writes `media` again |
| M23 `pane.open` media | mapped to the Editor (§4.11) |
| M24–M26 `OpenMedia`, hints, Operator Config | §4.11 |

The other paths into Media:
- `markdown-media.tsx:67-72` (`openInMediaPane`) and `ResultImages.tsx:52` call `openInEditorPaneOnScreen(path, {followFolder: true})`. Otherwise they create `{view: "editor", file: path}`.
- `files-open.ts`: `openTargetOf` loses the `"media"` target. Images, video, audio **and** PDF go to `"editor"`. The `BINARY` set shrinks to the files no viewer handles well (Office, `.pages`, `.key`, …) and still opens them in the OS app. Archives, `db`/`sqlite`, `so`/`dll`, fonts and `heic` go to the Editor (hex or info card), which offers "Open with system app" there. The Files pane's own "Open with default app" menu item stays.
- `DOC_TAB_HOSTS` (`keybindings/registry.ts:24`) becomes `["editor"]`. Doc-tab drag between Media panes no longer has a target type and is dropped.

#### 4.12.2 Migrating saved blocks

A saved block with `meta.view: "media"` resolves to the Editor through the alias. The Editor's `create(ctx)` then runs a **one-time meta migration** when it sees media keys:
1. `doctabs` entries whose payload is a plain path string (Media's `serialize: (d) => d.path`, `media-pane.tsx:76`) become Editor buffers: `{kind: "file", path, viewer: <probe at restore>, followFolder: true}`. Empty Media tabs are dropped.
2. `media:path` (pre-tabs Media blocks, M3), if it isn't already among the tabs, becomes a tab.
3. `media:open` requests are drained into `editor:pending_open_files`.
4. It writes `view: "editor"`, the new `doctabs`, and clears `media:path` and `media:open`. From then on the block is a plain Editor block.
   - The rewrite is deferred until the first successful mount, so a crash mid-migration leaves the old meta readable.
   - An older build opening the rewritten block sees an Editor. Viewers it doesn't know fall back to its own text path, which refuses binary as today.

**Tests:** a fixture of each historic Media meta shape (pre-tabs `media:path` only; doc tabs with files and an empty tab; a pending `media:open`) restores as an Editor with the right tabs and follow-folder on.

#### 4.12.3 Deprecation timeline
- **Release N (phase 1 lands):**
  - Media is unregistered; the alias and migration are live; all entry points route to the Editor.
  - `OpenMedia` is an alias whose result text says "opened in the editor (OpenMedia is now an alias of OpenEditor)".
  - The release notes say so.
- **N+1:** the deletion list in §4.13 is removed. The alias and migration stay.
- **N+2 or later:** consider removing `OpenMedia` from the tool list. Keep the `media` view alias and the layout-file import indefinitely; they are a few lines, and old layouts live on disk.

### 4.13 Moving the image and playback code (reuse, not rewrite)

New home: `frontend/app/view/editor/viewers/`.

| From | To | Change |
|---|---|---|
| `view/media/media-zoom.ts` (+ test) | `viewers/image/image-zoom.ts` (+ test) | **none**: a pure module, moved with `git mv` so history follows |
| `media-view.tsx:87-183`, `:271-283`, `:426-468` (zoom state, pointer, wheel, keys, resize, `<img>`, badge) | `viewers/image/image-viewer.tsx` | split out of `MediaView`. Takes `{path, file?, state}` props; the zoom badge becomes the toolbar's zoom readout plus the same in-image badge |
| `media-view.tsx:469-487` (`<video>`, `<audio>`, fade, `describeMediaError`) | `viewers/media/av-viewer.tsx` | split out; adds the range-stream path (§4.4.3) behind a size check |
| `media-view.tsx:185-236` (watch folder), `:328-370` (fetch/blob/revision) | `viewers/shared/use-file-bytes.ts`, `viewers/shared/use-follow-folder.ts` | turned into two hooks the image and AV viewers share, behaviour unchanged (fetch token, revoke, revision counter, clear the stale error) |
| `media-view.scss:9-38` | `viewers/image/image-viewer.scss` | class names renamed `media-view-*` → `image-viewer-*`; the rules are unchanged (no `will-change`, pixelated, cursors) |
| `local-media.ts` | stays | still shared by inline media; `fetchMediaBlob`, `fetchMediaRange`, `describeMediaError` used by the viewers |
| `media-pane.tsx` (doc tabs, queue, empty state), `media.tsx` (manifest), `media-drop.ts`, `media-open.ts`, the faint-logo empty state, the picker filter, `defwidget@media` | **deleted in N+1** | their behaviour lives on in the Editor (§4.12.1) |
| `media-pane.test.tsx`, `media.test.ts`, `media.drop.test.tsx` | ported where the behaviour moved (zoom, reload, follow-folder, drop of pathless bytes, front-tab-only); the rest deleted with the code | — |

**No-regression checklist.** Each item is a test or a live check in phase 1:
- [ ] plain-wheel zoom about the cursor
- [ ] Ctrl+wheel still zooms the pane
- [ ] drag-pan clamped at the edges
- [ ] double-click fit ↔ actual (2× for small images)
- [ ] `+ = - _ 0 1` and arrows
- [ ] maximum zoom `max(4, 32·natural/fit)`
- [ ] pixelated rendering past 2×
- [ ] the "N% · Fit" badge
- [ ] zoom kept across a same-size re-render; reset on a size change
- [ ] correct with the pane itself zoomed (CSS zoom ratio)
- [ ] fade-in with no jerk on swap
- [ ] SVG renders and never runs script
- [ ] video and audio play with native controls
- [ ] codec errors show the `MediaError` code text
- [ ] a background tab's video stops
- [ ] follow-folder replaces the tab on a newer render; the title follows; an in-place overwrite reloads
- [ ] a pathless dropped image or video shows from its bytes
- [ ] clicking an image in an agent reply adds a tab to the Editor on screen and reveals a minimized Editor without moving focus
- [ ] a split of an Editor opens an empty Editor (`SPEC_EDITOR_MEDIA_SPLIT_OPENS_EMPTY_2026_10_10.md`, unchanged)
- [ ] `OpenMedia` still opens the file

### 4.14 Theming
- Viewers use the theme's CSS variables only. The theme-colour ESLint rules (`eslint.theme-colors.config.js`) apply to the new files.
- Image backgrounds use theme surface colours for dark and light. The checkerboard uses two theme-derived neutrals.
- The PDF surround uses `--block-bg-color`; the pages stay white, with an optional invert.
- Hex columns use the editor's monospace font and its CodeMirror theme colours: offsets muted, the decoded column secondary, and selection in the editor's selection colour.
- Status chips reuse the existing footer chip style (the LSP chip).

---

## 5. Recommendations

### 5.1 PDF engine: pdf.js in the Editor; Chromium's viewer stays in the Browser pane

| | **pdf.js (`pdfjs-dist` 6.4.x)** | Chromium PDF viewer (PDFium, in CEF) | EmbedPDF (PDFium → WASM) | MuPDF.js |
|---|---|---|---|---|
| Works in the Editor's layout | yes: a DOM/canvas viewer inside the pane, so splits, floating panes, the tab strip and themes all work | needs a full browser frame. In the main page that means an `<iframe>` with a `blob:` PDF (**unverified in this app**; OOPIF PDF frames have had embedding regressions), or a native browser surface as in Browser panes (z-order and overlay costs) | yes | yes |
| Controls we own (page nav, find bar, agent `page`) | yes: `PDFViewer`, `EventBus`, `PDFFindController` | no API: a closed UI in an extension frame; `postMessage` is blocked for OOPIF full-page PDFs | yes | yes |
| Dependency on the CEF build | none | `enable_pdf` (proposed off in `cef-size-reduction.md`); Alloy-style PDF bugs in CEF's tracker (#3793, #3644); `--disable-pdf-extension` ignored since 126 | none | none |
| Rendering fidelity | very good; weaker than PDFium on some complex or broken files | best (PDFium) | PDFium | excellent |
| Security | JS-engine bugs are possible (CVE-2024-4367, CVE-2026-16633); mitigated by version pin, scripting off and the sandboxed iframe (§4.7) | Chromium's renderer sandbox; patched with CEF upgrades | C engine in WASM: memory-safe sandbox | C in WASM |
| Size | ~1 MB JS + ~2 MB side files, lazy | 0 (already in libcef) | ~4–5 MB WASM | large |
| Licence | Apache-2.0 | (bundled) | MIT viewer, PDFium Apache-2.0; **check v3 terms** | **AGPL-3.0** or commercial: not usable here |
| Forms/print | view-only (forms off) | full | partial | full |

**Recommendation: pdf.js** for the in-Editor viewer, with **"Open in browser pane"** one click away for printing and form filling (Chromium's viewer, which already works there since #4938).
- **EmbedPDF is the fallback** if pdf.js fidelity disappoints on real-world files. Its PDFium-in-WASM rendering is better on broken files, at a higher size cost. Its licence must be re-checked when the choice is made.
- **Phase 2 starts with a half-day spike** that settles the one unknown: whether an `<iframe src=blob:…pdf>` inside the main app page gets Chromium's viewer at all. If it does, it is a reasonable **zero-dependency stopgap**, but it still loses theming, find, agent page control and the guarantee against `enable_pdf=false`. So it does not change the recommendation.

### 5.2 Hex viewer: build it
- **The libraries:**
  - `react-hex-editor` 0.3.0 had its last release about 6 years ago, is rated inactive, and pulls in styled-components.
  - The other React packages (`hex-editor-react`, `react-hexviewer-ts`) are abandoned.
  - The app is Solid, not React, so a React component would also need a bridge.
- **What we need is small:** a virtualized grid over Range-fetched chunks, a selection model, an inspector and a go-to.
- **Use Microsoft's Hex Editor (MIT) as the design reference:** 16 columns, decoded text, inspector types and endianness, Ctrl+G, read-only by default. Don't take its code; it is a VS Code webview with a VS Code document model.
- **Estimate:** about 600–900 lines of Solid plus about 150 of Rust (range search).

### 5.3 Binary detection: reuse what srv has, add two rules
- Reuse `attachments/kind.rs` (magic bytes) and `text_encoding.rs` (BOMs and decoding).
- Add the **UTF-16 NUL-parity rule** and the **WHATWG binary-byte rule** (§4.2) in a new `editor_probe.rs`.
- Keep the probe window at 8 KB: git uses 8000 bytes, VS Code 512, WHATWG 1445; 8 KB catches more late NULs at no measurable cost.
- **No new crate now.** If the magic table outgrows `kind.rs`, adopt `infer` (MIT, pure Rust) rather than libmagic. libmagic needs a C library; `tree_magic_mini`'s embedded database is GPL.
- **One source of truth.** The frontend never guesses by extension for routing again. `local-media.ts`'s lists remain only for agent-reply inline rendering, which can't afford a probe per image reference.

---

## 6. Phased plan

Each phase is its own PR with a changeset. Every phase ends with a live check on an isolated `task dev` build, never the installed instance.

### Phase 1: one pane (the smallest useful slice)
Scope:
1. **srv:** the `fileprobe` RPC (`editor_probe.rs`: `kind.rs`, UTF-16 parity, WHATWG binary bytes, SQLite and font magic) and the `mime_from_path` additions (§4.6).
2. **Viewer registry** (§4.3), with five viewers:
   - **Text** (wraps today's CodeMirror path untouched)
   - **Image** (moved)
   - **Media** (moved; Blob path only, as today)
   - **Info**
   - **"Open as text anyway"**, which forces Text and bypasses `sniffUnopenable`'s NUL refusal but never the 100K-character line guard
3. **Probe-before-read** in `_openFileWithMode`. The Editor stops downloading binaries.
4. **Move** the Media image and playback code (§4.13, the first half: moves and hooks). The Media pane still compiles but is unregistered.
5. **Retire the Media pane:**
   - `aliases: ["media"]` and the meta migration (§4.12.2)
   - every entry point routed (agent reply cards, `ResultImages`, the Files pane `openTargetOf`, drops, `OpenMedia` alias, `pane.open` `media`, `muxsh view`, layout import)
   - `widgets.json` entry removed; tips; Operator Config and tool-hint text
6. **Follow folder** toggle (default per §4.9).
7. Fix markdown preview images (§4.4.1).

Tests:
- **Rust:** `fileprobe` over a fixture per kind (png, svg, avif, heic, pdf, zip, docx, exe, so, sqlite, woff2, UTF-16LE/BE with and without BOM, CP1252 text, UTF-8 with an emoji cut at 8 KB, an empty file, a NUL at byte 8000); `mime_from_path` additions; `build_pane_meta("media")` maps to the Editor; layout import of `media`.
- **Frontend unit:**
  - registry ranking and override
  - `openTargetOf` table
  - `editorDropVerdict` accepts any file
  - migration fixtures (§4.12.2)
  - `openInEditorPaneOnScreen` selection order (the five cases of `SPEC_MEDIA_OPEN_SHOWS_THE_MEDIA_PANE_2026_10_12.md` §5, ported)
  - the zoom tests, moved unchanged
  - the follow-folder hook (revision bump, stale-error clear)
  - `block-registry.test.ts`: `media` resolves to `editor`, and `splitBlockDef` is declared by agent and editor only
- **MCP:** `OpenMedia` posts the `OpenEditor` request with `viewer: "auto"` and `follow_folder: true`.
- **Live:**
  - open png, svg, gif, webp, mp4, wav, a pdf (→ Info, with "Open in browser pane"), zip, exe and sqlite from the tree, from the Files pane, by drop and via `OpenEditor`/`OpenMedia`
  - restore a saved layout with a Media pane
  - click an image in an agent reply with a minimized Editor
  - run the §4.13 checklist

### Phase 2: PDF
- **Spike first** (§5.1: the iframe and blob question). Then:
  - the pdf.js viewer in a sandboxed iframe (§4.4.4, §4.7)
  - Range loading via `postMessage`
  - toolbar, find, outline
  - `OpenEditor {page}`
  - "Open in browser pane"
- Phase 2b: thumbnails sidebar.
- **Tests:** a version-pin test (≥ 6.2.108); `enableScripting` is false; a fixture PDF with a JS action and a `javascript:` link shows no execution and no navigation; an encrypted PDF prompts; a 200 MB PDF opens page 1 with fewer than N range requests; find counts matches; a page survives a tab move and a restart.
- **Live:** render a spec PDF, a scanned PDF and a broken PDF in light and dark themes.

### Phase 3: hex and binary
- The hex viewer (§4.4.5), `filesearchbytes`, Range reads above the 500 MB cap, "Reopen with…" for every viewer, and `editor:viewer.associations`.
- **Tests:**
  - chunk LRU
  - offset ↔ row maths at 4 GB
  - inspector decode tables (endianness, f16)
  - hex pattern search with wildcards across chunk boundaries
  - selection sync
  - read-only (no write RPC is reachable)

### Phase 4: large files
- `fileindexlines`, the Large text viewer, the 10–50 MB read-only tier, tail mode.
- Optionally the encodings spec's phase 3 chip, which shares the footer slot.
- **Tests:** line index on CRLF/LF/mixed text and UTF-16; viewport fetch; jump to line 5,000,000; search over 1 GB; tail-follow on append.

### Phase 5: cleanup and extras (by value)
1. Delete the moved-out Media code (release N+1, §4.13).
2. Video extras: loop, speed, frame step. Image extras from the report: previous/next, compare A/B, PNG metadata (the ComfyUI workflow).
3. TIFF conversion (`image-convert`); HEIC per §7.
4. **CSV/TSV table view** (papaparse 5.7 in a worker; an option, not the default).
5. **Archive listing** (srv `zip`/`tar` crates, already dependencies; list only, no extraction).
6. **SQLite browser** (srv `rusqlite`, already a dependency, opened read-only with `SQLITE_OPEN_READONLY`; table list plus paged rows).
7. **Fonts** (a `FontFace` specimen).
8. **Notebooks** (`.ipynb` rendered read-only).
9. 3D (`@google/model-viewer`, glTF). Lowest priority.
10. Non-text viewers over SSH (remote probe and range).

---

## 7. Open questions
1. **Stream token** (§4.4.3): is a short-lived, single-path, GET-only capability URL acceptable to the security owner? Without it, video over about 500 MB can't play, and big video and PDF loads keep going through a Blob or the `postMessage` range path.
2. **Follow-folder default** (§4.9): on only for tabs coming from former Media entry points (recommended), or on for every image and video tab?
3. **The Files pane:** should PDFs and archives open in the Editor by default (recommended), or keep going to the OS app with "Open in editor" in the menu?
4. **HEIC:** ship libheif (LGPL-3.0, dynamically linked, plus HEVC patent exposure), wait for a pure-Rust decoder, or stay with the info card plus "Open with system app"? Recommended: the info card for now.
5. **`OpenMedia` removal:** keep it as an alias indefinitely, or drop it at N+2?
6. **Widget-provided viewers:** should a sandboxed widget be able to register a viewer for a file type (the VS Code custom-editor model)? That is powerful, but it is a new permission surface. Recommended: later, with its own spec.
7. **Office files:** show the attachment pipeline's extracted text (read-only) as a viewer option for docx/xlsx/pptx, or keep only "Open with system app"?
8. **Plain-wheel zoom for images inside the Editor:** the owner chose plain-wheel zoom for the Media pane. Should the same hold in the Editor, where a user might expect the wheel to scroll a tall screenshot? Recommended: keep plain-wheel zoom (it matches the owner's decision and dedicated viewers), and add the report's "Wheel: Zoom / Scroll" setting if anyone asks.
9. **CEF milestone:** JPEG XL renders natively only at Chromium ≥ 155. Track it with the next CEF upgrade rather than converting on srv?

---

## 8. Sources

### Code (this repo, `main` @ `521daa2ad`)
- **Editor:** `frontend/app/view/editor/editor.tsx`, `editor-model.ts` (open path 535–626; sniff 1437–1457; languages 1466–1513; reload 217–224, 281–285, 737–788), `editor-view.tsx` (languages 50–88; error panel 1046–1062; preview 1020–1034, 1141–1158; tree menu 840–856), `editor-drop.ts`, `package.json:81-106`
- **Media:** `frontend/app/view/media/media.tsx`, `media-pane.tsx`, `media-view.tsx`, `media-zoom.ts`, `media-view.scss`, `media-drop.ts`, `media-open.ts`; `frontend/app/element/local-media.ts`; `frontend/app/element/markdown-media.tsx`; `frontend/app/view/agent/components/ResultImages.tsx`
- **Routing:** `frontend/app/view/files/files-open.ts`, `files-view.tsx:373-396`; `frontend/app/block/block-registry.ts`; `frontend/app/block/pane-tab-registry.ts:144-219`; `frontend/app/keybindings/registry.ts:24`, `tips.ts:322-330`
- **srv:** `crates/srv/src/server/editor_handlers.rs` (157–235, 466–615), `crates/srv/src/server/files.rs` (38, 171–190, 204–357), `crates/srv/src/server/routes.rs:51-88,205-207`, `crates/srv/src/server/app_api/pane.rs` (223–321, 367, 396, 1042), `crates/srv/src/backend/attachments/kind.rs`, `attachments/process.rs:34-76`, `crates/srv/src/backend/text_encoding.rs`, `editor_file_watcher.rs`, `media_file_watcher.rs`, `layout_file.rs:505-516,930-945`, `fs_ops/remote.rs:310-325`, `config/widgets.json:228-239`, `operator-config-seed.json`, `backend/shellintegration/muxsh.mjs:82-158`, `crates/srv/Cargo.toml:39,82-103`
- **MCP:** `crates/mcp/src/tool_schemas.rs:362,692,836-866`, `crates/mcp/src/tools/panes.rs:11-170`, `tools/ui.rs:32`, `window_capture.rs:636`
- **CEF:** `crates/cef/src/commands/platform.rs:687-740,1037-1062`, `crates/cef/src/app/mod.rs:572-1095`, `ui_tasks/window.rs:1731-1758`, `browser_pane/creation.rs:225`, `floating_pane.rs:696,930`, `ipc.rs:66-100`; `scripts/cef-build/args.gn`, `args-windows.gn`, `args-darwin.gn`; `crates/srv/src/backend/widget_packages.rs:1113-1115`; commit #4938
- **Docs:** the specs listed under "Builds on"; `docs/specs/cef-size-reduction.md`; `docs/reports/REPORT_CEF_PROPRIETARY_CODEC_GAP_2026_07_26.md`; `docs/MUXSH.md`; `docs/analysis/ANALYSIS_READ_TOOL_PREVIEW_2026_10_01.md`

### Web (retrieved 2026-10-15)
- **VS Code:**
  - Custom Editors API: https://code.visualstudio.com/api/extension-guides/custom-editors
  - media-preview formats: https://raw.githubusercontent.com/microsoft/vscode/main/extensions/media-preview/package.json
  - image preview zoom/pixelation: https://raw.githubusercontent.com/microsoft/vscode/main/extensions/media-preview/media/imagePreview.js
  - binary/encoding detection: https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/workbench/services/textfile/common/encoding.ts
  - "Open Anyway": https://github.com/microsoft/vscode/issues/225486
  - binary diff as custom editor: https://github.com/microsoft/vscode/issues/324804
  - large-file thresholds: https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/editor/common/model/textModel.ts ; https://code.visualstudio.com/updates/v1_15 ; https://github.com/microsoft/vscode/pull/240919
- **Hex Editor:** https://github.com/microsoft/vscode-hexeditor/blob/main/README.md ; https://raw.githubusercontent.com/microsoft/vscode-hexeditor/main/package.json ; https://github.com/microsoft/vscode-hexeditor/releases
- **VS Code PDF extensions:** https://marketplace.visualstudio.com/items?itemName=tomoki1207.pdf ; https://github.com/mmtftr/vscode-pdfviewer
- **Zed:** https://zed.dev/docs/configuring-zed ; https://github.com/zed-industries/zed/discussions/37768 ; https://github.com/zed-industries/zed/discussions/24488
- **JetBrains:** https://www.jetbrains.com/help/idea/tuning-the-ide.html ; https://youtrack.jetbrains.com/articles/SUPPORT-A-4391
- **Sublime Text:** https://forum.sublimetext.com/t/sublime-opens-file-with-hex-view-even-though-it-has-null-bytes/22042 ; https://forum.sublimetext.com/t/st4-png-files-are-not-displayed-graphically-but-in-the-hex-dump/73058
- **CEF runtime styles and PDF:** https://github.com/chromiumembedded/cef/issues/3685 ; https://magpcss.org/ceforum/viewtopic.php?f=6&t=19510 ; https://github.com/chromiumembedded/cef/issues/3793 ; https://github.com/chromiumembedded/cef/issues/3644 ; https://github.com/chromiumembedded/cef/issues/3047 ; https://github.com/cefsharp/CefSharp/issues/4880 ; https://chromeenterprise.google/policies/always-open-pdf-externally/ ; https://issues.chromium.org/issues/40286700 ; https://github.com/cefsharp/CefSharp/wiki/Embedding-PDF.JS-in-CEFSharp
- **pdf.js:**
  - releases: https://github.com/mozilla/pdf.js/releases
  - 6.0 breaking changes: https://github.com/mozilla/pdf.js/releases/tag/v6.0.227
  - API options: https://raw.githubusercontent.com/mozilla/pdf.js/master/src/display/api.js
  - advisories (CVE-2026-16633, CVE-2024-4367): https://github.com/mozilla/pdf.js/security/advisories
  - scripting sandbox: https://github.com/mozilla/pdf.js/issues/18242
  - Vite worker setup: https://github.com/mozilla/pdf.js/discussions/19090
  - viewer components: https://www.nutrient.io/blog/pdfjs-react-viewer-setup/
  - WASM/CSP: https://github.com/snapotter-hq/SnapOtter/issues/2105
- **Other PDF engines:** EmbedPDF: https://github.com/embedpdf/embed-pdf-viewer ; https://www.npmjs.com/package/@embedpdf/pdfium — MuPDF.js: https://github.com/ArtifexSoftware/mupdf.js/
- **Images:**
  - SVG as image: https://developer.mozilla.org/en-US/docs/Web/SVG/Guides/SVG_as_an_image ; https://svgwg.org/specs/integration/
  - rendering and orientation: https://developer.mozilla.org/en-US/docs/Web/CSS/image-rendering ; https://developer.mozilla.org/en-US/docs/Web/CSS/image-orientation
  - downscaling: https://developer.mozilla.org/en-US/docs/Web/API/Window/createImageBitmap
  - format support: https://caniuse.com/heif ; https://caniuse.com/apng
  - JPEG XL in Chrome: https://developer.chrome.com/blog/jpeg-xl-in-chrome ; https://developer.chrome.com/blog/chrome-155-beta
  - decode limits: https://issues.chromium.org/issues/40261318
  - colour management: https://issues.chromium.org/issues/40249532
  - `image` crate codecs: https://docs.rs/image/latest/image/codecs/index.html
  - HEIC in Rust: https://formulae.brew.sh/formula/libheif ; https://github.com/tbraun96/heic-rs
- **Binary detection:**
  - git: https://chromium.googlesource.com/external/msysgit/git/+/fb6aa9a54fa455d8e8b9e3971b1738afabab2679/xdiff-interface.c
  - WHATWG MIME Sniffing: https://mimesniff.spec.whatwg.org/
  - crates: https://docs.rs/content_inspector/latest/content_inspector/ ; https://github.com/bojand/infer ; https://docs.rs/file-format/latest/file_format/ ; https://docs.rs/tree_magic_mini/latest/tree_magic_mini/ ; https://docs.rs/mime_guess/latest/mime_guess/ ; https://github.com/robo9k/rust-magic
- **Hex libraries:** https://github.com/kmck/react-hex-editor ; https://security.snyk.io/package/npm/react-hex-editor
- **Large files:**
  - HTTP Range: https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Range_requests
  - CodeMirror 6: https://discuss.codemirror.net/t/replace-the-entire-doc-performance/5289 ; https://discuss.codemirror.net/t/how-can-make-sure-the-whole-document-is-always-rendered-in-codemirror6/5241
  - Monaco: https://github.com/Microsoft/monaco-editor/issues/1330
- **Codecs:** https://chromium.googlesource.com/chromium/src/media/+/refs/heads/main/media_options.gni ; https://github.com/chromiumembedded/cef/issues/3559
- **Extras:**
  - zip.js: https://github.com/gildas-lormeau/zip.js/releases
  - SQLite WASM: https://sqlite.org/wasm
  - papaparse: https://www.npmjs.com/package/papaparse
  - notebooks: https://github.com/walkframe/jupyter-kit
  - FontFace: https://developer.mozilla.org/en-US/docs/Web/API/FontFace/FontFace
  - model-viewer: https://www.npmjs.com/package/@google/model-viewer
- **WCAG:** https://www.w3.org/WAI/WCAG22/Understanding/keyboard.html ; https://www.w3.org/WAI/WCAG22/Understanding/dragging-movements.html ; https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html

### Not verified for this spec
- **CEF runtime behaviour, not tested:**
  - whether an `<iframe>` showing a `blob:` PDF in the main app page gets Chromium's viewer
  - whether `--disable-extensions` (`app/mod.rs:1013`) affects it
  - that H.264/AAC actually plays in the current build
- **Format behaviour inferred from headers, not tested:** the PDF row of §2.6 (refused vs garbled text depends on each file's first 4 KB) and each binary format's NUL position.
- **Fast-moving facts to re-check when implementing:**
  - pdf.js 6.x `TextLayer`/`AnnotationLayer` signatures and `enableScripting` defaults per build
  - the CEF milestone the app will be on when JPEG XL matters
  - EmbedPDF v3 licence terms

---

## Phase 1 as built (2026-10-15)

Built as §6 phase 1 describes, with these differences:

1. **The Media pane's code is deleted now, not in release N+1** (§4.12.3, §4.13). Once nothing registered or opened it, CI's unused-file check (knip) failed on it. Its behaviour lives on in the Editor; the `media` view alias, the saved-block migration (`frontend/app/view/editor/media-migration.ts`), the `OpenMedia` alias and the layout-file import stay, as planned.
2. **The native "open file" dialog has no filter at all** (M17). It was only ever the Media pane's; the one other caller (Settings → Widgets, picking a `widget.json`) was being shown a media-only filter.
3. **`fileprobe` is registered next to the Files pane's `fs.*` commands** (`register_fs_handlers`), with its handler in `crates/srv/src/backend/editor_probe.rs`, so the editor's handler file doesn't grow past its size ratchet.
4. **The probe returns `image` for every format Chromium decodes** (PNG, JPEG, GIF, WebP, AVIF, APNG, BMP, ICO) and `image_file` for TIFF, HEIC and JPEG XL, which get the info card until phase 5's converter.
5. **The viewer bar holds its own controls** (fit, 100%, background; the viewer switch; follow folder; reveal; open with the system's app; the file's facts). The markdown Preview/Source/Split switch stays where it was.
6. **A dropped file with no host path** opens from its bytes in the image or media viewer (else the info card) under a session-only tab path; after a restart its tab says to drop it again, as the Media pane's did.
7. **Not in phase 1:** the stream token (§4.4.3, open question 1), so audio and video still load whole into a blob as Media did; `editor:viewer.associations` (phase 3).

## Phase 2 as built (2026-10-16)

The PDF viewer (`frontend/app/view/editor/viewers/pdf/`), with pdf.js 6.4.299. Differences from §4.4.4 and §4.7:

1. **pdf.js runs in the Editor's own page, not a sandboxed iframe.** An opaque-origin iframe can't load the app's module scripts or start pdf.js's worker from the app's origin without CORS and blob-worker workarounds on both the Vite dev server and srv. The protection kept: pdf.js at or past 6.2.108 (a test fails the build below it), XFA off, `isEvalSupported: false` (the font-program eval behind CVE-2024-4367), no scripting manager (so a document's JavaScript never runs), and links intercepted: http(s) opens a Browser pane, every other scheme is dropped. Parsing happens in pdf.js's worker. The iframe stays a possible later hardening.
2. **Large files load in ranges directly** through `fetchMediaRange` (64 KB chunks, `disableAutoFetch`), with no `postMessage` hop, since there is no iframe.
3. **pdf.js's side files are bundled by Vite** (CMaps, standard fonts, the JBIG2 and JPEG 2000 decoders) and handed to pdf.js through its `BinaryDataFactory`, since Vite gives each a hashed name rather than a folder.
4. **Find** is the viewer's own box in its toolbar (Ctrl+F there), not the Editor's CodeMirror find bar.
5. **Not in phase 2:** the thumbnails sidebar and outline (2b), `OpenEditor {page}`, and "Open in browser pane" (a Browser pane doesn't load `file:` URLs); "Open with the system's app" covers printing and forms meanwhile.
