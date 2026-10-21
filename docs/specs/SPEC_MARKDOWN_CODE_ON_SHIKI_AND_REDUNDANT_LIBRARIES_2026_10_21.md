# Spec: code in markdown on shiki, and the frontend libraries we can drop

**Status:** active — §4 (markdown code on shiki) in #5289; the removals in §6.2 in #5291. Remaining: §6.3's `overlayscrollbars`, a separate decision.
**Date:** 2026-10-21
**Author:** AgentA@Area54, at the operator's request
**Verified against:** `agentmux` `main` @ `61479f0a2` (2026-10-21), by reading code.
**Builds on:** `SPEC_FRONTEND_BUNDLE_SPLITTING_2026_10_21.md` (§8 named this as a separate decision), `SPEC_TOOL_OVERLAY_CODE_HIGHLIGHTING_2026_04_14.md` (shiki in the agent pane).

## 1. Ask

The operator, 2026-10-21, after a comparison of the two highlighters: "yes, move to shiki, write a plan to file first, also follow up with any other redundant libraries we can toss".

## 2. Today: two highlighters, two looks

| | highlight.js | shiki |
|---|---|---|
| Colours | Code blocks in markdown (`frontend/app/element/markdown.tsx` via `rehype-highlight`): agent replies, Memory and docs previews, `.md` files | Tool blocks, Hangar's file preview (`HighlightedCode`), shell commands (`shell-highlight/embedded-highlight.ts`), preview text (`preview-text/highlight.ts`) |
| Theme | GitHub Dark Dimmed (`markdown.scss` imports `highlight.js/scss/github-dark-dimmed.scss`), in every app theme | GitHub Dark / Light High Contrast; the shell highlighter carries both and picks by the theme's polarity |
| Loads | At startup, in every window (376 KB of source) | When first needed; each language on its own |
| How | Pattern rules, synchronous, inside the rehype pipeline | VS Code's TextMate grammars, asynchronous |

So the same snippet looks different in an agent's reply than in the tool block that read the file. In a light theme, reply code blocks show dark-theme colours on the light panel background (`pre.codeblock` uses `--panel-bg-color`).

## 3. Goal

- Every code block in markdown is coloured by shiki, with the same themes as the rest of the app, in light and dark.
- highlight.js and `rehype-highlight` leave the dependencies and the startup bundle.
- Nothing visible gets slower or worse while it happens: no flash of a whole message, no long freeze for a big block, no colour while a block is still streaming in.

## 4. Design

### 4.1 Where it happens

Not in the rehype pipeline: shiki is asynchronous and the pipeline is synchronous. The `Code` component that `markdown.tsx` already renders for every `<code>` (`markdown-codeblock.tsx`, which also handles Mermaid) colours its own block:

- **Which blocks.** A fenced block with a language (`class="language-x"`), as `rehype-highlight` did; it never guessed a language. Inline code and blocks without a language stay plain.
- **How.**
  - It renders the text as plain at once, the same text and DOM as today without colour.
  - It asks `highlightBody(text, lang)` (`shell-highlight/embedded-highlight.ts`), the shell highlighter's function. That function already gives coloured runs with both themes' colours as CSS variables, keeps only shiki's own variables, applies the same caps as `HighlightedCode` (200 KB, 2,000 lines) and caches results.
  - When the runs arrive, it replaces its text with `<span>`s styled by those variables. Solid builds the spans: no `innerHTML`, so nothing bypasses the markdown sanitizer.
- **Which colour shows.** A rule like `.sh-shiki span` (`_document-nodes.scss`) picks `--shiki-dark` or `--shiki-light` by `[data-theme-polarity]`. It moves to a shared stylesheet so markdown and shell commands use the same one.
- **Unknown languages** (shiki has no grammar, or the load fails) stay plain. `rehype-highlight` left those plain too.

### 4.2 Only one block changes, never the message

Colour arriving re-renders that block's `<code>` only. The markdown processor no longer depends on a highlighter that loads late, which is why highlight.js couldn't simply be loaded lazily (bundle-splitting spec §5). A restored conversation renders at once, then each code block gains colour on its own.

### 4.3 Streaming

`markdown.tsx` already builds the trailing open block without highlighting while a reply streams (`tailProcessor`), because colouring a growing block on every commit was the expensive part. That stays. The processor tells `Code` whether to colour through the existing `highlight` flag, so a block still streaming in is plain and colours once it closes. That costs one tokenise per block, and the cache makes later renders of the same block instant.

### 4.4 What goes

- `rehype-highlight` from the rehype plugins, and `highlight.js` and `rehype-highlight` from `package.json`.
- The `@import` of the highlight.js theme in `markdown.scss`.
- `/^hljs-./` from the span classes the markdown sanitizer allows (`AM_SPAN_CLASSES` stays). Nothing else emits `hljs-*` classes.

### 4.5 Tests

- **`Code`:**
  - plain first, then coloured spans when the runs arrive;
  - no colour without a language;
  - no colour while `highlight` is off, as for a streaming tail;
  - an unknown language stays plain;
  - Copy copies the same text before and after colour.
- **Existing markdown tests** that assert `hljs-*` classes (`markdown-semantic.test.tsx`, `MarkdownBlock.stream-pauses.test.tsx`) change to assert the shiki path. A test that pins processor builds keeps its counts: the processor no longer rebuilds for a highlighter.
- **By hand, in a build:** a reply with Rust, TypeScript, Python, shell, JSON and an unknown language, in a dark and a light theme, compared with the same code in a tool block.

### 4.6 Result

- **Startup chunk:** 3,183,510 → 3,019,614 bytes (−164 KB, −5.1%). highlight.js, `rehype-highlight` and `lowlight` are gone from the build.
- **Checked by hand** in a portable build, in Hangar's markdown preview of a sample with Rust, TypeScript, Python, shell, JSON, an unknown language and a block without one:
  - in the dark theme, the five known languages are coloured, and the unknown language, the block without one and inline code stay plain;
  - in the light theme, the same blocks show GitHub Light colours on the light panel, where highlight.js had shown GitHub Dark Dimmed colours.

## 5. Risks

- **First colour of a language takes a moment.** Shiki loads that language's grammar on first use. The block is readable plain meanwhile; this is the same behaviour tool blocks already have.
- **Very long conversations full of code.** Each block tokenises once. The cache in `embedded-highlight.ts` is small (100 entries), so it may need a byte-bounded cache like `HighlightedCode`'s; that is checked with a long conversation by hand.
- **Look.** Replies' code changes from GitHub Dark Dimmed to GitHub Dark High Contrast, the colours tool blocks already use. That is the point of the change, and is said in the changeset.

## 6. Redundant libraries

### 6.1 The audit

Every runtime dependency in `package.json` was traced to its import sites under `frontend/` (static, dynamic and SCSS imports). The size of each was measured, minified, from the source maps of a production build, along with whether it lands in the startup chunk.

**No dependency is unused.** Mermaid brings in its own markdown parser (`marked`), sanitizer (`dompurify`), date library (`dayjs`) and utility libraries (`lodash-es`, `es-toolkit`). Those load lazily with Mermaid and are not ours to drop.

### 6.2 What goes (PR 2)

| Package | Why it can go | Replacement | Minified, startup |
|---|---|---|---|
| `tailwind-merge` | Only `cn = twMerge(clsx(…))` in `util/util.ts`, 8 calls; two of them rely on its merging | `cn` becomes `clsx`; the two conflicts (a table cell's alignment, the table copy button's copied state) become explicit conditionals | 26.9 KB |
| `html-to-image` | One use: the drag preview in `TileLayout.core.tsx`, already async | Stays a dependency, loaded with `import()` on first drag | 12.9 KB off startup |
| `rxjs` | One `Subject` (`store/mps.ts`) with `next`, `subscribe`/`unsubscribe` and `complete` | A 20-line emitter with the same four methods | 8.1 KB (9 MB installed) |
| `debug` | About 18 calls in 4 files (`app.tsx`, `rpc-router.ts`, `ws.ts`, `termwrap.ts`) | `debugLog` (`util/debuglog.ts`): off unless `localStorage.debug` names the namespace, the same switch. Not `Logger.debug`, which always prints and sends every line to the host log, and these come once per RPC message or focus change | 6 KB |
| `colord` | One call: `colord(c).alpha(a).toHex()` in `termutil.ts` | `applyTransparencyToColor` reads hex and `rgb()` itself and anything else through a canvas, then sets the alpha as colord did. Tested against colord's own outputs. One difference: a colour it can't read is returned as it is, where colord gave semi-transparent black | 5.8 KB |
| `sprintf-js` | One call: `sprintf("sleeping %ds", timeout)` | A template literal | 3.3 KB |
| `base64-js` | One call, `fromByteArray`, in `stringToBase64` | `btoa` over the bytes (the neighbouring decode already uses `atob`) | 1.4 KB |

Also: `frontend/util/wsutil.ts` imports the Node `ws` package in a branch that never runs in the app (`typeof window === "undefined"`). The branch goes; `ws` stays a dev dependency for scripts.

Their `@types/*` packages go with them. Measured: the startup chunk went from 3,183,510 to 3,131,513 bytes before #5289 (−52 KB); with #5289, about 2.97 MB.

### 6.3 What stays, and why

| Package | Why it stays |
|---|---|
| `overlayscrollbars` (~43 KB with its CSS) | Native scrollbars could replace it, but they don't auto-hide, so markdown would visibly change. Not part of this plan; a separate decision |
| `css-tree` (lazy) | It vets `url()` in user backgrounds, a security check; replacing it is riskier than its lazy chunk costs |
| `rehype-raw` and `parse5` | Raw HTML in agent replies (`span`, `waveblock`, `mermaidblock`) needs a real HTML parser |
| `@xterm/*`, CodeMirror, Mermaid, pdf.js, uPlot, `qrcode`, `source-map-js`, `fuse.js`, `linkify-it`, `@floating-ui/dom`, `@atlaskit/pragmatic-drag-and-drop`, `clsx`, `throttle-debounce`, `solid-transition-group`, `@lobehub/icons-static-svg` | Each is the only library for its job, used widely, or already lazy. CodeMirror edits and shiki displays, so they are not duplicates |

## 7. Plan

1. **PR 1:** markdown code on shiki (§4), dropping `highlight.js` and `rehype-highlight`.
2. **PR 2:** the library removals from §6 that are safe, each with its replacement and tests.
3. Measure the startup chunk before and after each PR, as in the bundle-splitting spec.
