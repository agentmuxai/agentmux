# Spec: Content-first tool previews — WebSearch expanded, no chevron "tree parent", a clean header row

**Date:** 2026-09-26
**Status:** active — §3.2 (header row) implemented in #3871; §3.1, §3.3 and §3.4 (WebSearch content-first) in the second PR; §3.5 and §3.6 (text bodies, content blocks) in the third; §3.7 proposed
**Scope:** agent pane tool previews (`frontend/app/view/agent/`)
**Verified against:** `main` @ `6b5c2b59b`, which includes #3861 (the jekt
height cap)
**Related:** `SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md`,
`SPEC_WEBSEARCH_RICH_VIEW_2026_06_19.md`,
`SPEC_WEBSEARCH_CARD_FULL_CONTENT_AND_STYLING_2026_08_13.md`,
`PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md`,
`SPEC_TOOL_PREVIEW_HEIGHT_THIRD_AND_FOLLOW_LATEST_2026_09_25.md`,
`SPEC_COMPOSER_ACCOUNT_SWITCH_AND_JEKT_HEIGHT_CAP_2026_09_26.md` §3
**Companion report:** `docs/reports/REPORT_TOOL_PREVIEW_DRY_AND_ARCHITECTURE_2026_09_26.md`

---

## 0. TL;DR

The request:
- WebSearch previews expanded, without the collapsible "tree parent" line.
  Show the content itself.
- Remove redundant information from the header row.
- Style the preview.

What's wrong today:

1. **WebSearch result cards never render for Claude Code.** Claude Code returns
   WebSearch results as one prose string. The card extractor only understands
   JSON arrays, so every Claude Code WebSearch falls back to `CompactResult`.
   That renders a `▸ Web search results for query: "…" Links: [{"title":…`
   one-liner you have to click to expand. **That chevron line is the "tree
   parent."**
2. **The panel folds once its row scrolls off the top**, like every tool
   (`ToolBlock.autoExpanded()`). A tool loaded from history is never expanded
   at all.
3. **The header row repeats itself.** A completed WebSearch reads
   **`✓ 🌐 WebSearch solid docs ✓`**, and a running one reads
   **`⏳ 🌐 WebSearch solid docs ⏳ 3s…`**. The parser bakes the icon, tool
   name, and a status glyph into `node.summary`, and `ToolBlock` renders its own
   status glyph (and duration) around it. (Verified by running the real
   `ClaudeCodeStreamParser` on a tool_call/tool_result pair, §2.1.) An MCP call
   reads `✓ 🛠️ mcp__agentmux__WhoAmI  ✓`: the same double status glyph, plus the
   raw `mcp__server__` prefix and a double space.

A survey of the other tools against real transcript payloads (§2) found the
same class of problem in several more places. Most noticeable: **every MCP tool
call (all of the AgentMux App API) renders as a two-column `type | text`
table**, and **every Agent report renders as `▸ 0: {2 keys}`**.

The fix:
- A per-renderer **presentation** flag, so a tool can opt into
  "content-first": always expanded, starts at the top, no chevron.
- A real parser for Claude Code's WebSearch string, with a restyled body.
- A header row built at render time from node fields, not a baked string.
- Unwrapping text content-block arrays.

---

## 1. How a tool preview renders today

```
DocumentRow → ToolBlock   header row: [status glyph] [node.summary] [(duration)] [pill] [live tail]
               └─ .agent-tool-panel     (hidden unless expanded())
                   └─ ToolBlockOverlay → ToolOverlayLog → renderToolResultBody()
                                                            └─ registry-resolved renderer
```

- **Expansion** (`components/ToolBlock.tsx:210-224`):
  `expanded = pinned || autoExpanded() || userHolding()`. `autoExpanded()` is
  true while `running`/`pending_approval`, and after completion while the tool
  is in `documentState.expandedTools` (set on the live active→inactive
  transition, cleared when the row scrolls off the top). Clicking the row
  toggles `pinned`.
- **Header text**: `node.summary` is the string
  `` `${icon} ${tool} ${detail}${duration} ${statusIcon}` `` built once by
  `ClaudeCodeStreamParser.generateToolSummary()` (`stream-parser.ts:823-837`),
  on the call and again on the result. `ToolBlock` separately renders
  `STATUS_ICON[status]` before it and `(duration)` after it.
- **Renderer resolution** (`components/tool-renderers/registry.ts`): the
  highest-priority matching entry wins. Built-ins (priority 0, by coarse kind)
  live in `ToolOverlayLog.tsx:790-798`. Also registered: `web:search` (10),
  `web:fetch` (10), `dispatch:card` (10), `shape:record-table` (-1), and the
  catch-all `builtin:default` (-∞, `CompactResult`).
- **`CompactResult`** (`components/CompactResult.tsx`) is the chevron: a
  one-line summary (≤120 chars) with `▸`, which a click expands to the full body.
  It starts collapsed for every tool but Glob.
- **Scroll behavior** (`ToolOverlayLog.tsx:234-236`): every preview except
  Read/Write/Edit starts in **FOLLOWING** mode, pinned to the bottom.
- **Height**: the panel's log box is capped at `$transcript-preview-max-height`
  (`calc(50vh / 3)`). Since #3861 the same SCSS variable caps an expanded jekt
  body, so the two move together.
- **Virtualizer estimate** (`virtualization/renderers.ts:103-105`):
  `estimateTool` returns `TOOL_EXPANDED_PX` (200) only for **pinned** tools and
  32 px otherwise.
- **Result shapes** (`providers/claude-translator.ts:388-391`): a string
  `tool_result.content` becomes `{ content: "<string>" }`, and a content-block
  array is passed through as-is.

## 2. Findings

### 2.1 Header row (all tools)

This is the output of the real parser (a throwaway vitest run on 2026-09-26,
since deleted) for a WebSearch and an MCP call, Claude provider, without
`duration` (Claude `tool_result`s carry none):

| Event | `node.summary` | Row as rendered (`ToolBlock` adds the leading glyph) |
|-------|----------------|-------------------------------------------------------|
| WebSearch call | `🌐 WebSearch solid docs ⏳` | `⏳ 🌐 WebSearch solid docs ⏳ 3s…` |
| WebSearch result | `🌐 WebSearch solid docs ✓` | `✓ 🌐 WebSearch solid docs ✓` |
| MCP call | `🛠️ mcp__agentmux__WhoAmI  ⏳` | `⏳ 🛠️ mcp__agentmux__WhoAmI  ⏳ 3s…` |
| MCP result | `🛠️ mcp__agentmux__WhoAmI  ✓` | `✓ 🛠️ mcp__agentmux__WhoAmI  ✓` |

The redundancies:
- **R1:** the status glyph appears twice.
- **R2:** for providers that report `duration`, the duration also appears twice
  (the `generateToolSummary` suffix plus `ToolBlock`'s `.agent-tool-duration`).
- **R3:** the tool icon and tool name say the same thing for tools with a
  distinctive icon (🌐 + "WebSearch").
- **R4:** the `mcp__<server>__` prefix is noise.
- **R5:** an empty `detail` leaves a double space.
- **R6:** the WebSearch body header (`9 results · "solid docs"`) repeats the
  query that's already in the row.

Because the summary is frozen into the node, the glyph also goes **stale**.
The reducer forces orphaned tools to `canceled` with `{ ...n, status: "canceled" }`
(`store/agent-document/reducer.ts:186`, `:199`) and never regenerates
`summary`, so a cancelled tool's row reads `⏹ 🌐 WebSearch … ⏳`: two
contradictory status glyphs.

### 2.2 Result bodies: real payload shapes vs. what renders

Shapes are taken from real Claude Code transcripts on this machine (the last 60
`.jsonl` files under `~/.claude/projects`, plus a WebSearch captured live on
2026-09-26).

| # | Tool | Real `tool_result.content` | Renders as today | Problem |
|---|------|----------------------------|------------------|---------|
| F1 | **WebSearch** (reported) | `str`: header + `Links: [...]` + markdown summary + `REMINDER:` footer | `CompactResult`: `▸ Web search results for query: "…" Links: [{"title":"…` | Cards never render; content behind the chevron; raw JSON in the summary line |
| F2 | **mcp\_\_\*** (all App API tools, all MCP) | `list[{type:"text", text}]` | `RecordTable` with columns **`type` \| `text`** | A table where plain text belongs |
| F3 | **Agent** (no live dispatch match, which is always the case on the Agent History tab and for replays) | `list[{type:"text", text}]` | `renderAgent` → `CompactResult` → **`▸ 0: {2 keys}`**, expanding to JSON of the blocks | The report is unreadable |
| F4 | **Grep** | `str` → `{content}` | `Pattern: <pattern>` + `▸ N result lines` | Pattern duplicates the row; matches behind the chevron |
| F5 | Generic text tools (TaskOutput, TaskStop, ListAgents, ScheduleWakeup, Skill, …) | `str` → `{content}` | `▸ <first 120 chars>` | Multi-line results hidden behind the chevron |
| F6 | ToolSearch | `list[{type:"tool_reference", tool_name}]` | `RecordTable` `type` \| `tool_name` | Minor: a redundant `type` column |
| F7 | TodoWrite | *not in the sampled transcripts; verify.* The list is in `params.todos` | `CompactResult` of the boilerplate result | The list is never shown |
| F8 | ExitPlanMode | *not sampled; verify.* The plan is in `params.plan` | `CompactResult` of the result string | The plan is never shown |

**Not affected:** Read, Write, Edit, Bash, WebFetch, Glob, and answered
AskUserQuestion.

**Codex** maps `web_search` → `WebSearch` with the raw item as its result
(`providers/codex-translator.ts:127-135`). That shape wasn't sampled; verify it
before relying on it.

#### F1 detail: the real WebSearch string

```
Web search results for query: "SolidJS createResource documentation"

Links: [{"title":"Fetching data - SolidJS Documentation","url":"https://docs.solidjs.com/guides/fetching-data"}, …]

<markdown summary written by the search sub-model: headings, bullets, URLs>


REMINDER: You MUST include the sources above in your response to the user using markdown hyperlinks.
```

- `Links:` carries only `title` + `url`, with no snippet or date. The readable
  content is the summary.
- The `REMINDER:` line is an instruction to the model, not content for the user.
- A result can contain more than one `Links:` line (multi-round searches).

The August card spec (`…_2026_08_13.md`) assumes the cards render. For Claude
Code they never have. Its data-attribute and name-color fixes are still
relevant; its snippet un-clamping is moot for Claude Code, which sends no
snippets.

## 3. Design

### 3.1 Content-first tools (implemented in the second PR)

`components/tool-presentation.ts` holds a plain name table (`WebSearch`,
`web_search`) and two predicates:

- `isContentFirstTool(node)`: a content-first tool that finished with a result
  (`success`, or `failed` with its error). While running it's auto-expanded
  like any tool; `denied`/`canceled` have nothing to show.
- `startsAtTop(node)`: by name only, because the preview box is mounted while
  the tool is still running and must not start pinned to the bottom.

*(Changed from the draft, which proposed a `presentation` field on the renderer
registry.)* The virtualizer (`expansion-source.ts`, `renderers.ts`) needs the
same answer. The registry is only populated as a side effect of importing
`ToolOverlayLog`, which those modules and their tests never do, so a registry
field would silently read "panel" there. This is the fragility described in the
companion report's C5. The table moves into the report's per-tool descriptor
(A1) once that exists.

`ToolBlock`:

```ts
const contentFirst = () => isContentFirstTool(props.node);
const expanded = () => contentFirst()
    ? !props.userCollapsed                    // documentState.collapsedNodes
    : props.pinned || autoExpanded() || userHolding();
```

A header click (and the row's `e` key in `DocumentRow`) toggles the collapse for
a content-first tool, and the pin for everything else.

- User collapse uses the existing `documentState.collapsedNodes` set, which
  agent messages and jekts already use (both are expanded by default and
  collapse on click). It doesn't reuse `pinnedNodes`, whose meaning would be
  the opposite.
- **This applies to history too.** A content-first tool from a reopened
  transcript is expanded.
- **Height:** the existing `$transcript-preview-max-height` cap stays, the same
  value #3861 gave the jekt body. That makes content-first tool previews and
  jekts the same height by construction, and resolves the draft's Q2.
- **Start at the top:** `ToolOverlayLog`'s `initialFollow()` treats a
  content-first node like the Read/Write/Edit `DOCUMENT_KINDS`: DETACHED at
  the top. Otherwise a capped search preview opens scrolled to its last line.
- **Virtualizer estimate:** `estimateTool` returns a capped expanded estimate
  for content-first nodes that aren't in `collapsedNodes`, mirroring
  `estimateExpandedJekt` from #3861. Otherwise every history search is first
  laid out at 32 px and then jumps.

### 3.2 Header row: built at render time, nothing twice (R1–R5)

Stop rendering `node.summary` in `ToolBlock`. Compose the row from node fields
instead:

```
[status glyph] [tool icon] [label] [detail] [pill] [duration | live tail]
```

| Part | Source | Rule |
|------|--------|------|
| status glyph | `STATUS_ICON[node.status]` | Once only (R1). It always reflects the **current** status, never a baked one |
| tool icon | `TOOL_ICONS[toolName] ?? TOOL_ICONS[tool]` | |
| label | the tool's display label | **Omitted for the web tools** (WebSearch/WebFetch) when they have a detail: 🌐 plus a query or a host/path can only be a web tool (R3). Kept for every other tool; 🔧 or 📁 alone don't read as "Bash" or "Glob". MCP: `server · Tool` (e.g. `agentmux · WhoAmI`), prefix stripped (R4) |
| detail | `extractToolDetail(tool, params)` | Already exported and shared with the peek tooltip. Omitted when empty; no stray spaces (R5) |
| pill | `resultPill()` | Unchanged mechanism. New WebSearch pill: `N sources` (moved out of the body, R6) |
| duration | `node.duration` | Once only (R2) |

- Keep **generating** `node.summary`. Other consumers read it (activity/export
  paths, older persisted nodes, tests). But make `generateToolSummary` drop the
  status glyph and duration, so every consumer stops inheriting R1/R2.
  `ToolBlock` falls back to `node.summary` only for nodes with no `tool`/`params`
  (for example synthetic nodes such as the `AskUserQuestion` "❓ Waiting for your
  answer" node).
- The WebSearch row becomes: `✓ 🌐 SolidJS createResource documentation  [9 sources]`.
- The MCP row becomes: `✓ 🛠️ agentmux · WhoAmI`.
- The detail keeps the existing `.agent-tool-name` ellipsis rule. The full text
  stays in the peek tooltip.

### 3.3 WebSearch: parse Claude Code's string (F1)

New pure function in `tool-renderers/search-results.ts`:

```ts
export interface ParsedWebSearch {
    query?: string;
    links: SearchResultItem[];   // title + url (+ index); no snippet available
    summary?: string;            // markdown, REMINDER footer stripped
}
export function parseWebSearchText(text: string): ParsedWebSearch | null;
```

- `query` comes from `^Web search results for query: "(.*)"$` on the first line.
- Every line starting with `Links: ` is parsed as a JSON array (skip a line if
  its parse fails); the results are concatenated and de-duplicated by URL.
- `summary` is every other line, with a trailing
  `REMINDER: You MUST include the sources…` paragraph removed. It is trimmed,
  and `undefined` if it ends up empty.
- It returns `null` unless at least one link **or** a non-empty summary was
  found.

`extractSearchResults` tries this first for a string or `{content: string}`. The
existing array shapes keep working. The renderer is registered with
`presentation: "content"`.

### 3.4 WebSearch body styling

The body sits in a box capped at about 13 lines, so the design puts the
**answer first** and keeps the sources compact. A card per link (today's
design) would use most of the capped height on 9 bordered title-only cards
before the summary even starts.

```
┌ ✓ 🌐 SolidJS createResource documentation                 [9 sources] ┐
│                                                                        │
│  ## Official docs                                                      │  ← summary (Markdown)
│  - **API reference:** docs.solidjs.com/reference/…                     │
│  createResource is a specialized signal designed for …                 │
│  …                                                                     │
│  ─────────────────────────────────────────────────────────────────     │
│  SOURCES                                                               │  ← sources strip
│  [◉ docs.solidjs.com ×2] [◉ thisdot.co] [◉ solidjs.com ×4] [▶ youtube] │
│  [◉ medium.com] [◉ raresportan.com]                                    │
└────────────────────────────────────────────────────────────────────────┘
```

- **Summary:** `<Markdown text={summary} scrollable={false} />`, the same
  pattern and reason as `renderRead`'s markdown branch
  (`ToolOverlayLog.tsx:654-661`). It uses the pane's markdown typography one
  step smaller (the transcript body size, not the tool-log monospace), so it
  reads as prose, not as tool output. Headings render as bold body-size
  labels without rules: at document size, one `## Official docs` took a large
  share of the capped box (seen live). Verify that link clicks inside it open in
  the system browser like the cards do.
- **Sources strip:** a wrapping row of chips below a hairline divider
  (`border-top: 1px solid var(--border-color)`) and a small-caps `SOURCES` label
  (`font-size: 10px; letter-spacing; opacity: .6`).
  - Each chip shows the favicon (14 px; existing Google S2 URL and `onError`
    fallback) and the **domain**.
  - Links on the same domain group into one chip with a `×N` count. A click on
    a grouped chip lists its links inline under the strip (one line each: favicon,
    title, domain). It doesn't open a popover: a popover inside a virtualized row
    is clipped by the next row (see `PeekOverlay.tsx`).
  - Chip style: `border: 1px solid var(--border-color)`, `border-radius: 0`
    (matching today's cards), `padding: 1px 6px`, `font-size: 11px`, and
    `color: var(--secondary-text-color)`. Hover sets
    `color: var(--main-text-color)` plus the existing card hover background.
  - The `title` attribute is the page title + URL.
  - Click and keyboard (Enter/Space) call `openExternal`, as today.
- **No-summary case** (only links parsed): fall back to a compact list
  instead of chips. There's one line per link:
  `favicon · title · domain (dim)`, single-line with an ellipsis, and the whole
  line is clickable.
- **Array-shaped results that do have snippets** (other providers/tools) keep
  today's cards. The August spec's un-clamping still applies to them.
- The count moves to the header pill (§3.2, R6), and the body header line is
  removed.

### 3.5 No chevron on a text body (F4, F5, and the fallbacks)

When the result has a **string body** (`terminalText(result) != null`),
`CompactResult` renders the body directly in a `TerminalOutput` (already
tail-capped), with no summary line and no chevron. The panel is the
expand/collapse control. `▸` plus JSON stays only for genuinely **structured**
results.

Grep (F4):
- Drop the `Pattern:` line; the row shows it.
- The body is the match lines.
- The existing Grep pill falls back to counting non-empty content lines when
  there's no `matches` array.

### 3.6 Unwrap text content-block arrays (F2, F3)

In `claude-translator.ts:buildToolResults`, when `block.content` is an array
whose every element is `{type:"text", text:string}`, normalize it to
`{ content: <texts joined with "\n\n"> }`. Arrays with any non-text block
(`image` from `CaptureWindow`, `tool_reference`) pass through unchanged.

Verify that history replay (`parseHistoryLines`) goes through the same
translator. If not, normalize where both paths meet.

Defense in depth: `extractRecords` rejects arrays whose rows are all
`{type, text}`.

Effects:
- **MCP (F2):** a text body.
- **Agent body:** no longer repeats the description the row header already
  shows (seen live). A Workflow body shows its description only when the header
  shows a different title.
- **Agent (F3):** `renderAgent` renders `result.content` as
  `<Markdown scrollable={false}>`.

### 3.7 Params-carried content (F7, F8, F6) — lower priority

- **TodoWrite:** a checklist from `params.todos`, with ☐ / ◐ / ☑ glyphs.
  Content-first.
- **ExitPlanMode:** `<Markdown text={params.plan}>`. Content-first.
- **ToolSearch:** the loaded tool names as chips (the same chip style as §3.4),
  not a table.

Verify the F7 and F8 shapes with a live call first.

## 4. Which tools get `presentation: "content"`

| Tool | Mode | Why |
|------|------|-----|
| WebSearch / web_search | **content** | The request |
| TodoWrite | content | The list is the point, and it's short |
| ExitPlanMode | content | The plan is the point |
| Agent (report, no dispatch match) | panel | Can be long; the chevron goes (§3.6) but it still folds. Q2 |
| Everything else | panel | Unchanged folding; no chevron for text bodies (§3.5); the header fix applies to all |

## 5. Phasing

1. **P1, the reported issues:**
   - §3.1 (the presentation flag, `ToolBlock`, start-at-top, and the estimate)
   - §3.2 (the header row, which affects every tool)
   - §3.3 and §3.4 (the WebSearch parser and styling)
2. **P2, wrong-shape renders:** §3.6 (content-block unwrap for MCP and Agent)
   and §3.5 (no chevron for text bodies, Grep).
3. **P3:** §3.7.

Each phase is one PR. The companion report lists refactors that make P1–P3
smaller if they land first (notably one per-tool descriptor instead of five
parallel switches); none of them block P1.

## 6. Tests

Use real payloads as fixtures. Commit the WebSearch string captured on
2026-09-26.

- `search-results.test.ts`:
  - `parseWebSearchText` returns the query, 9 links, and the summary, with the
    REMINDER stripped.
  - Multiple `Links:` lines are merged.
  - A malformed `Links:` line is skipped.
  - A non-search string returns `null`.
  - `{content: string}` is accepted.
- `SearchResults.test.tsx`:
  - The real fixture renders the summary markdown, then a sources strip with
    one chip per domain and `×N` for repeated domains.
  - There's no `.agent-tool-compact-summary`, no REMINDER text, and no body
    header count.
  - Links only → compact list.
  - An array with snippets → cards, as today.
- `registry.test.ts`: `resolveToolPresentation` returns `"content"` for
  WebSearch and `"panel"` by default.
- `ToolBlock.test.tsx`:
  - The row has exactly one status glyph for running and success.
  - A distinctive-icon tool shows no label.
  - MCP shows `server · Tool`.
  - An empty detail leaves no double space.
  - The duration appears once.
  - A completed content-first tool rendered fresh (history) is expanded; a
    click collapses it and a second click re-expands; a panel-mode tool is
    unchanged.
- `stream-parser` tests: `generateToolSummary` output has no status glyph and
  no duration.
- `ToolOverlayLog.test.tsx`: a content-first node starts DETACHED at the top.
- `renderers.test.ts`: `estimateTool` gives a capped expanded estimate for
  content-first tools and 32 px once they're user-collapsed.
- `claude-translator` tests:
  - An all-text block array → `{content}`.
  - An `image` or `tool_reference` array passes through unchanged.
- `record-table.test.ts`: a `[{type,text}]` array is not a table.
- `CompactResult.test.tsx`: a string body has no chevron; a structured object
  keeps it.

## 7. Acceptance criteria

- A Claude Code WebSearch, both live and in a reopened transcript:
  - Its row reads `✓ 🌐 <query> [N sources]`, with exactly one status glyph.
  - The body shows the summary first, starting at the top, then the sources
    strip.
  - It stays expanded after scrolling off and back.
  - There's no `▸` line and no REMINDER text.
  - A header click collapses and re-expands it.
- No tool row anywhere shows the status glyph or duration twice. MCP rows read
  `server · Tool`.
- `mcp__agentmux__*` calls show text, not a `type | text` table.
- Agent calls on the Agent History tab show their report as markdown.
- Grep shows its matching lines as soon as the panel is open.
- Scrolling a transcript with many searches doesn't jump as rows mount.
- Read, Write, Edit, Bash, WebFetch, and Glob are unchanged. Verify this live,
  with screenshots in the PR.

## 8. Open questions

- **Q1. Label omission (R3):** implemented for the web tools only (§3.2).
  Should other tools with their own icon drop the name too? WebSearch and
  WebFetch share 🌐, so their rows are told apart
  only by the detail (a query vs. a host/path) and the pill. Is that enough, or
  should WebFetch get its own icon (the August spec's optional §3.4)?
- **Q2. Agent reports:** content-first (always expanded) or panel? They're often
  long.
- **Q3. More content-first tools?** For example WebFetch, which is already rich
  but folds on scroll-off.

*(Resolved from the first draft: the header row stays but is de-duplicated
(§3.2); the height uses the shared `$transcript-preview-max-height` cap from
#3861 (§3.1).)*
