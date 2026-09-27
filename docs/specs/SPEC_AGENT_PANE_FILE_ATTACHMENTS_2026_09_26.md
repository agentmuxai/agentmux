# SPEC: Attach any file in the agent composer (PDF, Office, text, code, …)

**Date:** 2026-09-26
**Status:** proposed — nothing in this spec is implemented. Extends the shipped image attachments (`SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md`; #3910, #3917, #3918, #3920). Written against `main` @ `6b7b74ef1`.
**Author:** Korp@narko

---

## 0. The ask

> we also want to support other files too, like pdf, docx, etc

Follow-ups from the repo owner (2026-09-26):

- Dropped or pasted documents go **in the tray, like images** — stored outside the repo and sent with the message; the working folder is no longer touched.
- **Any file** can be attached. Documents agents can't read natively also get a text version.
- > get the most popular icon types. for complex files, just use an icon. For files that have easy previews, use a thumbnail

## 1. What happens today

- The tray, store and delivery are image-only. `attachments.ingest` classifies paths with `is_image_path` and returns everything else as `non_images` (`agentmux-srv/src/backend/attachments/mod.rs:692`, `:748`).
- The frontend keeps the old behavior for those: `useAgentDropAttach` copies non-images into the agent's `cmd:cwd` and types `@name` (`frontend/app/view/agent/hooks/useAgentDropAttach.ts`). Ctrl+V and right-click Paste skip non-images with a "not images" note (`attachment-draft.ts:206`, `AgentFooter.tsx:517`).
- HEIC, AVIF and SVG are refused as "not supported yet" (`process.rs:145`).

## 2. What agents can do with documents (research, 2026-09-26)

| Agent | PDF | DOCX / XLSX / PPTX | Text and code |
|---|---|---|---|
| **Claude (API)** | `document` block (base64): each page becomes text + a page image; ≤ 100 pages per request on < 1M-context models, 32 MB request cap | **Not supported** — "must be converted to text or PDF first" | `document` block with `text/plain`, or just read the file |
| **Claude Code** | Read tool reads PDFs; over 10 pages needs `pages` (≤ 20 per call), which shells out to poppler's `pdftoppm` and fails without it | Read refuses binary files | Read |
| **Codex** | No file input type (app-server `UserInput` is text/image/localImage/audio/skill/mention; `exec` has only `-i`); no native PDF reader | none | shell tools |
| **Gemini CLI** | `@path` reads PDFs natively (≤ 20 MB) | "Cannot display content of binary file" | yes |
| **Qwen Code** | native, or `pdftotext` fallback | no | yes |
| **ACP** (Copilot CLI) | `resource_link` always; embedded `resource` when `promptCapabilities.embeddedContext` | same | same |

Sources: https://platform.claude.com/docs/en/build-with-claude/pdf-support · https://code.claude.com/docs/en/tools-reference · https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/v2/turn.rs · https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/utils/fileUtils.ts · https://agentclientprotocol.com/protocol/content

**Conclusions:**

1. The **numbered path list** already sent for images is the only route that works for every agent, so every file goes there.
2. **Office files need a text version made by AgentMux** — no agent reads them. (claude.ai does the same: "for non-PDF files Claude extracts text only".)
3. **PDFs** go inline to Claude as `document` blocks within the existing inline budgets (so Claude sees them without a tool call, and without poppler), and by path to everyone else. Stream-json `document` blocks are undocumented, so this needs a smoke test against the pinned CLI before relying on it (§11).

## 3. Goals

1. Drop, Ctrl+V or right-click Paste **any file** into the composer; it gets a tile in the tray and is sent with the message. Same limits as images: 128 attachments and 1 GB per message.
2. **Tiles:** a real thumbnail when the preview is cheap and safe; a colored type icon for everything else (§5).
3. Every agent can reach every file; Office documents also come with an extracted text version; Claude gets PDFs inline.
4. Nothing is written into the working folder.

## 4. Non-goals

- Rendering PDF or Office pages as thumbnails (needs a PDF/Office renderer; CEF's internal pdfium has no API). Possible later with PDF.js in the renderer or `hayro` in srv.
- Video and audio playback or first-frame thumbnails (the store route needs an auth header an `<video>` element can't send, and fetching a whole video for one frame is wasteful). Later.
- OCR of scanned PDFs, spreadsheet formulas, embedded images inside Office files.
- Opening attachments in external apps from the tray.

## 5. Tiles

Classification is by **content first, extension second** (a `.txt` that is really a ZIP is not text).

| Kind | Detected by | Tile |
|---|---|---|
| **Image** | magic bytes (unchanged) | thumbnail (unchanged) |
| **SVG** | `<svg` / `<?xml … <svg` | **thumbnail**: the file itself in an `<img>` from a blob URL. `<img>` never runs SVG scripts or loads external resources, so this is safe. Previously refused; the agent also gets it as text. |
| **Text-like** | extension in the text list (txt, md, markdown, csv, tsv, json, jsonl, yaml, yml, toml, ini, xml, html, htm, css, log, and source code: rs, ts, tsx, js, jsx, mjs, py, go, java, kt, c, h, cpp, hpp, cs, rb, php, swift, sh, ps1, bat, sql, …), **or** no extension match but the first 8 KB is valid UTF-8 with no NUL bytes | **thumbnail**: a mini page showing the first lines in a small monospace font, rendered from a `preview` file srv derives (first 40 lines, ≤ 2 KB) |
| **PDF** | `%PDF-` | icon `fa-file-pdf` (red) + page-count badge ("12 p") |
| **Word** | OOXML `word/document.xml`, or `.doc/.odt/.rtf` | icon `fa-file-word` (blue) |
| **Excel** | OOXML `xl/workbook.xml`, or `.xls/.ods` | icon `fa-file-excel` (green) |
| **PowerPoint** | OOXML `ppt/presentation.xml`, or `.ppt/.odp` | icon `fa-file-powerpoint` (orange) |
| **CSV/TSV** | (text-like, but gets its own icon when previews are off) | thumbnail as text; `fa-file-csv` fallback |
| **Archive** | ZIP (non-OOXML), gz, tar, 7z, rar | icon `fa-file-zipper` (amber) |
| **Audio** | mp3, wav, flac, m4a, ogg | icon `fa-file-audio` (purple) |
| **Video** | mp4, mov, webm, mkv, avi | icon `fa-file-video` (pink) |
| **HEIC / AVIF** | ftyp brands (unchanged sniff) | icon `fa-file-image`; attached as a file instead of refused |
| **Other** | anything else | icon `fa-file` (neutral) |

- Every icon tile shows the **extension label** (`DOCX`, `ZIP`, …) under the icon. Colors are CSS variables with brand-like defaults (`--attachment-pdf`, …) so themes can override them.
- A **macro-enabled** Office file (`[Content_Types].xml` declares a `vbaProject` part, whatever the extension) gets a small warning badge and the tooltip "Contains macros". Nothing ever opens it.
- The lightbox (click a tile) shows: images and SVGs as now; **text-like files as a scrolling text view** of the preview; everything else as the large icon with type, size, page count, "Text version extracted (N KB)" and the macro warning.
- The size summary counts all attachments: "12 attachments · 214 MB / 1 GB" (just "images" when they all are).

## 6. Processing (srv)

Same pipeline, CPU and memory limits, cancel, dedup by SHA-256 and retention as images. New per kind:

- **Store:** the original keeps a sanitized version of its real extension (`<sha256>.pdf`, `.docx`, …) so agents' tools recognise it. Metadata gains `kind`, `ext`, `page_count?`, `text_ext?`, `macros?`, `preview_ext?`.
- **Text-like:** derive `preview.txt` (first 40 lines, ≤ 2 KB, cut at a UTF-8 boundary).
- **PDF:** page count with `lopdf` (pure Rust, MIT), read under the extraction limits below; a malformed PDF just has no count. No text extraction in this spec: the PDF parsers that extract text panic on malformed input and would need process isolation.
- **DOCX / PPTX:** open the ZIP and stream only the parts needed through `quick-xml`: `word/document.xml` (plus headers, footers, footnotes); `ppt/slides/slideN.xml` in the order `presentation.xml` lists them. Emit `<w:t>` / `<a:t>` text, newlines at paragraph ends, tabs for `<w:tab/>`. Output a `text.txt` derived file with a short header ("Text extracted from report.docx by AgentMux; formatting and images are not included.").
- **XLSX / ODS / XLS:** `calamine` (pure Rust, MIT); each sheet as tab-separated rows under a `## Sheet name` heading.
- **Extraction limits** (research §7): ≤ 10,000 ZIP entries; a shared decompressed-bytes budget of 200 MB enforced with `.take()` (declared sizes can lie); refuse entries whose declared ratio exceeds 100:1; never follow nested archives; quick-xml resolves only predefined entities; extracted text capped at 2 MB (Codex's prompt limit is ~1M characters) with a "[truncated]" line; a 30-second wall-clock deadline per file. Any failure leaves the attachment attached by path only, with the tile note "No text version: <reason>".
- **SVG:** no derived files; the thumbnail is the original.

## 7. Delivery

The `<attached_images>` block becomes `<attached_files>` (the replay parser keeps reading the old name). Each line keeps the form `N. name — path`, with an optional note in square brackets after the name:

```
<attached_files>
The user attached 4 files. The numbers match how the user refers to them. …
1. screenshot.png — /…/ab….v1-e2000.send.png
2. spec.pdf [PDF, 12 pages] — /…/cd….pdf
3. report.docx [Word document; text version: /…/ef….text.txt] — /…/ef….docx
4. data.csv — /…/12….csv
</attached_files>
```

- **Claude (persistent stream-json):** images inline as now. **PDFs inline as `document` blocks** (base64, `application/pdf`, `title` = the file name) within the same per-message (20 MB) and per-session (50 MB) inline budgets, and only for PDFs of ≤ 100 pages. Everything else by path. The intro line says which files are shown above.
- **Codex, Gemini, Qwen, Kimi, ACP:** the list only (ACP `resource_link` blocks are a later refinement).
- Stored copies drop `document` blocks exactly like `image` blocks (`persisted_line`).

## 8. Frontend

- `attachments.ingest` stops returning non-images; the drop hook sends everything to the tray. `dnd:agentinserttoken` and the copy-to-cwd path stay only for `attachments:enabled = false`.
- Folder drops attach every file, skipping dot-folders and `node_modules`, `target`, `dist`, `build`, `.git`, `__pycache__`, `.venv`, within the 128 limit and the existing 10,000-entry walk cap.
- Ctrl+V and right-click Paste accept any file.
- Tiles (`AttachmentTile`) get a `kind`; icon tiles use FontAwesome classes from §5. `AttachmentInfo` gains `kind`, `ext`, `page_count`, `text_bytes`, `macros`.

## 9. Security

- Classification by content, never by the claimed name; a renamed executable is "Other", never "Text".
- ZIP and XML limits in §6; extraction never executes anything and never follows external references.
- SVG is only ever shown via `<img>` (no inline markup).
- Text previews are shown as text (`textContent`), never as HTML — including `.html` files.
- Macro-enabled documents are flagged; AgentMux never opens attachments with other applications.

## 10. Tests

- srv: classification table (content vs extension, renamed ZIP as `.txt`, OOXML detection, macros via content types); DOCX/PPTX/XLSX extraction on small fixtures; a zip bomb (high ratio, many entries, forged sizes) refused within limits; PDF page count on a fixture and a malformed PDF; text preview cut at a UTF-8 boundary; list lines for each kind; Claude line with a `document` block and its stored form.
- frontend: tile kind → icon/thumbnail; text preview rendered as text; summary wording for mixed attachments; parser reads `<attached_files>` and the old `<attached_images>`.

## 11. Rollout

1. **srv:** classification, per-kind processing and extraction, store/metadata changes, `<attached_files>` list, Claude PDF `document` blocks behind a smoke test (a unit test that the stream-json line is well-formed, plus a manual check against the pinned CLI; if the CLI rejects it, PDFs fall back to path-only).
2. **frontend:** tiles, lightbox views, all-files ingest, summary wording.

## 12. Open questions

1. PDF thumbnails (first page via PDF.js) — worth the ~1 MB renderer bundle? Deferred.
2. Video first-frame thumbnails — needs a token-authenticated range route. Deferred.
3. Should very large text files (logs) also go inline to Claude as text `document` blocks? This spec says no: the path is enough and keeps the transcript small.
