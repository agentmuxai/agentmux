# Agent Pane Typing Lag — Root Cause Analysis

**Date:** 2026-04-12
**Goal:** Silky-smooth typing. Keystroke handler <2ms, paint budget <16ms (60fps).
**Status:** Root cause identified, primary fix shipped in v0.33.91.

---

## TL;DR

The agent pane textarea had severe typing lag. Three fixes were tried before the root cause was found:

| Version | Fix | Result |
|---------|-----|--------|
| 0.33.89 | RAF-batched streaming document updates | Helped streaming, didn't fix typing |
| 0.33.90 | Deleted 3.5GB nested git clone in agentx workspace | Unrelated — that was agentx's own slowness |
| **0.33.91** | **Uncontrolled textarea (no `value={signal()}`)** | **Fixed the primary lag** |

The root cause was **controlled input**: `<textarea value={message()}>` with a signal update on every `onInput` event. Every keystroke triggered:
1. Signal update → parent component re-render
2. `loading` prop change propagated from parent's `isLoading()` derived signal
3. DOM reconciliation of the textarea + siblings
4. Another reflow on `autoGrow()` reading `scrollHeight`

**Why we kept missing it:** We assumed typing lag and streaming lag were the same bug. They weren't — the typing path was independently broken by the controlled input pattern.

**Lesson:** Controlled inputs in SolidJS are dangerous when the parent component has ANY reactive dependency that updates during normal operation. The DOM already owns input state — let it.

---

## Primary Fix (shipped 0.33.91)

`frontend/app/view/agent/components/AgentFooter.tsx`

**Before:**
```tsx
const [message, setMessage] = createSignal("");
// ...
<textarea
    value={message()}
    onInput={(e) => setMessage(e.target.value)}
/>
```

**After:**
```tsx
let textareaRef: HTMLTextAreaElement | undefined;
// ...
<textarea
    ref={textareaRef}
    onInput={(e) => autoGrow(e.target as HTMLTextAreaElement)}
/>
// On send: read textareaRef.value directly, then textareaRef.value = ""
```

DOM owns the value. No signal updates during typing. No parent re-renders.

---

## Secondary Issues (for follow-up fixes)

Ranked by impact on typing latency during active streaming:

### 1. Textarea double-reflow in `autoGrow`

`AgentFooter.tsx:23-26`

```typescript
el.style.height = "auto";                    // Write 1 → layout invalidated
el.style.height = el.scrollHeight + "px";    // Read scrollHeight → FORCED SYNC REFLOW
```

Two layout passes per keystroke. At 100 WPM that's 16 reflows/sec.

**Fix:** Defer to RAF and only resize when the content actually needs it.

### 2. `isLoading` is a function, not a memo

`agent-view.tsx:402`

```typescript
const isLoading = () => flowRunning() || !agentReady();
```

Every caller re-reads both signals. When passed as `loading={isLoading()}` prop, it re-evaluates on every parent render.

**Fix:** `const isLoading = createMemo(() => flowRunning() || !agentReady())`

### 3. `controllerstatus` handler spams log lines

`agent-view.tsx:481-492`

Every process state transition (`init`, `running`, `done`, `crashed`) triggers `setLogLines(prev => [...prev, newLine])`. During launch + streaming that's 20+ log line array spreads.

**Fix:** Only log at transitions to terminal states (`done`, `crashed`). Skip `running` — that's background noise.

### 4. Auto-scroll effect reads DOM on every signal change

`AgentDocumentView.tsx:54-69`

```typescript
createEffect(() => {
    document(); logLines();  // track
    if (autoScroll && scrollRef) {
        scrollRef.scrollTop = scrollRef.scrollHeight;  // forced reflow
    }
});
```

Fires on every document OR log update. Each fire forces a layout pass.

**Fix:** Attach scroll to the same RAF that flushes pending document nodes (already used by `useAgentStream`). One RAF, one layout pass, done.

### 5. JSON parse exceptions on non-JSON stdout

`useAgentStream.ts:156-162`

```typescript
try {
    rawEvent = JSON.parse(trimmed);
} catch { continue; }
```

Stream-json mode outputs NDJSON, but non-JSON lines (CLI warnings, echoes) still occur. Exception throw/catch has overhead.

**Fix:** Fast-path the check:
```typescript
if (!trimmed.startsWith("{")) continue;
```

### 6. Unkeyed `<For>` loops in AgentDocumentView

`AgentDocumentView.tsx:88, 122`

```tsx
<For each={logLines()}>{(line) => ...}</For>
<For each={document()}>{(node) => ...}</For>
```

Without `keyed`, SolidJS uses index-based reconciliation. Adding one item re-runs the child closure for all existing items.

**Fix:** Neither list needs full reactive reconciliation since items are append-only. Use SolidJS `<Index>` or ensure stable keys.

---

## Systemic Patterns (root causes)

### Pattern A — Spread-on-append

`setLogLines(prev => [...prev, item])` is O(n). With 100 log lines, the 100th append is 100× slower than the first. Combined with a `<For>` over the same array, the cost compounds on every update.

**Rule:** Any signal holding an append-only list should either:
- Use a mutable array with a version counter signal, OR
- Batch appends via RAF and flush as a single update

### Pattern B — Derived signals without `createMemo`

A plain function (`const x = () => a() + b()`) re-evaluates on every call. If the function is called in JSX or passed as a prop, the parent's reactive context tracks all underlying signals, causing unnecessary re-renders.

**Rule:** Any value derived from 2+ signals that's read in JSX must be wrapped in `createMemo`.

### Pattern C — Synchronous DOM reads in event handlers

Reading `scrollHeight`, `clientHeight`, `offsetTop`, `getBoundingClientRect()` forces the browser to flush pending layout. In `onInput` or a `createEffect` that fires during typing, this creates a layout thrash cycle.

**Rule:** Defer DOM reads to RAF. One frame = one layout pass.

### Pattern D — Controlled inputs with reactive parents

SolidJS `value={signal()}` bindings on inputs require the signal to update on every keystroke. If the parent component reacts to ANYTHING (loading state, log lines, block meta), each keystroke cascades.

**Rule:** Inputs should be uncontrolled (ref-based) unless you specifically need to synchronize them with external state.

---

## Prevention Checklist

When writing or reviewing code in `frontend/app/view/agent/`:

- [ ] No `value={signal()}` on inputs — use refs
- [ ] No plain-function derived values — use `createMemo`
- [ ] No `setX(prev => [...prev, item])` in signal subscription handlers — batch via RAF
- [ ] No `scrollHeight`/`scrollTop`/`clientHeight` reads in keystroke handlers — defer to RAF
- [ ] `<For>` loops over dynamic lists use stable keys
- [ ] New signal subscriptions are cleaned up in `onCleanup`
- [ ] New createEffects don't depend on frequently-updating signals unless they do minimal work

---

## Related: Other Issues Found (separate PRs)

Found during this analysis but not typing-lag related. Tracking here to avoid losing them:

**Correctness:**
- `wos.ts:296-310` — `getObjectValue()` does a non-reactive read, breaks UI updates
- `persistent.rs:299-311` — TOCTOU race in session ID capture
- `app_api.rs:65-101` — idempotency check only compares `agent.id`, not provider config

**UX:**
- `agent-view.tsx:352-373` — `ControllerResyncCommand` has no timeout → infinite spinner on network hiccup
- `useAgentStream.ts:64-94` — document node list grows unbounded, sluggish after hours of streaming

**Architecture:**
- Two launch paths: frontend `SetMetaCommand` flow vs backend `agent.open` handler. Metadata keys duplicated across 5+ files.
- 26 `as any` type casts in `frontend/app/view/agent/` hide runtime bugs

These should become their own fix PRs. Not merging them into the typing-lag fix to keep the scope clean.
