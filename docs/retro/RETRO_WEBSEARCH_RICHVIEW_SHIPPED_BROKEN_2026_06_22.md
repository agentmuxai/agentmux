# Retro: WebSearch rich result cards shipped but don't render

**Date:** 2026-06-22
**Discovered by:** Lark (agent pane live test — WebSearch result showed collapsed JSON)
**PRs involved:** #1514 (registry foundation), #1601 (WebSearch cards + Write view)
**Status:** Not yet fixed

---

## What happened

PR #1514 built the tool-result renderer registry and the `SearchResults` component.  
PR #1601 polished the card design, added a spec, and marked the feature shipped.

In practice, a live WebSearch tool call in the agent pane renders as **collapsed JSON** — the
`SearchResults` renderer fires but falls through to its `CompactResult` fallback every time.
The feature exists in the codebase and passes unit tests, but users never see it.

---

## Root causes (two independent defects)

### Defect 1 — JSON-string wrapping (primary, always fires)

`claude-translator.ts buildToolResults()` wraps the raw `block.content` string as
`{ content: "<JSON array string>" }` before forwarding it as the tool result:

```typescript
// claude-translator.ts:301
const fallback = blockContentIsString
    ? { content: block.content }   // ← wraps the array as a string under "content"
    : block.content;
```

`extractSearchResults` then receives `{ content: "[{url:..., title:...}]" }`. It finds the
`content` key (in `ARRAY_KEYS`) but tests `Array.isArray(o["content"])` — which is `false`
because it's a string, not a parsed array. It skips the `tryParseJsonArray` branch for
string values under known keys. Returns `null`. Falls back to `CompactResult`.

**The fix is one line in `search-results.ts`:** `findResultArray` already calls
`tryParseJsonArray` for the top-level value but not for values under known keys. The
latter path is already written for the array case but silently skips strings.

### Defect 2 — `canApplyStructured` overwrites array content (fires in single-result turns)

When exactly one `tool_result` block is in a message AND a sibling `tool_use_result`
object is present on the stream event, `buildToolResults` unconditionally replaces
`block.content` with the structured sibling:

```typescript
// claude-translator.ts:289-305
const canApplyStructured =
    toolResultBlocks.length === 1
    && structuredResult
    && typeof structuredResult === "object";
...
result: useStructured ? structuredResult : fallback,
```

For bash tools the sibling is `{ stdout, stderr, interrupted }` — the right shape.
For `web_search` the sibling is either absent (safe) or carries terminal-style metadata
that replaces the actual search-result array. Either way, when it fires the search results
are lost.

The guard `const blockContentIsString = ... && !isTerminalResult(structuredResult)` would
be the clean fix, but the spec's simpler alternative also works: only apply `structuredResult`
when `block.content` is already a string (stdout path).

---

## Why tests didn't catch this

`search-results.test.ts` tests `extractSearchResults` in isolation against pre-shaped
arrays and objects — it never exercises the `{ content: "<JSON string>" }` wrapper that
`buildToolResults` actually produces. The unit boundary was correct but incomplete.

`SearchResults.test.tsx` renders the component directly with pre-extracted items — it
also never passes data through `buildToolResults` or `extractSearchResults`.

No integration test covers the full path:
`CLI stream event → translator → stream parser → ToolNode → resolver → renderer → DOM`.

---

## What the spec got right

`SPEC_WEBSEARCH_RICH_VIEW_2026_06_19.md` §2.2 describes both defects accurately,
including the exact fix for Defect 1. The spec was written correctly; the implementation
just didn't finish closing the loop.

---

## Fix plan

**search-results.ts — findResultArray (Defect 1)**

```diff
- if (Array.isArray(o[k])) return o[k] as unknown[];
+ if (Array.isArray(o[k])) return o[k] as unknown[];
+ const parsed = tryParseJsonArray(o[k]);
+ if (parsed) return parsed;
```

This was already written in the spec (§3.1 option A) and in the existing code's
top-level path, but was accidentally omitted from the `ARRAY_KEYS` loop.

**claude-translator.ts — buildToolResults (Defect 2)**

```diff
  const canApplyStructured =
      toolResultBlocks.length === 1
      && structuredResult
      && typeof structuredResult === "object"
+     && blockContentIsString;  // only apply for terminal/stdout tools
```

This is already implicit — `useStructured = canApplyStructured && blockContentIsString` —
but `canApplyStructured` was documented as "can apply" when in practice the
`blockContentIsString` guard is the real gating condition. Making it explicit prevents
future callers from misreading the intent.

**New test: integration-style coverage**

Add a test that constructs a `{ content: "[{url:..., title:...}]" }` wrapper (the actual
shape from `buildToolResults`) and asserts `extractSearchResults` returns non-null items.

---

## Extension: WebFetch

`WebFetch` is the natural next renderer to add now that the registry and card pattern
are established. See `SPEC_WEBFETCH_CONTENT_VIEW_2026_06_22.md` for the design.

---

## Action items

| # | What | Owner |
|---|------|-------|
| 1 | Fix `findResultArray` — add `tryParseJsonArray` branch for key values | — |
| 2 | Fix `canApplyStructured` — add `blockContentIsString` guard | — |
| 3 | Add integration-style test for JSON-string extraction path | — |
| 4 | Ship WebFetch renderer (see companion spec) | — |
