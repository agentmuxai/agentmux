# Spec: One descriptor per tool (agent pane)

**Date:** 2026-09-26
**Status:** active — PR 1 (descriptors, status table) implemented in #3901; PRs 2–3 proposed
**Scope:** `frontend/app/view/agent/` — tool rows, tool previews, the activity row
**Verified against:** `main` @ `ba9abe92f` (after #3871, #3874, #3877, #3883)
**Source:** `docs/reports/REPORT_TOOL_PREVIEW_DRY_AND_ARCHITECTURE_2026_09_26.md`
items A1, A3, A4, A5, C4, C5
**Companion:** `SPEC_AGENT_PANE_ROW_DISCLOSURE_2026_09_26.md` (the row
open/closed model; it reads this spec's `presentation` field)

---

## 0. TL;DR

What the agent pane knows about a given tool is split across about a dozen
places, each keyed on a slightly different name (the coarse kind, the raw name,
or a lowercase provider alias). Adding or changing one tool this week meant
editing 5–7 files, and the header-row bugs fixed in #3871 (a status glyph shown
twice, a stale `⏳`) came from exactly that drift.

This spec moves every per-tool **fact** into one pure table of descriptors, with
one resolver, and makes the renderer registry explicit instead of
side-effect-registered. Rendering stays in the renderer registry, because the
modules that need the facts can't import JSX (§2).

## 1. Where per-tool knowledge lives today

| Fact | Where | Keyed on |
|------|-------|----------|
| Icon | `TOOL_ICONS` (`types.ts:903`) | coarse kinds **and** raw names mixed |
| Header detail | `extractToolDetail` (`stream-parser.ts:135`) | raw name or coarse kind, case by case |
| Activity-row argument | `extractToolArg` (`useAgentStream.ts:79`) | lowercase / provider aliases (`read_file`, `str_replace_editor`, `computer`), then a generic key fallback: **a second detail extractor that disagrees with the first** |
| Header label rules | `SELF_DESCRIBING`, `mcpDisplayName` (`components/tool-header.ts:28-40`) | raw name |
| Content-first / start at top | `CONTENT_FIRST` (`components/tool-presentation.ts:18`) | raw name |
| Start at top (documents) | `DOCUMENT_KINDS` (`components/ToolOverlayLog.tsx:67`) | coarse kind |
| Result pill | `resultPill()` switch (`components/ToolBlock.tsx:243-320`) | coarse kind, plus a raw-name WebSearch branch |
| Compact one-liner | `summarize()` switch (`components/CompactResult.tsx:29-95`) | coarse kind |
| Text read order | `readFrom` (`CompactResult.tsx:129`) | coarse kind (`Grep`, `Glob`) |
| Compact body starts expanded | `createSignal(props.tool === "Glob")` (`CompactResult.tsx:150`) | coarse kind |
| Height estimate | `estimateTool` (`virtualization/renderers.ts:112`) | the content-first predicate |
| Kind normalization | `knownTools` (`stream-parser.ts:830`) | raw name → coarse kind |
| Status glyph / label / activity | `STATUS_ICON` (`ToolBlock.tsx:129`), `STATUS_LABEL` (`ToolBlockOverlay.tsx:41`), `toolActivityStatus` (`activity/tool-adapter.ts:216`), `isActive` / `isFailTerminal` (`ToolBlock.tsx`) | status |
| Body renderer | registry (`components/tool-renderers/registry.ts`), built-ins registered in `ToolOverlayLog.tsx:819-827`, others by side-effect import (`ToolOverlayLog.tsx:40-44`) | coarse kind / raw name / shape |

Known drift today, found by capturing every fact for 20 real tool names before
the migration:
- A Codex `read_file` / `str_replace_editor` call shows its path in the working
  row (`extractToolArg`) but has no icon and no detail in its header
  (`TOOL_ICONS` and `extractToolDetail` have no alias cases).
- The Activity Dock titles a WebSearch or WebFetch row "WebSearch" /
  "WebFetch": `activity/tool-adapter.ts` looks the detail up by the coarse kind
  (`Other`), which has none.

### 1.1 Intended behavior changes (everything else is pinned by the parity test)

| Tool | Change |
|------|--------|
| Codex aliases `read`, `read_file`, `write`, `write_file`, `edit`, `str_replace_editor`, `multiedit`, `bash`, `grep`, `glob` | The header and summary gain the kind's icon and detail; `read_file` / `write_file` / `str_replace_editor` previews open at the top like Read/Write/Edit |
| `computer` | The header gains its `command` as detail |
| WebSearch, WebFetch | The Activity Dock title is the query / host+path, not the tool name |
| Agent, Workflow, WebFetch | The working row's argument is the description / title / host+path (previously none) |

## 2. Design

### 2.1 Two layers, one key

| Layer | Holds | Why separate |
|-------|-------|--------------|
| **Descriptors** (`tool-meta/tool-descriptors.ts`, new) | Every per-tool *fact*: icon, label, detail, pill, presentation, scroll, compact-body behavior | Pure TypeScript with no JSX and no Solid, and no side effects. `stream-parser.ts`, `useAgentStream.ts`, `virtualization/*`, `btw.ts` and `activity/*` need these facts and must not pull in the renderer module graph |
| **Renderers** (`tool-renderers/registry.ts`, unchanged API) | How a result body renders (JSX) | Only `ToolOverlayLog` needs them |

Both layers match on the same key: `toolNameOf(node)`, the raw provider name
falling back to the coarse kind. The existing `toolNameOf` moves into
`tool-descriptors.ts`, and `registry.ts` imports it from there.

### 2.2 The descriptor

```ts
interface ToolDescriptor {
    /** Raw names this descriptor covers, e.g. ["Read", "read_file"]. */
    names?: readonly string[];
    /** Or a raw-name prefix, e.g. "mcp__". */
    prefix?: string;
    icon?: string;
    /** Header label; null omits it (the web tools with a detail). Default: the raw name. */
    label?: (name: string, detail: string) => string | null;
    /** The call's main argument: path, command, query, host/path. */
    detail?: (params: Record<string, any>) => string;
    /** Header pill once the tool has a result. */
    pill?: (node: ToolNode) => Pill | null;
    /** "content": expanded by default once finished (the disclosure spec). */
    presentation?: "panel" | "content";
    /** Where the preview box starts: following the latest output, or at the top. */
    scroll?: "follow" | "top";
    /** Read order for a text body in CompactResult. */
    readFrom?: "head" | "tail";
    /** CompactResult one-liner for a structured result. */
    compactSummary?: (params: Record<string, any>, result: any) => string | null;
    /** CompactResult's structured body starts expanded (Glob's file list). */
    compactStartsExpanded?: boolean;
}
```

There's one exported, ordered array `TOOL_DESCRIPTORS`, and **no registration
calls**. A module that needs a fact imports the resolver, which is complete the
moment the module loads.

### 2.3 Resolution: per field

```ts
function toolFact<K extends keyof ToolDescriptor>(node: ToolNode, key: K): ToolDescriptor[K] | undefined
```

This returns the first descriptor in `TOOL_DESCRIPTORS` that **matches the
node and defines the field** (a `names` match before a `prefix` match, and
table order otherwise). A generic descriptor such as `prefix: "mcp__"` can then
supply `label` without also having to supply `detail` or `pill`. A final
catch-all descriptor supplies the defaults: icon 🛠️, label = the raw name, and
**no** header detail. The generic key fallback (`file_path`, `path`, `command`,
`query`, `pattern`) applies only to the activity row's argument, as it does
today. In the header it would add a detail to every MCP and ToolSearch row.

Thin helpers on top of it are what callers actually use: `toolIcon(node)`,
`toolDetail(name, params)`, `toolPill(node)`, `isContentFirstTool(node)`,
`startsAtTop(node)`, `textReadOrder(node)`, and so on.

### 2.4 What moves where

| Today | Becomes |
|-------|---------|
| `TOOL_ICONS` | `icon` on each descriptor. Its two call sites move to `toolIcon()` and the export goes |
| `extractToolDetail` + `extractToolArg` | `toolDetail(name, params)` (the explicit per-tool `detail`, used by the header, the summary and the Activity Dock) and `toolActivityArg(name, params)` (`toolDetail`, then the generic key fallback). One source of per-tool cases; both old functions go |
| `SELF_DESCRIBING`, `mcpDisplayName` | `label` on the web descriptors and on the `mcp__` prefix descriptor |
| `CONTENT_FIRST`, `DOCUMENT_KINDS` | `presentation: "content"` and `scroll: "top"` on WebSearch; `scroll: "top"` on Read, Write and Edit. `components/tool-presentation.ts` goes |
| `resultPill()` switch | A `pill` on Bash (exit code, including the `<exited N>` parse), Glob, Grep (`grepResultCount`), Write, Edit and WebSearch. The Agent/Task/Workflow dispatch pill stays in `ToolBlock`, because it needs the ordinal-matched `dispatchMatch` (context the descriptor doesn't have) |
| `summarize()` switch, the Glob/Grep checks in `CompactResult` | `compactSummary`, `readFrom`, `compactStartsExpanded` |
| `STATUS_ICON`, `STATUS_LABEL`, `toolActivityStatus`, `isActive`, `isFailTerminal` | One `TOOL_STATUS` table in `tool-meta/tool-status.ts`: `{ icon, label, activity, active, dismissed }` per status (report A3) |
| `knownTools` in `normalizeToolName` | Unchanged. The coarse kind is the renderer layer's business and `ToolNode.tool`'s type; descriptors never read it |

### 2.5 The renderer registry becomes explicit (report C4, C5)

- The built-in renderers (`renderEdit`, `renderBash`, `renderRead`,
  `renderWrite`, `renderSearch`, `renderAgent`, `renderTask`, `renderWorkflow`,
  `renderCompactDefault`) move out of `ToolOverlayLog.tsx` into
  `tool-renderers/builtins.tsx`. That removes the
  `DispatchCard` ↔ `ToolOverlayLog` import cycle.
- `tool-renderers/index.ts` exports `registerToolRenderers()`, which registers
  every renderer (built-ins, SearchResults, WebFetchResult, RecordTable,
  DispatchCard, ToolReferences) in one visible, ordered list. The five
  side-effect imports in `ToolOverlayLog.tsx` become one call, made once.
  Registration stays idempotent by label, so HMR and double calls are safe.

## 3. Non-goals

- No visual change beyond the §1.1 list. Every pill,
  label, icon and body stays as rendered today, and the tests pin that.
- The coarse `ToolNode.tool` enum stays as it is (it's persisted in snapshots and
  used by `byKind`).
- No new tools or renderers.

## 4. Phasing

1. **PR 1: descriptors and status table.** Covers `tool-descriptors.ts`,
   `tool-status.ts`, and migrating icon, label, detail (including the activity
   row), content-first, scroll, and status. Each old function is deleted as its
   last caller moves; none is kept as a wrapper.
2. **PR 2: pills and CompactResult facts.** Covers `pill`, `compactSummary`,
   `readFrom` and `compactStartsExpanded`.
3. **PR 3: explicit renderer registration.** Covers `builtins.tsx`,
   `registerToolRenderers()`, the end of the import cycle, and the end of the
   side-effect imports.

## 5. Tests

- **`tool-descriptors.test.ts`:**
  - Field-wise resolution: a prefix match supplies `label` while the catch-all
    supplies `icon`.
  - A `names` match beats a `prefix` match.
  - Every descriptor with `names` resolves each of its names.
  - The catch-all defaults.
- **Parity tables,** the core of this spec. For every tool in a fixture list
  (Read, Write, Edit, Bash, Grep ×3 modes, Glob, Agent, WebSearch, WebFetch,
  ToolSearch, `mcp__agentmux__WhoAmI`, TaskOutput, Codex `read_file`), assert
  that the header text, pill, icon, `isContentFirstTool`, `startsAtTop`, the
  compact summary and the text read order are identical before and after the
  migration. The only expected diffs are §1.1's, and the table lists each one
  explicitly.
- **`tool-status.test.ts`:** every `ToolNode["status"]` has an entry (a
  `Record` type makes this a compile error too).
- **PR 3:** the registered-label list equals the explicit list; there's no
  import cycle (`madge --circular` or an equivalent test on the two modules);
  the existing `registry.test.ts` parity suite passes unchanged.
- The full agent and store suites and `tsc` pass. A live dev-build check,
  rendering real tool shapes, shows no visual change.

## 6. Acceptance

- Adding a tool's icon, label, detail, pill or presentation touches one file,
  `tool-descriptors.ts`.
- `extractToolArg`, `SELF_DESCRIBING`, `CONTENT_FIRST`, `DOCUMENT_KINDS`, the
  `resultPill` switch arms (except dispatch) and the `summarize` switch no
  longer exist.
- `ToolOverlayLog.tsx` has no renderer bodies and no side-effect imports.
- Rendering matches before and after (the parity tables and the live check).
