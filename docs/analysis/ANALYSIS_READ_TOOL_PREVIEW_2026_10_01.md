# The Read tool in the agent pane: which lines, why it sometimes collapses, and what the preview does and could do

**Date:** 2026-10-01
**Status:** implemented — §3 (the line range) and §4 (the collapse) in #4159; §3.1 (Edit and Write ranges, `start:end` before the path) in #4165; §5 (the 24 missing Shiki grammars) in #4192; §6.1 (images, PDFs, unchanged files, token-cap notes) in #4194. §7 lists further ideas, not commitments.
**Author:** korp
**Trigger:** Repo owner, 2026-10-01: *"in the read tool (agent pane) we want to know what range of lines are being read. Also I notice it sometimes comes out collapsed (instead of expanded until off the screen, then collapsing). Also investigate what previews and code highlighting we can get for Read, what is there currently, what is available?"*
**Related:** `SPEC_AGENT_PANE_ROW_DISCLOSURE_2026_09_26.md`, `PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md`, `SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md`, `SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md`, `SPEC_TOOL_OVERLAY_CODE_HIGHLIGHTING_2026_04_14.md`.

Code citations are against `main` @ `e9a45699f`.

---

## 1. Summary

1. **Range.** A Read row showed only the path. It now shows which lines were read, in the header (`120:179 of 456`) and above the preview (`lines 120–179 of 456`), including while the read is still running. The numbers come from the CLI itself, which already sends them, and the translator was throwing them away.
2. **Collapsed.** A Read is a few milliseconds. Its call and its result routinely land in one stream flush, so the row is first drawn already finished and the "expand until it scrolls off" hold, which is triggered by watching the row *change* from running to finished, never fires. Fixed by also holding a row that arrives finished with a fresh call stamp.
3. **Highlighting is much weaker than it looks.** `detectLanguage` maps about 90 extensions, but the highlighter loads Shiki's **web** bundle, which has 78 languages and **no Rust, Go, TOML, PowerShell, Dockerfile, Makefile, Ruby, Kotlin, Swift or C#**. A Read of a `.rs` or `.toml` file, which is most of this repo, renders as plain text, with only a console warning. §5.
4. **Non-text reads are poor.** An image read shows `▸ 0: {2 keys}`; a "file unchanged" stub and a PDF note are syntax-highlighted as if they were code; a token-capped read buries its warning inside the code block and breaks the gutter alignment. §6.

## 2. What agents actually send and receive

Surveyed from 18,632 Claude Code transcripts on this machine (4,812 Read calls, 3,584 results with a structured file record).

**Call parameters**

| Parameters | Calls | Share |
|---|---|---|
| `file_path`, `offset`, `limit` | 2,986 | 62% |
| `file_path` only | 1,689 | 35% |
| `file_path`, `limit` | 109 | 2% |
| `file_path`, `offset` | 25 | 0.5% |
| `file_path`, `pages` (PDF) | 3 | 0.1% |

So most reads are ranged, and a path alone does not say what was read.

**Results** (the CLI's structured `tool_use_result`, beside the model-facing text)

| Type | Count | What it carries |
|---|---|---|
| `text` | 3,311 | `filePath`, `content`, **`startLine`, `numLines`, `totalLines`** |
| `text` + `truncatedByTokenCap` | 15 | the same, with `numLines` < `totalLines`, and the model-facing text ends with a `<system-reminder>[Truncated: PARTIAL view — showing lines 1-1082 of 1320 total…]` note |
| `image` | 220 | `base64`, `type` (e.g. `image/png`), `originalSize`, `dimensions` |
| `file_unchanged` | 37 | `filePath` only; the text is *"Wasted call — file unchanged since your last Read. Refer to that earlier tool_result instead."* |
| `pdf` | 1 | `filePath`, `base64` (13 MB), `originalSize`; the text is `PDF file read: <path> (12.6MB)` |

No notebook results appeared in this sample; the shape of one is unverified.

## 3. The line range

**Sources, in order of reliability** (`tool-meta/file-range.ts`):
1. `result.range`: the CLI's `startLine`, `numLines`, `totalLines` and cap flag. The translator now keeps these small numbers from the structured result for a single-result text read, and never the file text or base64 beside them.
2. The result text: the `<N>\t` gutter's first and last numbers, and the `showing lines A-B of N total` note. This covers results recorded before (1) was kept, and history replay.
3. The call: `offset`/`limit` (Claude's, 1-based, confirmed by `startLine: 1` for a default read), `start_line`/`end_line`, or `pages`. The only source while the read is running, and wrong once the file turns out shorter than asked, which is why 1 and 2 outrank it. Another provider's `offset` is not guessed at: Gemini's may be 0-based.

**Display**
- Header chip (`.agent-tool-range`, before the path inside the name run, so a long path's ellipsis cuts the path and never the range): `120:179`, `120:179 of 456`, `1:214` for a whole file, `pages 1–5`. It shows from the moment the call lands, from the parameters, and updates to the actual range when the result arrives.
- Above the preview: `lines 120–179 of 456`, `all 214 lines`, or `lines 1–1082 of 1320 · cut off at the token cap`.
- The plain-text header form (used by `/btw`) ends with the chip.

### 3.1 Edit and Write, and the format

The owner asked for the same on Write: *"if it is writing to only a part of the file, are line numbers available?"* From the same survey of real transcripts (4,400 file-change results):

| Tool | Result type | Count | What it carries |
|---|---|---|---|
| Edit | (none) | 3,829 | `structuredPatch`: hunks with `oldStart`, `oldLines`, `newStart`, `newLines`, and the diff `lines`; also `oldString`, `newString`, `replaceAll`, and `originalFile` (the whole old file) |
| Write | `create` | 540 | `content`, an **empty** `structuredPatch` |
| Write | `update` | 64 | `content`, and a `structuredPatch` against the file it replaced |

**Write has no partial form**: its parameters are `file_path` and `content`, so it always writes the whole file. The answer to "is it writing only part of a file" is Edit. Both are covered now:
- **Edit**: the changed lines, in the file as it is after the edit, from the patch. A hunk carries three lines of context either side, so the span runs from the first to the last `+`/`-` line, not the hunk's own bounds. A deletion sits where the next line now is. Several hunks show as `5:11, 40:46`, or `10:31 · 3 places` beyond two. It is known only once the result is in: before that the call has an old and a new string, and the pane doesn't have the file.
- **Write**: `1:N` from the content being written, known while it runs. The preview line says `new file, N lines` for a create, or `all N lines · differs at 16:118` for an overwrite.
- The translator keeps only these numbers: never the diff text, and never `originalFile`.

**Format.** The header chip is `start:end` (`33:334`), placed **before the path** inside the name run: `Read 33:334 C:\…`. A long path's ellipsis then cuts the path and never the range. A single line is `42:42`, because a lone number reads as a count. Whole files read `1:214`. When a Read covers only part of a known-length file it adds `of 456`. The long form above the preview stays in words.

Not done: a diff view with real line numbers in its gutter. The patch hunks have what it needs; `DiffViewer` builds its diff from the old and new strings and shows none.

## 4. Why a Read sometimes comes out collapsed

**The rule.** A finished tool row stays open "until its row scrolls off the top" (`PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE`). Mechanically, `ToolBlock` watches for the row's status changing from running to finished while mounted and then adds the tool to `documentState.expandedTools`; the virtual list releases it once it scrolls off. A row first drawn *already finished* never changes, so it is never held. That was deliberate: a loaded transcript must not open every row (codex P1 on #988).

**What goes wrong.** The stream parser emits the running node and then, when the result arrives, an update for the same id. Both go into one queue and are flushed together once per animation frame. The reducer's own comment says so: *"events that share an animation frame can produce a newNode AND an updatedNode for the same id."* They are applied in a single pass, so the document gains the tool already finished. A Read finishes in a few milliseconds, so this is the common case for Read, Grep, Glob and short Bash commands, and the reason it is "sometimes": it depends on whether the call and the result straddled a frame boundary. A slow tool (a long Bash) is always seen running first.

**The fix.** A finished row that arrives with a live call stamp is held on arrival. Live calls are stamped with `Date.now()` when the call arrives; replayed ones have no stamp (`stream-parser.ts` `isReplay`), which is the existing, reliable way to tell live from history. A row counts as a live completion when it is finished, not denied or canceled, and stamped within the last 5 seconds (comfortably longer than the flush delay under load, far shorter than any history). Loaded transcripts carry no stamp and still open collapsed. It applies to every fast tool, which is the behaviour the rule already promised.

**Not changed:** a row that completes while it is not mounted (a hidden pane, or scrolled off) still opens collapsed. The hold is for "finished on screen".

## 5. Code highlighting

**What is there.** `HighlightedCode` renders a plain `<pre>`, then swaps in Shiki HTML (theme `github-dark-high-contrast`), cached by `(lang, code)`, skipped above 200 KB or 2,000 lines, with a 1,000-line head cap upstream (`MAX_TOOL_OUTPUT_LINES`) and a hidden-lines marker. Markdown files render as Markdown. Indentation is dedented and narrowed to 2 columns; the `<N>\t` gutter is re-emitted right-aligned. `DiffViewer` loads Shiki the same way.

**The gap (closed, see below).** Both imported `shiki/bundle/web`. Checked directly against the installed Shiki 3.23:

| Bundle | Languages |
|---|---|
| `shiki/bundle/web` (what ships) | 78 |
| `shiki/bundle/full` | 332 |

Mapped by `detectLanguage` but **not in the web bundle**: `rust`, `go`, `toml`, `powershell`, `fish`, `dockerfile`, `makefile`, `ini`, `ignore`, `ruby`, `kotlin`, `swift`, `csharp`, `lua`, `terraform`, `hcl`, `dart`, `elixir`, `elm`, `haskell`, `clojure`, `scala`, `groovy`, `perl`. `codeToHtml` throws for each (*"Language `rust` is not included in this bundle"*), the component catches it, logs `console.warn`, and stays on plain text. Present and working: TypeScript/TSX/JS/JSX, Python, Bash, Markdown, JSON/JSONC, YAML, XML, HTML, CSS/SCSS/Less, SQL, Java, C, C++, PHP, Vue, Svelte, GraphQL, R.

The same cap applies to the Write preview, Edit diffs, and fenced code in agent messages, wherever the web bundle is used.

**Cost to close it.** An earlier draft of this section summed each grammar's own file and said "about 370 KB raw for twelve". That overstated it: some grammars embed others (Ruby pulls in 20 files, mostly HTML, JavaScript and CSS), and most of those embedded grammars already ship with the web bundle. What counts is the grammars that don't ship yet:

| Set | Raw | gzip | Brotli |
|---|---|---|---|
| Rust, TOML, Go, PowerShell | 96 KB | 13 KB | 11 KB |
| + Dockerfile, Make, INI | 109 KB | 16 KB | 14 KB |
| + Kotlin, Swift, C#, Lua | 324 KB | 46 KB | 40 KB |
| All of them (the 23 above, less `ignore`, plus `dotenv`) | 600 KB | 84 KB | 73 KB |

Measured from `@shikijs/langs/dist`. The shipped JavaScript is about 12.8 MB raw and 3.2 MB gzipped, so all of them add under 3%. Each grammar is its own chunk, loaded the first time a file in that language is highlighted: nothing at startup.

**Closed.** `components/shiki-highlighter.ts` builds a highlighter from the web bundle's languages plus these 24 (`createBundledHighlighter`, the same Oniguruma engine), and `HighlightedCode` and `DiffViewer` load it in place of `shiki/bundle/web`. In a production build the 24 come out as separate chunks totalling about 530 KB raw and 82 KB gzipped. Shiki has no grammar for ignore files, so `.gitignore` and friends now map to plain text instead of failing; `.env` files map to `dotenv`. A test checks that every language `detectLanguage` can return has a grammar.

**Also available:** the Editor pane's CodeMirror has language packs for seven languages (including Rust), and a read-only CodeMirror view could serve long files better than a `<pre>` with innerHTML (selection, folding, find), at a higher mount cost.

## 6. What each kind of Read shows today

Rendered in a throwaway test (`ToolBlock`, pinned open) with the shapes from §2.

| Read of | Header | Body today | Problem |
|---|---|---|---|
| Source file, whole | `📖 Read path` | Highlighted code with right-aligned gutter | Fine for supported languages; plain for the 24 in §5 |
| Source file, ranged | `📖 Read a.ts` | Same, gutter shows the real line numbers | The header did not say it was a slice (§3) |
| Markdown | `📖 Read README.md` | Rendered Markdown | Fine |
| Token-capped | `📖 Read a.ts` | The `<system-reminder>…PARTIAL view…` note is **inside the code block**, and because that line has no number the gutter check fails, so the whole preview falls back to raw `  1\t…` with tabs | The truncation is the most important fact and is the least visible; alignment breaks |
| Image | `📖 Read a.png` | `▸ 0: {2 keys}` | No thumbnail, no dimensions. 6% of reads in the sample |
| "File unchanged" | `📖 Read path` | The "Wasted call…" sentence, highlighted **as code** in the file's language | Reads as a bug; should be one muted line |
| PDF | `📖 Read a.pdf` | `PDF file read: … (12.6MB)` highlighted as text | No page info (`pages` is not shown); no way to open it |
| Notebook | not seen | unknown | Unverified |
| Error (not found, too large) | | The generic compact result | Not examined |

### 6.1 Now

The repo owner asked to *"see the image or whatever"*. Done in the PR after the grammars:
- **Image:** the translator keeps an image result as `{content, images: [{mediaType, data}], file: {kind, size, width, height}}` (`mediaResultOf`) instead of the raw blocks. The row shows the image itself (`ResultImages`, from the base64 already in memory, no fetch) under a line such as `PNG image · 240 × 160 · 3.5 KB`. The known dimensions reserve its height before it decodes. Clicking it opens the file in a Media pane. Only `image/*` types go into a data URL.
- **MCP screenshots** and any other tool returning image blocks get the same image in the default renderer, with the result's text below it. With no file to open, a click toggles full size.
- **PDF:** one line, `PDF · pages 1-5 · 708.4 KB`, and **Show in folder**. The document's base64 (often megabytes) is no longer kept in the pane. An "open with the default app" button needs `open_native_path`, which the CEF host doesn't handle yet (`openNativePath` falls through to "Unknown command").
- **File unchanged:** one muted line, `unchanged since the last Read`, in place of the note highlighted as code.
- **Token cap:** the CLI's trailing `<system-reminder>` notes are taken off the preview, so the gutter renders. The range line already says `· cut off at the token cap`. Only trailing notes are removed, since a file may mention the tag.
- Large image results still unload when collapsed and are read back on open, through the same translator.

## 7. What is available to improve the preview

| Idea | What it uses | Size |
|---|---|---|
| Keep and show the CLI's facts (range, total, cap, image type and dimensions, file size) | Already in the stream; §3 keeps the line numbers | Small |
| Move the token-cap note out of the code into the row | Already parsed in `read-range.ts` | Small |
| One muted line for `file_unchanged` and PDF notes, no highlighting | Result type | Small |
| Image thumbnail with dimensions and size, click to enlarge | The image is already in the `tool_result` as base64; the attachment lightbox pattern exists (`AttachmentLightbox`), but it fetches attachments by id from srv, so a data-URL variant is needed | Medium |
| Add the missing Shiki grammars | §5 | Small to medium |
| "Open in Editor" on the path, and "Reveal" | `OpenEditor` and `reveal_in_file_explorer` exist | Small |
| Highlight the read range inside the file (open the file with the range selected) | Editor pane | Medium |
| PDF thumbnail or page view | No PDF renderer in the app; the Media pane has none | Large; not recommended without a need |
| A "Read again" / "changed since" marker | Needs a file watcher or a stat on open | Medium; separate design |

**Recommended order:** (1) the Shiki grammars, since the effect is largest and the cost small; (2) the token-cap note and the `file_unchanged`/PDF lines; (3) the image thumbnail; (4) Open in Editor. Each is independent.

## 8. How this was checked

- Transcript survey: a script over the Claude Code project transcripts counting the Read call parameters and the `toolUseResult` shapes (§2).
- Rendering: each result shape was rendered through `ToolBlock` in a scratch test and its header, language and body text read back (§6).
- Highlighting: `shiki/bundle/web`'s `bundledLanguages` against `detectLanguage`'s map, and a direct `codeToHtml` call per language to confirm which throw (§5).
- The collapse is established from the code (the flush queue, the reducer comment, the `ToolBlock` rule) and from the tests: the new tests for a row that arrives finished fail on the old `ToolBlock` and pass on the new one. It was **not** observed live in a running pane; the first live Read after this ships is the check.
