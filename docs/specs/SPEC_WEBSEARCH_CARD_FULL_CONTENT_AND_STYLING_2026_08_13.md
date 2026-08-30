# Spec: WebSearch tool-card — full (unclamped) content + styling fixes

**Date:** 2026-08-13
**Type:** Design spec (frontend-only, Agent pane)
**Status:** Proposed — not yet implemented
**Trigger:** User request — in the Agent pane's tool-call stream, WebSearch results should show their full content (not visually cut off), and the card should be reviewed for styling improvements.
**Builds on:** `docs/specs/SPEC_WEBSEARCH_RICH_VIEW_2026_06_19.md` (the spec that built the current card — already implemented; this spec revises two of its deliberate decisions in light of the new request) and `docs/specs/SPEC_TOOL_OUTPUT_CAP_2026_05_30.md` (the codebase's general "a render cap is never silent" rule, which the current snippet clamp violates — see §2.2).

---

## 0. TL;DR

WebSearch already has a dedicated, registry-based rich card renderer (`SearchResults.tsx`) — it does not fall through to the generic JSON view. Two concrete, evidence-backed problems:

1. **Content is clamped with no way to see the rest.** Each result's snippet is hard CSS-clamped to 3 lines (`-webkit-line-clamp: 3`) and its title to one line with ellipsis — both silently, with no "show more" affordance and no `OutputHiddenMarker`, unlike every other capped output in this codebase.
2. **A styling rule already exists for WebSearch but never fires** — a `data-tool="websearch"` CSS selector for the row's identity border-color is dead code, because the attribute is actually set from the coarse tool *kind* (which WebSearch always normalizes to `"Other"`), not the raw tool name.

This spec fixes both, plus flags two smaller, optional polish items (missing name-color rule, WebSearch/WebFetch sharing one icon).

---

## 1. Current implementation (verified against source)

Rendering pipeline, per `SPEC_WEBSEARCH_RICH_VIEW_2026_06_19.md` §2.1 (unchanged since):

```
ToolBlock (collapsed row) → ToolBlockOverlay → ToolOverlayLog
    → resolveToolRenderer(node) → "web:search" renderer
        (byName("WebSearch","web_search"), priority 10 — SearchResults.tsx:147-152)
        → extractSearchResults(node.result)  (search-results.ts)
            ✓ items found → SearchResultCards (SearchResults.tsx:72-94)
            ✗ null        → CompactResult (JSON fallback)
```

This registry (`frontend/app/view/agent/components/tool-renderers/registry.ts`) is the codebase's established per-tool-name customization point — `registerToolRenderer({ priority, label, match, render })`, matched by `byName`/`byNamePrefix`/`byShape`, highest `priority` wins. This spec extends existing registrations; it does not need a new mechanism.

`ToolNode` (`frontend/app/view/agent/types.ts:225-237`) carries two separate name fields:
- `tool`: a closed coarse kind — `"Read" | "Edit" | "Bash" | "Write" | "Grep" | "Glob" | "Task" | "Agent" | "Other"`. `stream-parser.ts`'s `normalizeToolName` (`stream-parser.ts:738-745`) maps anything not in that 8-name list — including `WebSearch`/`web_search` — to `"Other"`.
- `toolName?: string`: the raw provider tool name (`"WebSearch"`, `"web_search"`, `"mcp__..."`, etc.), preserved specifically so the renderer registry (and, per its own doc comment, anything else that needs the real name) doesn't have to go through the lossy coarse kind.

---

## 2. Problems

### 2.1 Snippet and title are clamped with no expand affordance

`frontend/app/view/agent/styles/_document-nodes.scss:1435-1453`:
```scss
.agent-search-card-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;       // single line, hard cut mid-word
    ...
}
.agent-search-card-snippet {
    display: -webkit-box;
    -webkit-line-clamp: 3;     // hard cut at 3 lines
    -webkit-box-orient: vertical;
    overflow: hidden;
    ...
}
```
This was a deliberate choice in the prior spec (`SPEC_WEBSEARCH_RICH_VIEW_2026_06_19.md` §3.4: *"Snippet: shown only when present; max 3 lines clamped"*) — not a bug. This spec proposes revisiting that choice per the new, explicit request.

**Why this is worth fixing, not just a preference call:** `docs/specs/SPEC_TOOL_OUTPUT_CAP_2026_05_30.md` §6 establishes a codebase-wide rule, quoted verbatim in `OutputHiddenMarker.tsx`'s own header comment: *"A cap is never silent."* Every other capped tool output in the Agent pane (Bash stdout, Read content, the search-results **list** itself via `MAX_TOOL_OUTPUT_LINES` in `SearchResults.tsx:73`) shows an `OutputHiddenMarker` ("… N more lines hidden") when content is cut. The per-card snippet/title clamp is the one place that cuts content with **zero indication anything was cut and zero way to see the rest** — worse than the codebase's own standard, not just a style nit.

**Scale check — is unclamping actually safe?** Search-result snippets are provider-supplied summaries, typically one to a few sentences (a few hundred characters), not full page bodies — nothing like the multi-KB/MB bodies `MAX_TOOL_OUTPUT_LINES`/`MAX_TOOL_OUTPUT_CHARS` (`output-cap.ts:16,22`) exist to bound. Removing the per-card clamp does not reintroduce the DOM-bloat risk `SPEC_TOOL_OUTPUT_CAP_2026_05_30.md` was written for — the *list* still IS capped (§ above, unchanged by this spec) for exactly that reason; only the *within-card* text truncation is in scope here.

### 2.2 The WebSearch/WebFetch identity-color CSS rule is dead

`frontend/app/view/agent/components/ToolBlock.tsx:368`:
```tsx
data-tool={props.node.tool.toLowerCase()}
```
uses the **coarse** `tool` field. Since `normalizeToolName` always maps WebSearch to `"Other"` (§1), this attribute is always `data-tool="other"` for a WebSearch row — never `"websearch"`. But `_document-nodes.scss:493-494` has:
```scss
.agent-tool-block[data-tool="webfetch"]:not(.running):not(.failed):not(.pinned),
.agent-tool-block[data-tool="websearch"]:not(.running):not(.failed):not(.pinned)
  { border-left-color: var(--term-bright-blue); }
```
This selector can never match anything as the code stands today — confirmed no other call site ever sets `data-tool` to `"websearch"`/`"webfetch"`, and `ToolNode.tool`'s type doesn't even include those as literals (`types.ts:228`). The rule is inert code, not a defect a reviewer would spot by reading the SCSS in isolation — only cross-referencing the two files shows it.

**Secondary gap, same root cause:** even if `data-tool` fired correctly, the collapsed row's tool-name text color list (`_document-nodes.scss:473-481`, one rule per `data-tool` value for `.agent-tool-name`) has no `websearch`/`webfetch` entries at all — only the border-left strip was ever specced for these two, not the name text. Worth closing in the same pass since it's the same mechanism.

### 2.3 WebSearch and WebFetch are visually identical (minor, optional)

`types.ts:775-778`: `TOOL_ICONS.WebSearch = TOOL_ICONS.web_search = TOOL_ICONS.WebFetch = TOOL_ICONS.web_fetch = "🌐"`. Once §2.2 is fixed, both tools would still share one accent color *and* one icon — indistinguishable from each other in the collapsed row (only the tool-name text itself, e.g. "WebSearch" vs "WebFetch", would differ). Flagged as optional; the user's request centered on WebSearch specifically, and WebFetch's own rendering is out of scope here.

---

## 3. Design

### 3.1 Un-clamp snippet and title (SCSS-only change)

`.agent-search-card-snippet`: drop `-webkit-line-clamp`/`-webkit-box`/`overflow: hidden`; let it wrap naturally (`white-space: normal`, keep `line-height`). No hidden-content marker needed once nothing is hidden — this satisfies §2.1's "cap is never silent" gap by removing the cap, not by adding UI to a cap that no longer exists.

`.agent-search-card-title`: switch from single-line `nowrap` + ellipsis to wrapping up to 2 lines (`-webkit-line-clamp: 2` is acceptable *here* specifically because titles are reliably short — one sentence — unlike snippets, which are the actual content the user asked to stop cutting; if a title still overflows 2 lines in practice, revisit). Alternative, simpler option: let the title wrap fully too, with no clamp at all, matching the snippet — recommended default unless it visually crowds the card in practice.

`.agent-search-card-domain` and `.agent-search-query`/header stay single-line-ellipsis as-is — these are identifiers (a URL host, a short query string), not the content the user is asking to see in full.

### 3.2 Fix the `data-tool` attribute to use the raw tool name

`ToolBlock.tsx:368` — introduce a small helper (co-located in `ToolBlock.tsx` or `types.ts`) instead of a bare `.toLowerCase()`:

```ts
/** DOM identity hook for per-tool CSS. Prefers the raw provider name for the
 *  handful of tools whose coarse `tool` kind collapses to "Other" but that
 *  still have dedicated styling (WebSearch/WebFetch today); falls back to
 *  the coarse kind for everything else, unchanged. */
function toolDataAttr(node: ToolNode): string {
    const raw = node.toolName?.toLowerCase();
    if (raw === "websearch" || raw === "web_search") return "websearch";
    if (raw === "webfetch" || raw === "web_fetch") return "webfetch";
    return node.tool.toLowerCase();
}
```
and `data-tool={toolDataAttr(props.node)}`. This is deliberately narrow (an explicit allow-list of the two names that currently have dead CSS waiting for them) rather than blanket-switching every row to `toolName` — that would be a much larger behavior change (every `mcp__*`/unknown tool's `data-tool` would change value) for a bug that only affects two specific tools today.

### 3.3 Add the missing name-color rule

`_document-nodes.scss:473-481` — add, alongside the existing per-`data-tool` `.agent-tool-name` color rules:
```scss
.agent-tool-block[data-tool="websearch"] .agent-tool-name,
.agent-tool-block[data-tool="webfetch"]  .agent-tool-name { color: var(--term-bright-blue); }
```
(matching the border-left color already specced for both, §2.2, so the row's name text and its border strip agree once §3.2 makes the attribute real.)

### 3.4 (Optional) Distinguish WebSearch from WebFetch visually

Not required to close the user's request (which named WebSearch specifically) but worth a one-line note: if pursued, change one of the two icons in `types.ts:775-778` (e.g. keep 🌐 for `WebSearch`, use a different glyph — e.g. a link/page icon — for `WebFetch`, since "search the web" and "fetch one known URL" are different actions and a shared globe icon undersells that). No accent-color split proposed (the term-bright-blue "network/remote" identity fits both).

---

## 4. Files changed

| File | Change |
|---|---|
| `frontend/app/view/agent/styles/_document-nodes.scss` | Remove `-webkit-line-clamp` from `.agent-search-card-snippet`; relax/remove single-line clamp on `.agent-search-card-title`; add the missing `websearch`/`webfetch` `.agent-tool-name` color rule (§3.3) |
| `frontend/app/view/agent/components/ToolBlock.tsx` | Add `toolDataAttr()` helper; use it in place of the bare `props.node.tool.toLowerCase()` for `data-tool` (§3.2) |
| `frontend/app/view/agent/types.ts` (optional, §3.4) | Differentiate `WebFetch`/`web_fetch` icon from `WebSearch`/`web_search` |

No backend/srv changes — this is entirely a frontend rendering/styling fix.

---

## 5. Acceptance criteria

- A WebSearch result card shows its **full snippet text**, wrapped, never cut off — visually confirmed with a long (multi-sentence) snippet.
- A WebSearch result card's **title** is no longer cut mid-word on one line for a long title.
- A completed, non-running, non-failed, non-pinned WebSearch tool row's left border renders in `var(--term-bright-blue)` — confirmed via `data-tool` actually reading `"websearch"` in the DOM (previously always `"other"`).
- The same row's collapsed tool-name text also renders in the same accent color (§3.3).
- No regression to any other tool's `data-tool` value or coloring (Bash/Read/Edit/etc. unaffected — `toolDataAttr` only special-cases WebSearch/WebFetch and falls through unchanged otherwise).
- The overall **results list** cap (`MAX_TOOL_OUTPUT_LINES`, `OutputHiddenMarker`) is untouched — still capped, still marked when capped. Only the per-card text clamp is removed.
- `npx tsc --noEmit` clean.
- Existing `SearchResults.test.tsx`/`search-results.test.ts` pass unmodified (this spec changes no extraction logic, only presentation).

---

## 6. Out of scope

- Any change to `extractSearchResults`/result-shape parsing — already correct per `SPEC_WEBSEARCH_RICH_VIEW_2026_06_19.md`, not touched here.
- WebFetch's own result rendering (it likely falls to `CompactResult` today, unlike WebSearch — a separate renderer for it, if wanted, is its own spec).
- The results **list** cap (`MAX_TOOL_OUTPUT_LINES`) — unrelated to this request, left as-is.
- Redesigning the card layout beyond un-clamping text (favicon/domain/citation-number/date positions stay as shipped in the prior spec).

---

## 7. Sources

- `frontend/app/view/agent/components/tool-renderers/SearchResults.tsx`
- `frontend/app/view/agent/components/tool-renderers/search-results.ts`
- `frontend/app/view/agent/components/tool-renderers/registry.ts`
- `frontend/app/view/agent/components/ToolBlock.tsx` (lines 340-375, `data-tool` + row class wiring)
- `frontend/app/view/agent/components/output-cap.ts`, `OutputHiddenMarker.tsx`
- `frontend/app/view/agent/styles/_document-nodes.scss` (lines 471-494 per-tool identity color rules; 1346-1460 SearchResults card styles)
- `frontend/app/view/agent/stream-parser.ts` (`normalizeToolName`, lines 738-745)
- `frontend/app/view/agent/types.ts` (`ToolNode`, lines 225-237; `TOOL_ICONS`, lines 775-778)
- `docs/specs/SPEC_WEBSEARCH_RICH_VIEW_2026_06_19.md` (the spec that built the current card)
- `docs/specs/SPEC_TOOL_OUTPUT_CAP_2026_05_30.md` (the "a cap is never silent" principle §2.1 relies on)
- `docs/specs/SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md` (the registry pattern §1 describes)
