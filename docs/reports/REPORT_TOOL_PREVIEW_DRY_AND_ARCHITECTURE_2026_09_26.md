# Report: DRY and architecture cleanup — agent-pane tool previews and message blocks

**Date:** 2026-09-26
**Verified against:** `main` @ `6b5c2b59b`
**Scope:**
- Tool previews: `frontend/app/view/agent/components/ToolBlock.tsx`,
  `ToolBlockOverlay.tsx`, `ToolOverlayLog.tsx`, `CompactResult.tsx`, and
  `tool-renderers/*`.
- The per-tool helpers in `stream-parser.ts`, `useAgentStream.ts`, and
  `activity/tool-adapter.ts`.
- The collapsible message blocks (`JektBubble.tsx`, `AgentMessageBlock.tsx`),
  including #3861's jekt height cap.
- Row-height estimates (`virtualization/renderers.ts`).

**Companion spec:** `docs/specs/SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md`.
Most items below either make that spec's phases smaller or remove the root
cause of a bug it found.

Every item cites where it was observed. Sizes are rough (S ≈ under a day,
M ≈ 1–2 days, L ≈ multi-PR).

---

## 1. Summary

The tool-preview code has **one good abstraction and several parallel ones
beside it**. The renderer registry (June) routes result *bodies* by name or
shape. Everything else a tool needs is decided somewhere else, by its own
`switch` or constant list, each keyed on a slightly different name:
- its icon and label
- the detail string for its header
- its result pill
- its one-line compact summary
- whether its preview starts at the top or follows the latest output
- whether it's expanded by default
- its estimated height

Adding or changing one tool means editing five to seven files, and they drift.
The header-row bugs in the companion spec (a status glyph shown twice, a stale
`⏳` on a cancelled tool) come straight from that drift.

The top three, in order:

1. **A single `ToolDescriptor` registry** (A1), extending the existing renderer
   registry. It absorbs A2–A5 and makes the spec's "content-first" flag one
   field instead of a new cross-cutting concept.
2. **Stop baking presentation into `node.summary`** (A2). It's the root cause
   of R1, R2, and the stale glyph.
3. **One disclosure model for all collapsible transcript nodes** (B1). Tools,
   jekts, agent messages, user messages, and shells use three different state
   sets with opposite defaults.

## 2. Findings

### A. Per-tool knowledge is spread across parallel switches

**A1. There's no single place that describes a tool. (L, highest leverage)**

Per-tool decisions today, one per location:

| Decision | Where | Keyed on |
|----------|-------|----------|
| Body renderer | `tool-renderers/registry.ts` + registrations in `ToolOverlayLog.tsx:790-798` and 4 renderer modules | coarse `tool` / `toolName` / result shape |
| Icon | `TOOL_ICONS` (`types.ts:901-916`) | mixed: coarse kinds **and** raw names (`WebSearch`, `web_search`, …) |
| Header detail | `extractToolDetail` (`stream-parser.ts:136-166`) | coarse kind *or* raw name, depending on the case |
| Activity-row argument | `extractToolArg` (`useAgentStream.ts:79-102`) | raw lowercase/provider names (`read_file`, `str_replace_editor`, …): **a second detail extractor** |
| Result pill | `resultPill()` switch (`ToolBlock.tsx:229-301`) | coarse kind |
| Compact one-liner | `summarize()` switch (`CompactResult.tsx:28-94`) | coarse kind |
| Default-expanded compact body | `createSignal(tool === "Glob")` (`CompactResult.tsx:111`) | coarse kind |
| Starts at top vs. follows | `DOCUMENT_KINDS` (`ToolOverlayLog.tsx:64`) | coarse kind |
| Kind normalization | `knownTools` (`stream-parser.ts:844`) | raw name → coarse kind |
| Row-height estimate | `estimateTool` (`virtualization/renderers.ts:103`) | none (pinned or not) |

**Proposal:** extend `ToolRendererEntry` into a `ToolDescriptor`, with every
field optional except `match`:

```ts
interface ToolDescriptor {
    label: string;                 // registry de-dup key (exists today)
    priority: number;
    match: ToolMatcher;
    icon?: string;
    displayName?: (node) => string;          // e.g. MCP "server · Tool"
    detail?: (params) => string;             // replaces extractToolDetail + extractToolArg
    pill?: (node) => Pill | null;            // replaces resultPill's switch
    render?: ToolRenderer;                   // today's field
    presentation?: "panel" | "content";      // companion spec §3.1
    scroll?: "follow" | "top";               // replaces DOCUMENT_KINDS
    estimate?: (node, state) => number;      // replaces estimateTool's constant
}
```

Resolve each field independently: take the highest-priority matching descriptor
that **defines** that field. A `mcp__` prefix descriptor can then supply
`displayName` without also having to supply `render`.

`extractToolDetail`, `extractToolArg`, `TOOL_ICONS`, `resultPill`,
`DOCUMENT_KINDS`, and the Glob special case become per-descriptor entries. The
peek tooltip, the Activity Dock (`activity/tool-adapter.ts:236` already calls
`extractToolDetail`), `btw.ts`, and the header all read the same resolver.

**Migration:** keep the old exports as thin wrappers over the resolver for one
release, then delete them.

**A2. `node.summary` is presentation baked into data. (M, fixes real bugs)**

`generateToolSummary` (`stream-parser.ts:823-837`) freezes
`icon + tool + detail + duration + statusGlyph` into the node when it's parsed.
`ToolBlock` then renders its own status glyph and duration around it. Verified
output: `✓ 🌐 WebSearch solid docs ✓`. The reducer's orphan-cancel paths
(`store/agent-document/reducer.ts:186`, `:199`) spread the node and
never regenerate `summary`, so the row reads `⏹ … ⏳`.

The other readers are `btw.ts:61` (a text line for the btw overlay) and the
AskUserQuestion reducer guard (`reducer.ts:984`, which keeps the existing
summary on purpose).

**Proposal:** the header is composed at render time from `status`, the A1
descriptor, `params`, and `duration`. `summary` survives only as a plain-text
fallback without a status glyph or duration (for `btw.ts` and synthetic nodes
like `❓ Waiting for your answer`). Longer term, `btw.ts` can call the same
header-text resolver and `summary` can become optional.

**A3. Three status maps for one enum. (S)**

- `STATUS_ICON` (`ToolBlock.tsx:121`) covers all 7 statuses.
- `STATUS_ICONS` (`types.ts:912`) covers 4, which is why a baked summary for
  `pending_approval`, `awaiting_answer`, or `denied` gets **no** glyph while the
  row still gets one.
- `STATUS_LABEL` (`ToolBlockOverlay.tsx:41`) is the text labels.
- `activity/tool-adapter.ts:220` maps the same statuses to activity states.

**Proposal:** one `TOOL_STATUS` table in `types.ts`
(`{ icon, label, activity, isActive, isFailTerminal }`), which also absorbs
`isActive` (`ToolBlock.tsx:177`) and `isFailTerminal` (`ToolBlock.tsx:206`).
(`commands/global/tools.ts:21-35` has maps with the same names, but they
describe the tool-install catalog. They're unrelated and should stay separate;
noted only so nobody merges them by mistake.)

**A4. Two detail extractors that disagree. (S; folded into A1)**

`extractToolDetail` knows WebSearch, WebFetch, Agent, and Workflow but not
`read_file` or `str_replace_editor`. `extractToolArg` is the other way round,
and falls back to trying `file_path` / `path` / `command` / `query` / `pattern`
in turn. The header and the activity row can therefore show different text for
the same Codex call.

**Proposal:** one function, which becomes the A1 `detail` field: explicit
cases, then `extractToolArg`'s generic key fallback.

**A5. Kind normalization loses what the renderers need. (S)**

`normalizeToolName` title-cases the raw name and checks it against
`knownTools`. Everything else becomes `"Other"`, and the raw name is kept in
`toolName`. That part is correct. But matchers and switches pick between the
two inconsistently:
- `byKind` / the switches use `tool`.
- `byName` uses `toolName ?? tool`.
- `TOOL_ICONS` is looked up by the **raw** name in `generateToolSummary`,
  whereas other paths look it up by kind.

**Proposal:** once A1 lands, the rule is simple. Descriptors match on
`toolNameOf(node)`, and `tool` is used only for the handful of coarse-kind
built-ins.

### B. Collapsible transcript nodes

**B1. Three disclosure mechanisms with opposite defaults. (M)**

| Node | Default | State used | Toggle semantics |
|------|---------|------------|------------------|
| tool | collapsed (except while active or held) | `pinnedNodes` + `expandedTools` + a local `userHolding` signal | pin means "force open" |
| agent_message, jekt_message | expanded | `collapsedNodes` | means "force closed" |
| user_message, shell | (own rules) | `pinnedNodes` | pin |

Source: `DocumentRow.tsx:235-271`, `state.ts:103-105`, and `types.ts:814-830`.

The companion spec's content-first tools add a fourth case: a tool that's
expanded by default. `estimateTool` reads only `pinnedNodes`, so it already
mis-estimates **held-open** tools (`expandedTools`) today, and would mis-estimate
content-first ones too.

**Proposal:** one `disclosure(node, state) → "open" | "closed"`, computed from:
- a per-kind (or per-A1-descriptor) **default**
- an **override set**, one set whose member means "the user flipped the default"
- the transient **hold** (`expandedTools`, `userHolding`)

Both `DocumentRow` and the row-height estimators call it, so an estimate can
never disagree with what renders.

**B2. `JektBubble` and `AgentMessageBlock` are the same component twice. (S)**

Both components:
- take `{ node, collapsed, onToggle }`
- render a clickable header with `▸`/`▾` and an icon or summary span
- show the body under `<Show when={!props.collapsed}>`
- attach the same `useNodePeek` pattern

(`JektBubble.tsx:32-104`, `AgentMessageBlock.tsx:20-66`.) Their chevron CSS is
identical (`_document-nodes.scss:928-931` and `:997-1000`).

**Proposal:** a `CollapsibleMessage` shell (header slot, body slot, chevron,
peek), with both components as thin wrappers supplying their header content,
body, and metadata. #3861's body cap then lives in one place.

**B3. Two strategies for the "capped, scrollable preview box". (M)**

Since #3861 the jekt body and the tool log share `$transcript-preview-max-height`
(good; `_document-nodes.scss:3-14`), but each box behaves differently:
- The **tool box** sets `overscroll-behavior: contain` and hand-forwards the
  wheel at its edges in JS (`ToolOverlayLog.tsx:162-186`). It also has its own
  follow/detach state machine (`ToolOverlayLog.tsx:234-363`).
- The **jekt body** relies on native scroll chaining (the #3861 comment says so
  explicitly) and has no follow logic.
- The jekt raw-payload `<pre>` is a third capped box, also native.

These can be deliberate differences: a streaming log needs follow, a static
message doesn't. But then the justification for `contain` plus the JS hand-off
on the tool box should be rechecked, because the jekt box shows native chaining
works in this CEF build.

**Proposal:** a `PreviewBox` component (or a `use:previewBox` directive) that
owns the cap, the chaining policy, and an optional `follow` mode, used by the
tool log, the jekt body, the jekt raw payload, and the content-first bodies in
the companion spec. At minimum, write down in one place *why* the tool box needs
`contain` and the jekt box doesn't.

**B4. Height estimates duplicate CSS values by hand. (S)**

- `JEKT_EXPANDED_MAX_ESTIMATE_PX = 290` and `TOOL_EXPANDED_PX = 200`
  (`virtualization/renderers.ts:83-84, 119`) both approximate
  `calc(50vh / 3)` plus chrome, at a fixed 1400 px window.
- Neither follows the real `vh`, and they disagree with each other by 90 px for
  the "same" cap.

**Proposal:** one `previewCapPx()` helper (`window.innerHeight * 0.5 / 3`, cached
on resize) plus per-kind chrome constants. That's used by `estimateExpandedJekt`,
`estimateTool`, and the companion spec's content-first estimate.

### C. Tool-renderer internals

**C1. Duplicated URL helpers. (S)** `prettyUrl`, `hostname`, and `openUrl` are
defined identically in `SearchResults.tsx:27-52` and `WebFetchResult.tsx:29-55`.
Move them to `tool-renderers/url.ts`. The companion spec's sources strip and a
favicon helper (`faviconSrc`, with its `onError` fallback) belong there too.

**C2. Duplicated `str()` coercion helper. (S)** It's in both
`search-results.ts:25` and `web-fetch-result.ts:21`, next to `num`/`bool`.
Move them to `tool-renderers/coerce.ts`.

**C3. Every renderer re-derives "the result's text". (M)** Four places each
decide on their own where a result's string lives:
- `terminalText` (`terminal-text.ts`): string, `stdout`/`stderr`, `output`,
  `content`.
- `findResultArray` (`search-results.ts:40-56`): an array, a JSON string, or
  under one of 6 keys.
- `extractFetchResult` (`web-fetch-result.ts`): string, `content`, `body`,
  `text`, `html`, `data`.
- `CompactResult.summarize`: string, `content`, `output`.

On top of that, `claude-translator.buildToolResults` wraps strings as
`{content}` but passes content-block arrays through raw. That's the root of the
companion spec's F2/F3 (the MCP `type | text` table, Agent's `0: {2 keys}`).

**Proposal:** normalize once, at the translator boundary: string → `{content}`;
an all-text block array → `{content: joined}`. Then add one
`resultText(node): string | null` that every renderer uses.

**C4. Built-in renderers live in `ToolOverlayLog.tsx`, with a circular import.
(S)** `ToolOverlayLog.tsx` (804 lines) is the scroll/follow/FLIP container
**and** hosts `renderEdit`/`renderBash`/`renderRead`/`renderWrite`/`renderSearch`/
`renderAgent`/`renderTask`/`renderWorkflow` plus their registrations.
`DispatchCard.tsx:20` imports `renderAgent`/`renderTask`/`renderWorkflow` back
from `ToolOverlayLog`, while `ToolOverlayLog` side-effect-imports
`DispatchCard`: a module cycle that works today only because of evaluation
order.

**Proposal:** move the built-ins to `tool-renderers/builtins.tsx`, and let
`DispatchCard` import them from there.

**C5. Registration by side-effect import. (S)** `ToolOverlayLog.tsx:40-43`
imports four modules purely for their `registerToolRenderer` calls. The winner
among equal priorities depends on import order (`registry.ts` "ties break by
registration order"), and a new renderer is silently dead until someone adds its
import.

**Proposal:** a single `tool-renderers/index.ts` that exports an explicit
ordered `registerBuiltinToolRenderers()`, called once. Tests already have
`resolveFrom(list, node)` for pure resolution.

**C6. `renderRead` and `renderWrite` share a pipeline. (S)** Both do: cap →
dedent/format → `isMarkdown` (`endsWith(".md") || endsWith(".mdx")`, at
`ToolOverlayLog.tsx:622` and `:698`) → `Markdown` or `HighlightedCode` → a
hidden-lines marker. They differ only in the gutter formatter and the header
row. Extract `FilePreview({ path, text, format })`. Keep the documented reason
why Write must not use the gutter-aware formatter
(`ToolOverlayLog.tsx:684-692`) as the `format` argument's doc.

**C7. Truncation code in `CompactResult` is repeated five times. (S)**
`summarize()` repeats `trimmed.length > N ? trimmed.slice(0, N) + "..." : trimmed`
with N = 120 or 150 (`CompactResult.tsx:30-85`), using `...`, while the rest of
the pane uses `…`. Use one `truncate(s, n)` in `output-cap.ts`, next to
`capChars`. (Most of `summarize()` goes away anyway if the companion spec §3.5
drops the one-liner for string bodies.)

**C8. `CompactResult` destructures its props. (S, latent bug)**
`CompactResult.tsx:110` is
`({ tool, params, result }: CompactResultProps) =>`. The codebase has documented
repeatedly (`ToolBlock.tsx:30-38`, `ToolOverlayLog.tsx:567-575`, `DocumentRow`)
that destructuring Solid props freezes them. It's harmless today only because
every caller re-creates `CompactResult` when the result changes. A caller that
kept it mounted across a result update (a streaming result, or a slot reuse)
would show stale output. Switch it to `props.x`.

### D. Smaller items

- **D1.** The chevron glyph and its styling appear three times
  (`.agent-tool-compact-chevron`, `.agent-message-chevron`,
  `.agent-jekt-chevron`), with different sizes (9 px vs. 10 px). A shared
  `%disclosure-chevron` placeholder or mixin would fix that. Moot for the tool
  one if the companion spec §3.5 removes it.
- **D2.** `ToolBlockOverlay`'s header (`ToolBlockOverlay.tsx:52-66`) exists
  only to show a status label for failed / denied / canceled / awaiting-approval,
  and is hidden with inline `style.display` otherwise. With A3's single status
  table, this can be a status-class rule on the row. The overlay then reduces to
  `ToolOverlayLog`, and the file can go.
- **D3.** A comment in `ToolBlock.tsx` (lines 502–514) still describes an
  `overlay` panel mode that was removed. `panelMode()` returns only
  `hidden`/`flow`. Delete the stale lines.

## 3. Suggested order

| Step | Items | Why this order |
|------|-------|----------------|
| 1 | C1, C2, C4, C5, C8, D3 | Pure moves and fixes, no behavior change. Unblocks everything else and removes the import cycle |
| 2 | A3, A2 | Fixes the header bugs (a status glyph twice, a stale `⏳`). This is the companion spec's §3.2, done at the root |
| 3 | C3 | The companion spec's §3.6, done once at the translator |
| 4 | A1 (with A4, A5 folded in) | The per-tool descriptor. The companion spec's `presentation` then becomes one field on it |
| 5 | B1, B4 | One disclosure model plus a real preview-cap estimate. Needed for content-first tools to estimate correctly |
| 6 | B2, B3, C6, C7, D1, D2 | Consolidation. Each is independent |

The companion spec's P1 (WebSearch) can ship before any of this. If it lands
first, it should still be written against steps 1–2, which are small, so it
doesn't add a sixth switch.

## 4. Method and limits

- Everything above comes from reading the code at `6b5c2b59b` and from grep
  sweeps for duplicated helpers and per-tool switches in `frontend/app/view/agent`
  and `frontend/app/store/agent-document`.
- Header output was confirmed by running the real `ClaudeCodeStreamParser` in a
  throwaway vitest (not committed).
- Result payload shapes were confirmed against real Claude Code transcripts on
  this machine.
- **Not verified live on screen:** the rendered pixels of the header row (the
  data and the render code agree; no screenshot was taken), and whether the
  tool box still needs `contain` plus the JS wheel hand-off (B3).
- The Codex/other-provider paths were read, but no Codex transcripts were
  sampled.
