# Tool previews: why the formatting breaks, and one pipeline to fix it

**Status:** active — Phase 0 items 1, 2 and 5 (bashwrap's echoed cursor report, the 200-column PTY, stale doc statuses) in #4508; items 3 and 4 moved into Phase 2 (§6). Phases 1–3 are in progress.
**Date:** 2026-10-08.
**Requested by:** repo owner (asafebgi): "the preview formatting sometimes doesn't process right. We had a couple work on it including simulating 2 space tabs that doesn't appear to work. Sometimes lines are wordwrapped for no reason … I worry the code is spaghetti and isn't cleanly processing display in a robust way … we want a clear preview parser that ensures previews are always clean."
**Author:** Masty.
**Related:** `docs/specs/SPEC_TOOL_PREVIEW_DEDENT_2026_08_08.md`, `docs/analysis/tool-preview-indentation-and-wrapping-2026-09-02.md`, `docs/specs/SPEC_TOOL_OUTPUT_TEE_AND_TERMINAL_RENDER_2026_06_17.md`, `docs/specs/SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md`, `docs/reports/REPORT_TOOL_PREVIEW_DRY_AND_ARCHITECTURE_2026_09_26.md`.

"Preview" here means the capped boxes under each tool call in the agent pane: Read, Write, Edit, Bash, Grep/Glob, MCP and other tools, web results, jekts. All paths below are under `frontend/app/view/agent/` unless they start with `crates/` or `frontend/app/`.

---

## 1. Summary

The concern is justified. There is no preview pipeline: each preview type takes its own route from the tool result to the screen, with its own text changes and its own CSS. The two symptoms come from that:

- **"2-space tabs don't work."** No code turns tabs into spaces. The only lever is one CSS rule, `tab-size: 2`, on one part of the tool panel. It doesn't reach jekts, the persistent shell or the activity dock. Where it does reach, tab stops are counted from the start of the line *including the line-number gutter*, so the first indent level of a tab-indented file comes out half as wide as the rest. Separately, the JS indent narrowing gives up the moment a file has a tab.
- **"Lines wrap for no reason."** Three separate causes:
  1. Every plain-text result (Grep, Glob, MCP, Task, the default renderer, a Read that fell back) goes through a viewer that soft-wraps anywhere, mid-path and mid-token. Code previews (Read, Write, Edit, Bash) never wrap. So whether a line wraps depends on which tool produced it.
  2. Every Bash command runs in an **80-column terminal** (`crates/bashwrap/src/bash_wrap.rs:990`, `COLUMNS=80`). Programs that size their output to the terminal (tables, help text, column lists, progress bars) put real line breaks at column 80 before the preview ever sees the text. No CSS can undo that.
  3. While a tool streams, each output chunk is its own block. bashwrap flushes at 4096 bytes or 50 ms of quiet, so a line can break mid-line while streaming and rejoin when the tool finishes.

There are also defects nobody has reported yet:
- Every Bash preview starts with the literal text `^[[1;1R`.
- The rules that style highlighted diff lines are dropped by the browser, so they never apply.
- The expanded-JSON hanging indent shifts every line but the first.
- About six CSS rules and two components are dead.

The fix is one pipeline: a pure text stage that turns any tool output into a structured document, one renderer with one DOM shape, and one stylesheet mixin with two explicit wrap modes. §5 describes it and §6 gives a phased plan. Some quick wins (§6, Phase 0) are worth doing now, independent of the redesign.

## 2. What was measured

Everything in §3 was checked against code. The main claims were also measured in a real browser: a dev build of `main` at `34e3f1886`. The real renderers came from the app's own tool renderer registry, rendered inside the agent pane's real ancestor chain (`.agent-view .agent-tool-block .agent-tool-panel .agent-tool-overlay-log`) at 520 px, with the app's own CSS. The harness is not in the repo; §7 proposes a permanent version.

| Preview | Input | Measured |
|---|---|---|
| Grep (string result) | one 150-character `path:line:code` line with no spaces | **3 screen lines** for 1 logical line, broken mid-path. `white-space: pre-wrap`, `overflow-wrap: anywhere`, `text-indent: -3ch` |
| Read | CLI-format gutter (`     6\t…`), tab-indented Go, 12 lines | Tabs survive and get `tab-size: 2`. After the 3-character gutter, code starts at 3.1ch (level 0), **4.1ch (level 1)**, 6.1ch (level 2), 8.1ch (level 3). The first level is 1 column, the others 2 |
| Bash | stdout as bashwrap returns it, with tabs | First line renders as **`^[[1;1Rabc1234  fix…`**. `white-space: pre`, no wrap, horizontal scroll (815 px content in a 504 px box). The 7-character artifact also shifts that line's tab stops, so the two rows of tab-separated output don't line up |
| Edit (highlighted diff) | a one-line change | The added line is an **inline** `span`: its green background covers 94 px of a 506 px line. The rule that made diff lines full-width blocks is dropped by the browser (see §3.4). Marker opacity is 1; the dead rule wanted it dimmed |
| Expanded JSON (structured result) | a small object | Left edges of the lines: 7 px for `{`, then **27 px or more for every other line**. `text-indent` affects only the first line of a block, and the whole JSON is one `<pre>` |
| Jekt body, context-delivery body | any text | `tab-size: 4`, not 2 |
| Live CSS object model | — | **0** rules with `:global(` in the selector; **0** rules targeting Shiki `.line` elements in previews. The source has eight such blocks and the built CSS (`dist/frontend/assets/style-*.css`) contains them verbatim |
| Bash environment | `python3 -c "shutil.get_terminal_size()"` | `columns=80, lines=24`; `COLUMNS=80` |

## 3. Findings

### 3.1 There is no single path, so every symptom is local

From tool result to DOM (`ToolOverlayLog.tsx:452-465` picks the branch):

- **Read:** `withoutTrailingNotes` → `capText` (1000 lines) → `formatReadPreview` (`components/dedent.ts:301`: split the gutter, remove the common indent, narrow space indentation to 2 columns, re-emit the gutter) → Shiki → one `<pre>` with inline `.line` spans.
- **Write:** cap → `formatCodePreview` or `formatMarkdownPreview` → the same `FilePreview`.
- **Edit:** raw `result.diff` with **no dedent**, or `formatDiffSides` → an LCS diff built in `DiffViewer.tsx` → one of two DOMs (plain: a block `div` per line; Shiki: one source string with inline spans).
- **Bash, finished:** `parseExitPrefix` → `capText` (tail) per stream → two separate `<pre>`s (stdout, stderr), so the order between them is lost. No ANSI handling here; the backend strips escape sequences.
- **Bash and other tools, streaming:** spinner collapsing → chunk capper → one `<pre>` **per chunk**.
- **Grep, Glob, MCP, Task, default renderer:** `terminalText` → single-line vs multi-line choice (made on the trimmed text, then the untrimmed text is rendered) → `TerminalOutput` → `AnsiLine`, one `<div>` per line. It parses colour codes only and resets them on every line.
- **Agent report, WebSearch summary:** Markdown, with highlight.js for code fences, not Shiki.
- **WebFetch:** trimmed plain text, **no cap**.
- **Jekt:** strip the envelope, then trim → `<pre>` with linkified text.

The same jobs are done several times, differently:
- the gutter regex (`dedent.ts:52` and `tool-meta/file-range.ts:76`)
- Shiki loading, theme and `<pre>` slicing (`HighlightedCode.tsx` and `DiffViewer.tsx`), with the theme hard-coded to `github-dark-high-contrast`
- three ANSI implementations (`AnsiLine`, `sanitizeLogTextForTerminal`, the backend `strip_ansi`)
- four ways of picking text out of a result
- trimming that varies by path
- caps that vary from 200 rows to unlimited

### 3.2 Tabs: one CSS rule, partly applied, with the gutter in its tab stops

- `frontend/app/reset.scss:26` sets `tab-size: 4` everywhere, and Tailwind's preflight sets it again.
- The only override is `tab-size: 2` on `.agent-view .agent-tool-block .agent-tool-panel` (`styles/_document-nodes.scss:643`, PR #2785).
- **Not reached (still 4):** jekt body and raw payload, context-delivery bodies, the persistent shell log (`PersistentShellBlock.tsx` renders `.agent-tool-panel` under `.agent-shell-block`, not `.agent-tool-block`), the activity dock log, message bodies, and the hover peek (portaled outside `.agent-view`).
- **No JS converts tabs.** `normalizeIndentWidth` returns early if any leading run contains a tab (`dedent.ts:171`, "tab width is CSS's business"). A tab-indented file is therefore not narrowed at all, and a file mixing tabs and spaces keeps its space indentation at full width.
- **Tab stops include the gutter.** The Read gutter is re-emitted as right-aligned digits plus a space, so code starts at column W+1. With an odd gutter width the first tab lands one column away and the rest two (measured above). In diffs the one-character `+`/`-` marker does the same. A tab inside `TerminalOutput` or the JSON `<pre>` is also measured from a −3ch text indent.
- **The setting is wrong for output.** `tab-size: 2` also applies to Bash and terminal output, which isn't dedented. Output that lines columns up with tabs (`git log --format=%h%x09%s`, TSV, `ls -l` variants) expects 8-column tabs and misaligns at 2.
- The 2026-09-02 analysis measured 0 of 127 indented lines in a real Read using tabs and concluded the rule did nothing. It also found it caused the gutter to step sideways at line 10. The gutter fix (#2958) removed the stepping but not the parity effect above.
- `PREVIEW_INDENT_UNIT = 2` (`dedent.ts:41`) and `tab-size: 2` are kept equal by a comment, not by code.

### 3.3 Wrapping: four policies and no owner

| Policy | Where |
|---|---|
| `pre`, horizontal scroll | Read, Write, Edit, Bash (finished and streaming), activity dock log |
| `pre-wrap` + `overflow-wrap: anywhere` (breaks mid-token) | `TerminalOutput` (every string result: Grep, Glob, MCP, Task, default, Read fallback), single-line compact results, expanded JSON, record-table cells, activity dock command |
| `pre-wrap` + `word-break: break-all` (always mid-token) | jekt raw payload, hover peek |
| `pre-wrap`, no break rule (long tokens spill sideways) | jekt body, context-delivery body, message bodies |
| `word-break: break-word` | WebFetch |

Consequences:
- **The same text wraps or not depending on the tool.** A long path in a Read never wraps; the same path in a Grep result wraps mid-name.
- **Output reflows when a tool finishes.** For every non-Bash tool, streaming chunks are `pre` and the final `TerminalOutput` is `pre-wrap`. `_tool-overlay-portal.scss:62-76` records an attempt to unify them that was reverted.
- **The hanging indent only half works.** It works per line in `TerminalOutput` (one `div` per line). It doesn't work in JSON (one `pre`), and it shifts the "N lines hidden" marker about 20 px left of its box (`TerminalOutput.tsx:45`; the marker's own padding rule wins on specificity).
- **Narrow tables break cells mid-token.** `table-layout: fixed; width: 100%` from `_document.scss:221` beats the record table's own rules.
- **Inner horizontal scrollbars are often out of view.** In tall previews, the inner element's horizontal scrollbar sits at its bottom, below the visible area.

There are about 30 rules governing wrapping and tabs in previews, about six of them dead. There is no shared "code text" mixin: each preview sets its own `white-space`, break rule and line height (1.35, 1.4, 1.45 or 1.5).

### 3.4 Dead CSS that removes styling silently

`agent-view.scss` is imported as a plain stylesheet (`agent-view.tsx:65`), not a CSS module. The `:global(...)` wrappers in `styles/_document-nodes.scss` therefore reach the browser verbatim, and a browser drops any rule whose selector it can't parse:
- `:896-900`: Shiki `.line` display, padding and min-height
- `:971-993`: every highlighted-diff line background, the marker and hunk text
- `:1052-1055`

In the Shiki path, diff lines are therefore inline and their highlight covers only the glyphs (measured). The other dead code:
- `.agent-bash-cmd-code` (its `BashCommandView` is never used)
- `.agent-tool-search .agent-tool-search-results` (never matches)
- `.agent-tool-generic`, `.agent-tool-task-info`, `.agent-raw-output` (no TSX uses them)

### 3.5 Text that should never reach the screen

- **`^[[1;1R` on every Bash result.** bashwrap pre-writes a cursor-position report into the terminal (`crates/bashwrap/src/bash_wrap.rs:1149`). The terminal echoes it back as the printable characters `^[[1;1R`, which escape-code stripping can't recognise. It is the first text of every Bash tool result, in the preview and in what the model receives.
- **Read's gutter goes into Shiki as code** (`tool-renderers/builtins.tsx:221`), so line numbers are tokenised by the file's grammar.
- **Shebang detection doesn't work for Read.** It looks at the first line, which still has the gutter on it (`builtins.tsx:85`).
- **The raw tab gutter comes back** whenever `splitNumberedGutter` declines (any line without a number), or when a Read has no `content` and falls back to `TerminalOutput`, which then wraps.
- **Carriage returns, backspaces and non-colour escape codes** are shown as raw text by `AnsiLine`, for example progress bars that redraw a line.
- **Truncation markers** are inserted as text (`"\n…(truncated)"`, `output-cap.ts:65`), so they take part in dedent and highlighting.

### 3.6 Why repeated fixes didn't hold

| PR | What it did |
|---|---|
| #1798 | Removed wrapping from streaming chunks only. |
| #2785 | Added `tab-size: 2` to one subtree, which turned out to do nothing for space-indented code and to step the gutter sideways. |
| #2958 | Fixed the gutter and added space narrowing and the hanging indent. It kept `overflow-wrap: anywhere` against its own analysis. Its squashed commit message describes a streaming change it reverted. |
| #3877 | Sent more tools to the wrapping viewer. |
| #3933, #3936, #4473 | Unified scrolling and caps, not text or wrap rules. |

Each fix was local to the path in front of it, and nothing was checked in a live pane. The dedent spec (§5) and the 09-02 analysis (§7) both say so.

Tests check JS transforms in jsdom, which has no layout. The one tab-stop test (`dedent.test.ts:233`) simulates `tab-size: 2` in JS. No test checks that any wrap or tab rule exists or applies.

Several docs disagree with the code:
- The dedent spec still presents `tab-size: 2` as the fix.
- `SPEC_TOOL_PREVIEW_REFINEMENTS_2026_06_26` says "proposed" though it was built.
- `ANALYSIS_READ_TOOL_PREVIEW_2026_10_01` says "none implemented" though §6 shipped.
- `SPEC_TOOL_OVERLAY_CODE_HIGHLIGHTING_2026_04_14` says "Draft" though it was built.

## 4. What "always clean" should mean

1. Text that isn't content never shows: terminal control replies, escape codes other than colour, carriage-return redraws, the Read gutter as text, CLI notes.
2. Indentation looks the same in every preview: one tab width, applied in one place, measured from the code's own first column, never the gutter's.
3. Wrapping is a decision per kind of content, made in one table, not a side effect of which component rendered it. Code and command output don't soft-wrap. Prose does, at word boundaries first.
4. Line breaks in the source are the only line breaks that aren't soft wraps. Nothing is folded at a fixed width upstream (the 80-column terminal), and streaming doesn't add breaks the finished view doesn't have.
5. The streaming and finished views of the same output look the same.
6. One DOM shape: one block per line, with gutter and markers in their own column. Highlighting, wrapping, hanging indents and diff backgrounds then behave the same everywhere.

## 5. Proposed design: one pipeline, three layers

### 5.1 A pure text stage: `preview-text/` (TypeScript, no DOM)

```ts
type PreviewKind = "code" | "diff" | "output" | "prose" | "json";

interface PreviewDoc {
    kind: PreviewKind;
    lang?: string;               // detected from path or content; never from a gutter line
    lines: PreviewLine[];
    truncated?: { hidden: number; from: "head" | "tail" };   // metadata, not text
}
interface PreviewLine {
    number?: number;             // Read gutter, as data
    marker?: "+" | "-" | " " | "@@";   // diff
    text: string;                // tabs already expanded; no control characters
    spans?: StyleSpan[];         // colour from SGR, kept across lines like a terminal
    stream?: "stdout" | "stderr";
}

function toPreviewDoc(tool: ToolNode, opts: { tabWidth: number; indentUnit: number; maxLines: number }): PreviewDoc;
```

The stages run in a fixed order, each a small pure function with its own tests:
1. **Extract:** one `previewSourceOf(tool)` replaces `terminalText`, `stdout ?? content`, `extractFetchResult` and the others.
2. **Decode terminal text:** parse colour codes into spans, with state carried across lines; drop other CSI/OSC; apply `\r` and `\b` like a terminal (keep what's left after the last redraw); drop echoed control replies such as `^[[1;1R` (also fixed at the source, §6 Phase 0).
3. **Split** into lines once.
4. **Structure:** take the Read gutter off into `number` (every format the CLI uses, including right-aligned, left-aligned and `→`); diff markers into `marker`; stdout and stderr kept in order if the source has it.
5. **Expand tabs in JS** with correct tab stops measured from the start of `text`, never from the gutter or marker. One `tabWidth` per kind: 2 for code and diffs (matching `indentUnit`), 8 for output (what the programs that wrote it assumed). After this, `tab-size` in CSS no longer matters, and the same file looks the same in a Read, an Edit, a jekt or the shell log.
6. **Dedent and narrow** (the existing `dedent.ts` logic, unchanged in spirit). It now always runs on spaces only, so the "bail out on tabs" case disappears.
7. **Cap** once, structurally, with the hidden count as metadata.

Code and diffs are highlighted per line from `lines[]`. Diffs use two token streams (old side, new side) so grammar state doesn't bleed across. One Shiki loader is shared and follows the app theme.

### 5.2 One renderer: `<PreviewLines doc mode>`

- One block element per line, in a two-column grid: an optional gutter or marker column, then the text. The gutter is a real column, so it can't affect tab stops or wrapping, and it never gets highlighted as code.
- Shiki tokens become spans inside each line's text block. Diff backgrounds live on the line block, so they always span the line.
- Hidden-line and truncation markers are their own rows, outside the text flow.
- Streaming uses the same component, appending lines. A partial last line is held until it ends or the tool finishes, instead of each chunk being its own block. Streaming and finished views then can't differ.
- Every existing preview (`FilePreview`, `DiffViewer`, `BashOutputViewer`, `TerminalOutput`, `CompactResult` JSON, jekt body, context delivery, shell log, dock log) becomes a thin wrapper choosing a `PreviewKind` and a mode.

### 5.3 One stylesheet mixin, two modes

```scss
@mixin preview-text($mode) {      // $mode: scroll | wrap
    font-family: var(--font-mono);
    line-height: 1.45;
    // Tabs are expanded in JS (preview-text/); this only guards stray ones.
    tab-size: 8;
    @if $mode == scroll { white-space: pre; }
    @else {
        white-space: pre-wrap;
        overflow-wrap: break-word;     // words first; break a token only if it can't fit alone
        // Hanging indent per line block, so wrapped rows are visibly continuations.
        padding-left: 3ch;
        text-indent: -3ch;
    }
}
```

The horizontal scroll lives on the preview box, not an inner element, so its scrollbar is always visible.

**One table decides the mode per kind.** Proposed default:

| Kind | Mode |
|---|---|
| code, diff, output (Bash, Grep, Glob, MCP text) | scroll |
| prose (jekt body, web text, Agent reports in Markdown) | wrap |
| JSON | scroll |

A per-preview "wrap lines" toggle could flip it, with the choice stored per pane. This is the decision the 2026-09-02 analysis (§6.3) recommended and that didn't ship.

All `:global(...)` rules, the dead rules listed in §3.4, and the per-component `white-space` and break rules are removed in favour of this mixin.

## 6. Plan

**Phase 0: quick wins, independent of the redesign** (small PRs; each fixes something visible today)
1. **bashwrap:** stop the `^[[1;1R` echo at the source (`bash_wrap.rs:1149`). Either disable echo before pre-writing the report, or strip that exact reply from the captured output. It also reaches the model.
2. **bashwrap PTY width:** open the PTY wider than 80 columns (200, decided in §8), and set `COLUMNS` to match. Width-aware tools then stop hard-wrapping. Programs that pad tables to the full width will print longer lines, which the scroll mode handles.
3. **Fix the dropped diff and Shiki rules:** replace `:global(.x)` with plain selectors. This restores full-width diff backgrounds and the marker styling. *Moved into Phase 2:* Shiki puts a `"\n"` text node between its `.line` spans, so making `.line` a block under `white-space: pre` adds an empty line between every two lines. Full-width line backgrounds need the one-block-per-line DOM of §5.2.
4. **Apply `tab-size` to the surfaces that miss it** (jekt, context delivery, shell log, dock log), as a stopgap until Phase 1 expands tabs in JS. *Dropped:* Phases 1–2 follow straight on, so the stopgap would be removed again at once.
5. **Correct the stale doc Status lines** listed in §3.6.

**Phase 1: the text stage** (`preview-text/`, pure, fully unit-tested)
- Fixtures taken from real tool output: CLI Read gutters in every format, tab-indented and mixed files, colour codes spanning lines, `\r` progress bars, 80-column-folded output, interleaved stdout/stderr, a token-capped Read.
- Moves the logic from `dedent.ts`, `output-cap.ts`, `terminal-text.ts`, `AnsiLine` and the diff builder into one place, behind `toPreviewDoc`.

**Phase 2: one renderer and the mixin**
- Add `<PreviewLines>` and the `preview-text` mixin.
- Migrate previews one at a time, each behind the same visual check (below): Read/Write, Edit, Bash finished + streaming, string results, JSON, jekt/context/shell/dock.

**Phase 3: delete**
- The old per-component transforms, the dead CSS, the duplicate Shiki loader and the duplicate gutter regex.

## 7. Guarding it: layout tests in a real browser

jsdom can't see any of these bugs, which is why every past fix passed its tests. The repo already has a pattern for measuring in a real browser: `components/MyAgentsList.layout.test.tsx` compiles the real SCSS with `sass`, renders the component, and measures it in headless Chrome.

Add one `PreviewLines.layout.test.tsx` that renders a fixture set at two widths and asserts:
- visual lines per logical line (1 in scroll mode, ≥ 1 with a hanging indent in wrap mode);
- the x position of the first character at each indent level (equal steps from level 0);
- diff line backgrounds spanning the line;
- no control characters in the text;
- the same box size for the streaming and finished renderings of the same output.

That test is the definition of "previews are always clean".

## 8. Decisions

The owner asked for the recommendations (2026-10-08: "do the fixes, use best recommendations"):

1. **Code and command output scroll** instead of soft-wrapping; prose wraps (§5.3).
2. **Tab width:** 8 for command output, 2 for code and diffs.
3. **PTY width for agent Bash commands:** a fixed 200 columns (50 rows). Bash derives `COLUMNS` and `LINES` from it.
