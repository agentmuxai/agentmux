# SPEC: WebFetch content view

**Date:** 2026-06-22
**Status:** Planned (implemented — see note below)
**Author:** Lark

> **2026-08-07 audit note:** Implemented (`WebFetchResult.tsx`, PR #1706).
> Status field was never updated. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
**Related:** `frontend/app/view/agent/components/tool-renderers/WebFetchResult.tsx`,
             `SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md`,
             `RETRO_WEBSEARCH_RICHVIEW_SHIPPED_BROKEN_2026_06_22.md`

---

## 1. Problem

`WebFetch` tool results are rendered as a collapsed JSON blob — the same baseline
experience that WebSearch had before the card renderer was built. The result is often
a large string of page content, which is unreadable in the raw form.

---

## 2. WebFetch result shape

The AgentMux `WebFetch` tool (MCP tool name: `WebFetch`) returns results in one of
two shapes depending on provider and version:

**Shape A — string** (most common; plain text or truncated HTML)
```
"Page content here..."
```

**Shape B — structured object**
```json
{
  "url": "https://example.com/page",
  "status": 200,
  "content": "Page content here...",
  "title": "Page Title",
  "truncated": true
}
```

The renderer must handle both gracefully. Shape A falls through to `CompactResult`
with a reasonable content preview; Shape B enables the richer header display.

---

## 3. Design

### 3.1 Rendering pipeline

Same pattern as WebSearch:

```
ToolBlock (collapsed) → ToolBlockOverlay → ToolOverlayLog
    → resolveToolRenderer(node)
        → "web:fetch" renderer (byName "WebFetch", "web_fetch")
            → WebFetchResult.tsx
                → extractFetchResult(node.result)
                    → FetchResultView (structured)
                    → CompactResult (string fallback)
```

### 3.2 Extraction (web-fetch-result.ts)

```typescript
export interface FetchResultData {
    url?: string;
    title?: string;
    status?: number;
    content: string;        // always present
    truncated?: boolean;
    contentType?: string;
}

export function extractFetchResult(result: unknown): FetchResultData | null
```

Extraction rules:
- If `result` is a string: return `{ content: result }` — always succeeds
- If `result` is an object: extract `content` (required), `url`, `title`, `status`,
  `truncated`, `contentType` from best-effort field name matching
  (`content_type`, `mimeType`, `mime_type`, etc.)
- If nothing matches: return null (falls through to CompactResult)

### 3.3 Component layout (WebFetchResult.tsx)

```
┌─────────────────────────────────────────────────────────────┐
│ 🌐 example.com/path                    [200 OK]             │
│ Page Title (if available)                                    │
├─────────────────────────────────────────────────────────────┤
│ Page content text...                                         │
│ (scrollable, monospace or prose depending on content type)   │
│                                                              │
│ [Truncated — showing first N chars]  (if truncated: true)    │
└─────────────────────────────────────────────────────────────┘
```

**Header row (only when URL is available):**
- Left: favicon + `host/path` (same as search card domain treatment)
- Right: HTTP status badge — green for 2xx, yellow for 3xx, red for 4xx/5xx
- Second line: page title if present (muted, italic)

**Content area:**
- Shown in a `<pre>` block for JSON/code content types (detect by `contentType` or
  by trying `JSON.parse` on a prefix of the content)
- Shown as prose text otherwise
- Max height: `calc(100vh - 200px)` with `overflow-y: auto` — no artificial char cap;
  let the overlay scroll
- If `truncated: true`, show a muted banner at the bottom:
  `⚠ Content truncated — showing first N characters`

**Status badge:**
- `200 OK`, `404 Not Found`, `500 Error` — HTTP status text from a small map
- Omit when status is undefined
- CSS classes: `.fetch-status-ok` (2xx), `.fetch-status-redirect` (3xx),
  `.fetch-status-error` (4xx/5xx)

**Collapsed row summary (stream-parser.ts):**
Current: `🛠️ WebFetch ✓`
After: `🌐 WebFetch example.com/path ✓`

Extract from `params.url`:
```typescript
case "WebFetch":
case "web_fetch":
    try { return new URL(params.url || "").host + new URL(params.url || "").pathname; }
    catch { return params.url || ""; }
```

### 3.4 Registration

In `WebFetchResult.tsx` (side-effect, mirrors SearchResults pattern):
```typescript
registerToolRenderer({
    priority: 10,
    label: "web:fetch",
    match: byName("WebFetch", "web_fetch"),
    render: (node) => <WebFetchResult node={node} />,
});
```

Import in `ToolOverlayLog.tsx`:
```typescript
import "./tool-renderers/WebFetchResult";
```

---

## 4. Files changed

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/tool-renderers/web-fetch-result.ts` | New: `extractFetchResult`, `FetchResultData` type |
| `frontend/app/view/agent/components/tool-renderers/web-fetch-result.test.ts` | New: unit tests for extraction shapes |
| `frontend/app/view/agent/components/tool-renderers/WebFetchResult.tsx` | New: renderer component + registration |
| `frontend/app/view/agent/components/ToolOverlayLog.tsx` | Add side-effect import |
| `frontend/app/view/agent/stream-parser.ts` | Add `WebFetch`/`web_fetch` to `extractToolDetail` |
| `frontend/app/view/agent/types.ts` | Add `WebFetch`/`web_fetch` to `TOOL_ICONS` |
| `frontend/app/view/agent/styles/_document-nodes.scss` | New fetch view styles |

Also fix WebSearch while in the area:
| `frontend/app/view/agent/providers/claude-translator.ts` | Add `isTerminalShaped` guard to `canApplyStructured` — prevents terminal-style `structuredResult` from overwriting non-bash tool results (e.g. WebSearch string content) |

---

## 5. Acceptance criteria

- WebFetch with a plain-string result renders the string in a scrollable content area
- WebFetch with a structured object renders header (URL, status badge, title) + content
- Status badge shows correct color: green 2xx, yellow 3xx, red 4xx/5xx
- `truncated: true` shows the truncated banner
- JSON content displays in a `<pre>` block; prose content displays as text
- Collapsed row shows `🌐 WebFetch host/path ✓` with actual URL host
- Unknown/null result falls back to `CompactResult` (no regression)
- TypeScript compiles clean
- Tests cover: string result, structured result, null/undefined, truncated flag

---

## 6. Out of scope

- Full HTML rendering / iframe embedding
- Content diffing across fetches
- Download / copy button for full content
- Syntax highlighting beyond basic `<pre>` for JSON
