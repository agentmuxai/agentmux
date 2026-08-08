# SPEC: Tool-result renderer registry (rich, per-tool result UIs that scale)

**Date:** 2026-06-17
**Status:** Proposed (analysis + design; not implemented) (implemented — see note below)
**Author:** smike

> **2026-08-07 audit note:** Implemented, load-bearing — `registry.ts`/
> `registry.test.ts` are the actual mechanism underpinning several other
> still-stale-status specs (WebFetch, WebSearch, Write-MD content views).
> See `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
**Builds on:** `SPEC_TOOL_OUTPUT_TEE_AND_TERMINAL_RENDER_2026_06_17.md` (PR #1511 — `TerminalOutput` + `terminalText`, the first result-shape classifier)
**Components:** `frontend/app/view/agent/stream-parser.ts`, `frontend/app/view/agent/types.ts`, `frontend/app/view/agent/components/ToolOverlayLog.tsx`, `…/components/*`

---

## 1. Problem

Different tool calls want different result UIs, and the set of tools is **open-ended**:
- `WebSearch` → a list of result **cards** (title / url / snippet), not a JSON blob.
- `WebFetch` → readable page text / markdown.
- `TodoWrite` → a checklist.
- MCP tools (`mcp__<server>__<tool>`) → whatever that server returns — images, tables, structured records.
- provider-specific tools we don't know about yet.

Today every one of these renders as **pretty-printed JSON in a `<pre>`** (or, after PR #1511, as a terminal when the result happens to carry a string body). The `TerminalOutput` work fixed the "looks-like-a-terminal" slice; this spec generalizes the pattern so *any* tool can get a rich renderer without editing a hardcoded switch — and, crucially, so the renderer can even **tell which tool it is**.

---

## 2. Current architecture (verified)

### 2.1 The result model cannot name the tool
`ToolNode.tool` is a **closed 9-value union** (`types.ts:213`):
```ts
tool: "Read" | "Edit" | "Bash" | "Write" | "Grep" | "Glob" | "Task" | "Agent" | "Other";
```
Every translator *does* carry the real provider tool name (`claude-translator.ts:287` `block.tool_name`, `acp/gemini/kimi/codex-translator` `toolName`, etc.), but `stream-parser.ts:504-508` collapses anything unknown to `"Other"`:
```ts
private normalizeToolName(tool: string): ToolNode['tool'] {
    const normalized = ...;
    return knownTools.includes(normalized) ? (normalized as ToolNode['tool']) : "Other";
}
```
So **the raw name is thrown away** before a `ToolNode` exists. `WebSearch`, `WebFetch`, `TodoWrite`, and every `mcp__*` tool all become `"Other"` — indistinguishable downstream.

### 2.2 Rendering is a hardcoded switch over that enum
`ToolOverlayLog.tsx:233-312` `renderToolResultBody(node)` is a `switch (node.tool)`:
`Edit`→`DiffViewer`, `Bash`→`BashOutputViewer`, `Read`→`HighlightedCode`,
`Grep`/`Glob`→search block + `CompactResult`, `Agent`/`Task`/**default**→`CompactResult`
(whose expanded body is JSON, or — post #1511 — `TerminalOutput` when `terminalText(result) != null`).

Because every novel tool is `"Other"`, the switch's `default` is the *only* branch they can reach. Adding a rich renderer means adding a `case` to a switch over an enum **the data can't even populate**.

### 2.3 A second tool-keyed dispatch: icons
`types.ts:602` `TOOL_ICONS: Record<string,string>` + `stream-parser.ts:463`
`TOOL_ICONS[tool] || TOOL_ICONS.Other` pick the summary emoji. So tool identity is dispatched in **two** places (render body + icon), both keyed on the closed enum, and the per-node `summary` string is assembled in the parser.

### 2.4 Prior art the registry should subsume
- **#1511** `terminalText(result)` → `TerminalOutput` is exactly a *(classifier → renderer)* pair for one result shape. It's the seed of this design.
- **AskUserQuestion** already renders a rich, non-JSON UI (`AgentQuestionPanel`) — but via a *node field* (`status:"awaiting_answer"` + `node.question`) and a special-case in `stream-parser.ts:311`, not through the result renderer. That's the "rich tool UI" precedent, done ad hoc.

---

## 3. Why it needs a rethink (not just more `case`s)

1. **Open tool universe.** MCP especially means the tool set is unbounded and user-defined; a closed enum + switch can never cover it.
2. **The data is lost at the wrong layer.** Routing is impossible because `normalizeToolName` discards the only key that matters.
3. **Two parallel switches** (render + icon) and an ad-hoc third path (AskUserQuestion) mean every "rich tool" is bespoke wiring.
4. **The real axis is result *shape*, not tool name.** Many tools share a shape (anything terminal-ish → `TerminalOutput`; anything list-of-records → a table; anything search-result-ish → cards). Keying purely on name misses reuse; keying purely on shape misses tool-specific polish. We want both.

---

## 4. Design principle

> **Separate "what is this result" (classification) from "how to render it" (a registry of renderers). Key on the *real* tool name first, result *shape* second, with a graceful fallback chain. Unknown tools degrade to terminal-or-JSON, never break.**

---

## 5. Proposed architecture

### 5.1 Preserve the raw tool name (Phase 0 — the unblocker)
Add `toolName: string` to `ToolNode` (the exact provider name: `"WebSearch"`, `"mcp__github__search_issues"`, …). Keep the coarse `tool` enum as a *derived* "kind" for back-compat (icons, existing switch) during migration. The data already exists at `stream-parser.ts` (`event.tool` before normalization) — this is a one-field carry-through, no translator changes.

### 5.2 A renderer registry
Replace the `switch` with a registry of **matchers → renderers**:
```
type ToolResultRenderer = (node: ToolNode) => JSX.Element;
type Matcher =
  | { name: string }                         // exact: "WebSearch"
  | { prefix: string }                       // "mcp__"
  | { shape: (r: unknown) => boolean };      // predicate, e.g. looksLikeSearchResults
registerToolRenderer(matcher, renderer, priority?)
```
`resolveToolRenderer(node)` walks: **exact name → prefix → shape predicate → fallback chain**, returning the first match. The fallback chain is the current behavior generalized:
`registered renderer` → `terminalText` ⇒ `TerminalOutput` (#1511) → JSON `<pre>` (`CompactResult`).

`renderToolResultBody` becomes a two-liner: `resolveToolRenderer(node)(node)`. The existing per-tool components (`DiffViewer`, `BashOutputViewer`, `HighlightedCode`, …) are migrated to **registrations** that produce byte-identical output — a pure refactor, no UX change.

### 5.3 Reusable result-*kind* components
Shape-driven, tool-agnostic; many tools map to one:
- `TerminalOutput` (done) — stdout/log/plain text.
- `CodeView` — Read / file content (wraps `HighlightedCode`).
- `DiffView` — Edit (wraps `DiffViewer`).
- `FileList` — Glob / file arrays.
- **`SearchResults`** — WebSearch & friends: title / url / snippet cards. *(The first new kind; the proof of the design.)*
- `KeyValueTable` — generic `{records:[…]}` / object-of-scalars.
- `ImageView` — base64 / url images (MCP).
- `ErrorView` — failed results with a message.

### 5.4 Classifier helpers (the "shape" matchers)
Small pure predicates, each tested in isolation (`terminalText` is the template):
`looksLikeSearchResults(r)`, `looksLikeFileList(r)`, `looksLikeImage(r)`, `looksLikeRecords(r)`, … Used both as registry shape-matchers and as the fallback chain.

### 5.5 Unify the second/third dispatch (later phase)
A registry entry can optionally own its **icon** and **summary** contribution, retiring `TOOL_ICONS` + the parser's bespoke summary assembly and folding the AskUserQuestion special-case into a registered renderer (keyed on `status:"awaiting_answer"`). Out of scope for the first cut; called out so the registry is designed to grow into it.

---

## 6. The key fork — where structure comes from

- **(A) Normalize at the translator.** Each provider maps its raw result into a typed tagged union (`{kind:"search", results:[{title,url,snippet}]}`); renderers are dumb + provider-agnostic. **Cleanest long-term**, but touches all ~6 translators and every translator must learn each kind.
- **(B) Classify at render time.** Keep the raw result; shape predicates sniff it. **Incremental**, zero translator churn, but predicates must tolerate provider differences (a Claude `WebSearch` result shape ≠ another provider's).

**Recommendation: hybrid, B-first.** Ship the registry + shape classifiers now (extends #1511 with no translator work). When a kind proves fragile across providers, normalize *that kind* at the translators (A) and switch its matcher to the tagged kind. Name-based matchers (`WebSearch`, `mcp__*`) are provider-stable and carry most of the value without any shape-sniffing.

---

## 7. Migration plan (incremental, each step shippable)

- **Phase 0 — carry the raw name.** `ToolNode.toolName`; populate in `stream-parser.ts`. No render change. (Tiny.)
- **Phase 1 — registry, no behavior change.** Introduce `registerToolRenderer`/`resolveToolRenderer` + the fallback chain; re-register the existing `switch` cases to produce identical output; `renderToolResultBody` → registry lookup. Snapshot/RTL tests assert parity.
- **Phase 2 — first rich kind.** `SearchResults` + `looksLikeSearchResults`, registered for `WebSearch` (name) — the visible payoff and the design's proof.
- **Phase 3 — breadth.** `mcp__*` prefix handling, `ImageView`/`KeyValueTable`/`FileList`, then unify icons/summary (§5.5) and fold in AskUserQuestion.

Each phase is its own small PR. Phase 0+1 are a pure refactor; the user-visible win starts at Phase 2.

---

## 8. Relationship to PR #1511

Not throwaway — the **seed**. `terminalText`→`TerminalOutput` is precisely a *(classifier, kind-renderer)* pair and slots in as the terminal entry of the fallback chain. This spec generalizes that one pair into the registry + a set of kinds.

---

## 9. Risks / when NOT to

- **Over-abstraction.** If the tool set were closed and small, the `switch` is fine. The justification is specifically the **open** universe (MCP, web tools, provider churn). Keep the registry thin — it's a `Map` + a fallback chain, not a framework.
- **Shape-sniffing fragility.** Mitigated by making **name matchers primary** and shape predicates the fallback; and by escalating flaky kinds to translator normalization (A).
- **Result shape variance across providers.** Same mitigation; predicates must be defensive (typeof checks, optional fields) like `terminalText` already is.
- **Scope creep into icons/summary.** Deliberately deferred to Phase 3 so the first cut stays small.

---

## 10. Open questions
- Keep the coarse `tool` enum as a derived "kind" long-term, or replace it entirely with `toolName` + a derived kind? (Lean: keep during migration, reassess at Phase 3.)
- Should a registered renderer also drive the **collapsed summary** (one source of truth), or is the parser-built summary good enough? (Lean: Phase 3.)
- Where do renderers live + how do tests assert parity for Phase 1? (Lean: `components/tool-renderers/` + registration table with snapshot tests.)

---

## 11. Key file references
- `frontend/app/view/agent/types.ts:213` — closed `ToolNode.tool` enum (add `toolName`)
- `frontend/app/view/agent/types.ts:602` — `TOOL_ICONS` (second tool-keyed dispatch)
- `frontend/app/view/agent/stream-parser.ts:504-508` — `normalizeToolName` (where the raw name is dropped)
- `frontend/app/view/agent/stream-parser.ts:311,463` — AskUserQuestion special-case + icon pick
- `frontend/app/view/agent/components/ToolOverlayLog.tsx:233-312` — `renderToolResultBody` switch (becomes the registry lookup)
- `frontend/app/view/agent/components/CompactResult.tsx` — JSON/terminal fallback (becomes the chain's tail)
- `frontend/app/view/agent/components/TerminalOutput.tsx` + `terminal-text.ts` — the seed (#1511)
- provider translators (`claude-/codex-/acp-/gemini-/kimi-translator.ts`) — carry raw tool name; the (A) normalization site
