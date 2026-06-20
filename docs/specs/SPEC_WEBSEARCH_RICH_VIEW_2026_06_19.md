# SPEC: Web-search rich result view

**Date:** 2026-06-19
**Status:** Planned
**Author:** smike
**Related:** `frontend/app/view/agent/components/tool-renderers/SearchResults.tsx`,
             `frontend/app/view/agent/components/tool-renderers/search-results.ts`,
             `frontend/app/view/agent/stream-parser.ts`

---

## 1. Problem

Web-search tool results are currently displayed as a collapsed JSON string in the tool
overlay. The user sees raw structured data instead of a readable list of sources.

The infrastructure to render rich cards already exists (`SearchResults.tsx` + registry)
but two defects prevent it from firing, and the card design that does render is sparse
(no favicon, no citation numbers, no query display, no page date).

---

## 2. Current implementation

### 2.1 Rendering pipeline

```
ToolBlock (collapsed row) → ToolBlockOverlay → ToolOverlayLog
    → renderToolResultBody(node) → resolveToolRenderer(node)
        → "web:search" renderer (byName "WebSearch" | "web_search") → SearchResults.tsx
            → extractSearchResults(node.result)
                ✓ items found → SearchResultCards
                ✗ null       → CompactResult (JSON fallback)
```

`SearchResults.tsx` is imported by `ToolOverlayLog.tsx` (line 35) so the registration
side-effect runs — the renderer is live.

### 2.2 Root causes of JSON fallback

**Defect 1 — JSON-string content not parsed (primary)**

`claude-translator.ts buildToolResults()` (line 297):
```typescript
const fallback = typeof block.content === "string"
    ? { content: block.content }   // ← wraps JSON string as { content: "..." }
    : block.content;
```

When Claude Code CLI serialises the `tool_result` content as a JSON string (common for
web_search because the result array isn't bash stdout), `block.content` arrives as a
string. The fallback wraps it as `{ content: "[{...}]" }`.

`extractSearchResults` checks `ARRAY_KEYS` for `content`, but
`Array.isArray("[{...}]")` is false (it's a string), so the function returns null and
`CompactResult` renders the JSON blob.

Fix: `findResultArray()` should try `JSON.parse` when a key value is a string.

**Defect 2 — canApplyStructured overwrites structured web results**

When exactly one `tool_result` block is present and the event carries a sibling
`tool_use_result` (Bash-style `{ stdout, stderr, interrupted }`), `buildToolResults`
uses that structured result instead of `block.content`. For web_search the sibling is
either absent or carries terminal-style metadata — neither shape is search-results-shaped.

Fix: treat `tool_use_result` as supplemental only for terminal tools. Don't apply it
when the tool name is `web_search` / `WebSearch`, or when the sibling has only
`{ stdout, stderr }` fields but the block content is an array.

### 2.3 Claude web_search_result block shape

When Claude uses `web_search`, the `tool_result.content` is an array of content blocks:

```json
[
  {
    "type": "web_search_result",
    "url": "https://example.com/page",
    "title": "Page title",
    "encrypted_content": "<opaque>",
    "page_age": "June 15, 2026"
  }
]
```

Key notes:
- No `snippet` field — only `title`, `url`, `encrypted_content`, `page_age`
- `encrypted_content` is opaque (internal to Anthropic's API); ignore it in display
- `page_age` is a human-readable date string, NOT a snippet — current code uses it as
  a snippet fallback which is misleading

### 2.4 Current card design (sparse)

Collapsed summary: `🛠️ web_search ✓` — no query, no result count.

Expanded overlay per card:
```
[Title]            ← blue, clickable, truncated
example.com/path   ← green, URL only
June 15, 2026      ← page_age shown as snippet (misleading)
```

---

## 3. Design

### 3.1 Fix extraction (search-results.ts)

**A: parse JSON strings**

```typescript
function tryParseJsonArray(v: unknown): unknown[] | null {
    if (typeof v !== "string") return null;
    try {
        const p = JSON.parse(v);
        return Array.isArray(p) ? p : null;
    } catch {
        return null;
    }
}

function findResultArray(result: unknown): unknown[] | null {
    if (Array.isArray(result)) return result;
    // Top-level JSON string
    const topLevel = tryParseJsonArray(result);
    if (topLevel) return topLevel;

    if (result && typeof result === "object") {
        const o = result as Record<string, unknown>;
        for (const k of ARRAY_KEYS) {
            if (Array.isArray(o[k])) return o[k] as unknown[];
            // String value may be a JSON-encoded array
            const parsed = tryParseJsonArray(o[k]);
            if (parsed) return parsed;
        }
    }
    return null;
}
```

**B: handle `web_search_result` typed objects**

Extract `page_age` as date metadata (not snippet):

```typescript
export interface SearchResultItem {
    title: string;
    url: string;
    snippet?: string;
    date?: string;      // ← new: from page_age
    index?: number;     // ← new: 1-based citation number
}
```

In extraction:
```typescript
const date   = str(o.page_age) ?? str(o.published_date) ?? str(o.date) ?? undefined;
const snippet =
    str(o.snippet) ??
    str(o.description) ??
    str(o.text) ??
    (typeof o.content === "string" ? str(o.content) : undefined) ??
    undefined;
```

`page_age` moves from the snippet fallback slot to the dedicated `date` field.

### 3.2 Fix canApplyStructured in claude-translator.ts

Add guard: skip applying structured result when it's terminal-shaped:

```typescript
const isTerminalResult = (r: any): boolean =>
    r && typeof r === "object" && ("stdout" in r || "stderr" in r || "interrupted" in r)
    && !Array.isArray(r);

const canApplyStructured =
    toolResultBlocks.length === 1
    && structuredResult
    && typeof structuredResult === "object"
    && !isTerminalResult(structuredResult) === false  // only apply for terminal tools
    ...
```

Simpler alternative: add `web_search` / `WebSearch` to a block-list that prevents
overwriting array-shaped content. Preferred approach: only apply `structuredResult`
when the block content is a string (stdout path); if `block.content` is already an
array or parsed object, use it directly.

### 3.3 Search query in collapsed summary (stream-parser.ts)

Add `web_search` to `extractToolDetail`:

```typescript
case "web_search":
case "WebSearch":
    return params.query || "";
```

Add to `TOOL_ICONS` in `types.ts`:

```typescript
WebSearch: "🌐",
web_search: "🌐",
```

Collapsed row becomes: `🌐 web_search "agentmux architecture" (1.2s) ✓`

### 3.4 Improved card layout (SearchResults.tsx)

New card structure per result:

```
┌─────────────────────────────────────────────────────────┐
│ [favicon] example.com                         [1]        │
│ Page title (clickable, blue)                             │
│ Snippet text if available, max 3 lines                   │
│ Jun 15, 2026                                             │
└─────────────────────────────────────────────────────────┘
```

- **Favicon**: `https://www.google.com/s2/favicons?domain={host}&sz=16` — 16×16 img,
  fail silently (hide on error, no broken image icon)
- **Domain**: host extracted from URL, shown in muted text next to favicon
- **Citation number**: `[N]` in top-right corner, 1-based, muted
- **Title**: blue, clickable (opens in system browser), single line with ellipsis
- **Snippet**: shown only when present; max 3 lines clamped
- **Date**: shown as `page_age` when present, muted, bottom of card

### 3.5 Header bar

Above the card list, inside the overlay:

```
🌐  5 results  ·  "agentmux architecture"
```

- Result count (pulled from items.length)
- Query string (from `node.params.query` if available)
- Separated by `·`
- Only shown when items.length > 0

No "Open all" button — too disruptive for a passive result view.

### 3.6 Error / empty state

When extraction returns null (unknown result shape): retain `CompactResult` fallback —
no regression on unexpected payloads.

When extraction returns empty array: show "No results found." message instead of blank.

---

## 4. Files changed

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/tool-renderers/search-results.ts` | JSON-string parsing in `findResultArray`; add `date`/`index` to `SearchResultItem`; fix `page_age` field mapping |
| `frontend/app/view/agent/providers/claude-translator.ts` | Guard `canApplyStructured` to not overwrite array content with terminal result |
| `frontend/app/view/agent/stream-parser.ts` | Add `web_search`/`WebSearch` to `extractToolDetail` |
| `frontend/app/view/agent/types.ts` | Add `web_search`/`WebSearch` to `TOOL_ICONS` |
| `frontend/app/view/agent/components/tool-renderers/SearchResults.tsx` | Favicon, citation number, header bar, date field, empty state |
| `frontend/app/view/agent/styles/_document-nodes.scss` | New card layout styles (favicon row, citation badge, date line) |

---

## 5. Acceptance criteria

- `extractSearchResults` returns non-null for:
  - Top-level `web_search_result` array (already works)
  - `{ content: "[{...}]" }` where value is a JSON string (bug fix)
  - `{ web_search_results: [...] }` keyed array
- Collapsed tool row shows `🌐 web_search "query" ✓` with actual query text
- Expanded overlay shows:
  - Header: result count + query
  - Per card: favicon, domain, citation number, title, snippet (if any), date (if any)
  - Clicking a card opens the URL in the system browser
  - Empty array → "No results found." message
  - Unrecognised shape → CompactResult (no regression)
- `npx tsc -p tsconfig.json --noEmit` clean (ignoring pre-existing errors)
- Existing `search-results.test.ts` tests pass; new tests cover JSON-string input

---

## 6. Out of scope

- `encrypted_content` decryption / rendering — opaque, ignore
- Inline preview / hover card — future work
- Search result caching or deduplication across turns
- "Open all results" — disruptive, skip
