# Spec: a smaller startup bundle, so each window starts sooner

**Status:** active — step 1 (§5) in #5269; steps 2 and 3 (§6, §7) not started.
**Date:** 2026-10-21
**Author:** AgentA@Area54, at the operator's request
**Verified against:** `agentmux` `main` @ `afd0387db` (2026-10-21), by reading code and measuring a portable build (`task package -- --fresh`) on Windows 10.
**Builds on:** `SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15.md`, `SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md`, `SPEC_FE_SOURCE_MAP_RESOLVER_2026_05_27.md`.

## 1. Ask

The operator, 2026-10-21, after the startup measurements in §2: "lets explore bundle splitting", then "yes, write a plan to file and proceed".

## 2. Where a window's start goes

With migrations off the boot path (#5252) and the main window shown cloaked (#5258), AgentMux takes about 0.6–1.1 s from launch to a visible window. The largest piece left is the frontend loading in each new window.

A CPU profile of a cache-bypassing reload of the main window (about 305 ms to `DOMContentLoaded`):

| | ms |
|---|---|
| Idle: reading the 4.5 MB `index-*.js` and the 800 KB stylesheet, renderer start | 148 |
| `(program)`: mostly V8 parsing and compiling the bundle, Blink parsing the CSS | 120 |
| Our JavaScript running | 26 |

The cost is the bundle's size, not what it runs. On a cold start the main window reaches `DOMContentLoaded` at 380–590 ms.

V8's code cache does not help today. Keeping the frontend on one port between runs (tried 2026-10-21, not merged) made Chromium's HTTP cache hit. Even so, V8 neither reused nor rewrote its compiled code, and `DOMContentLoaded` did not move.

## 3. What the startup bundle holds

The main entry chunk is 14 MB of source (about 10 MB ours, the rest npm), minified to 4.5 MB. Measured from its source map:

- **Every pane type**, imported eagerly by `frontend/app/block/block-registry.ts`. The agent pane alone is 3.3 MB of source; the others (Swarm, browser, settings, files, editor, vault, tower, identity, sysinfo and more) are about 2.5 MB together.
- **Libraries that only some features use:**

| Package | Source | Used by |
|---|---|---|
| CodeMirror (`@codemirror/*`, `@lezer/*`) | ~1 MB | the editor and memory editor panes |
| `@xterm/*` | ~530 KB | terminal panes |
| `css-tree` | 409 KB | `frontend/util/muxutil.ts`, only for `url(...)` in a custom pane or tab background |
| `highlight.js` (via `rehype-highlight`) | 376 KB | code blocks in `frontend/app/element/markdown.tsx` |
| `parse5` (via `rehype-raw`) | 269 KB | raw HTML inside markdown |
| `uplot` | 142 KB | `frontend/app/view/sysinfo/sysinfo-plot.tsx` charts |
| `source-map-js` | 107 KB | `frontend/log/source-map-resolver.ts`, only after an error |
| `qrcode` | 71 KB | `frontend/app/statusbar/PairDevicePanel.tsx` |

Already lazy, and staying lazy: shiki and its languages, mermaid, pdf.js, cytoscape, katex (about 11 MB of source).

The source maps (358 files, 42 MB, about 10 MB compressed) stay. They don't affect startup: Chromium ignores them unless DevTools is open, and `source-map-resolver.ts` fetches a chunk's map only after an error in it, so the host log gets readable stacks.

## 4. Approach

Load code when the feature that needs it is first used, not at startup. Do it in steps, measuring each with the same cold-start runs: the main window's `DOMContentLoaded` over CDP, five cold starts of a portable build. Stop when the gains flatten.

Rules for every step:

- **No visible regression.** Anything on screen at startup renders the same. Something that appears a moment later (a highlighted code block) is preloaded right after startup, so in practice it is ready before it is needed.
- **A chunk that fails to load is an error the user sees**, in the place the feature would have been, with a retry. Never a silent blank.
- **One chunk per feature, shared libraries kept shared.** CodeMirror is used by two panes; it must not end up in both chunks.
- **Tests keep passing as they are:** vitest imports modules directly, so a dynamic import there resolves at once.

## 5. Step 1: libraries only some features use

No change to the pane contract. Each library moves behind a dynamic `import()` at its single use site:

- **`source-map-js`:** `source-map-resolver.ts` imports it on the first error, in its existing async path. `resolveStackSync` already returns frames unresolved when a map isn't loaded.
- **`qrcode`:** `PairDevicePanel.tsx` imports it when it draws a code.
- **`uplot`:** the sysinfo chart imports it when a chart mounts.
- **`css-tree`:**
  - A background without `url(` needs no rewriting, and passes through as before.
  - A background with one loads `css-tree` and computes the style once it arrives; until then the pane shows no background image.
- **`highlight.js`: left in.** Every markdown view would render once without colour and again with it. At startup that re-renders a whole restored conversation, which costs more than its share of the bundle (about 3%) saves. Replacing it with shiki (§8) removes it properly.

**Result:** the startup chunk went from 4,641,173 to 4,336,511 bytes (−305 KB, −6.6%). The libraries now come in their own chunks: `css-tree` (194 KB), `sysinfo-plot` with uPlot (57 KB), `qrcode` (26 KB), `source-map-js` (30 KB).

At the §2 split, that is roughly 10–15 ms of reading and compiling per window start. That is within the noise of a single cold start, so it is not measured on its own. Step 2 is where the larger share is.

## 6. Step 2: rarely open pane types as lazy chunks

`block-registry.ts` registers pane types that few layouts have open at startup as lazy entries (settings, vault, identity, tower, toolchain, warden, help, connectors and similar). Their module loads when such a pane is created.

This needs the pane contract to allow an async `create`: the pane shows its label and icon from the static part of the manifest, and a placeholder body until the module arrives. Agent and terminal panes stay in the main chunk: almost every layout has them.

A follow-up spec section settles the contract change before code.

## 7. Step 3: editor and terminal libraries

CodeMirror and xterm move into their panes' chunks, if step 2 shows those panes are often closed at startup. Decided on step 2's measurements.

## 8. Not doing

- **Replacing highlight.js with shiki in markdown.** It would remove one of two highlighters (and the last startup cost of code colouring), but it changes how every code block looks. That is a separate decision.
- **Dropping the source maps from releases** (§3).
- **A stable frontend port** for the code cache (§2): no startup gain measured.
