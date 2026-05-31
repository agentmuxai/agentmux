# Analysis & Plan: Agent-Pane Typing Latency (Streaming-Markdown O(n²))

**Date:** 2026-05-30
**Author:** AgentA
**Status:** Root cause confirmed (code + measured data). Fix not yet implemented — plan below.
**Priority:** 🔴 Critical (user-flagged: "highest priority… it is critical").

---

> **Update (post-review, 2026-05-31):** the §6 "per-block incremental render"
> plan was implemented and **rejected on review** — splitting at blank lines
> breaks lists (loose lists, multi-paragraph items), paragraph spacing, and
> cross-block reference definitions (reagent P1s + codex P2s on #1213).
> The shipped fix keeps each message a **single parse** and instead
> **throttles** streaming re-renders (~1 per 90 ms) while **deferring syntax
> highlighting** until the stream settles (`MarkdownBlock.tsx` + a reactive
> `highlight` prop on `Markdown`). The root-cause analysis below stands; only
> the fix shape changed.

## 1. Symptom & measured data

Typing in the agent-pane composer lags badly **while an agent response is streaming**. Measured on the clean v0.40.3 instance via the existing keystroke perf marks (#1146) + the long-task observer:

```
agent-input-raf   308ms → 1241ms → 3184ms      ← keystroke RAF enqueue→fire latency (should be ~16ms)
long-task         52 → 68 → 118 → 263 → 465 → 850 → 1507 → 1754 ms   ← main-thread blocks, roughly DOUBLING
```

`agent-input-raf` measures how long the keystroke's `requestAnimationFrame` waits to fire. 3.2s means **the main thread was blocked** for 3.2s. The escalating, ~doubling long-tasks are the blocker — a **runaway**. The composer is not slow; it is **starved**.

---

## 2. Root cause

`frontend/app/element/markdown.tsx:466-540` — the `renderedMarkdown` `createMemo` re-runs the **entire** pipeline from scratch on every change of its text input:

```js
const renderedMarkdown = createMemo(() => {
    const txt = transformedText();              // = props.text, grows every streamed frame
    const processor = unified()
        .use(remarkParse).use(remarkPlugins)    // processor + plugin arrays REBUILT every run
        .use(remarkRehype).use(rehypePlugins);  // incl. rehypeHighlight, rehypeRaw, rehypeSanitize
    const mdast = processor.parse(txt);          // full re-parse of the WHOLE message
    const hast  = processor.runSync(mdast);      // full re-highlight + re-sanitize of EVERYTHING
    return toJsxRuntime(hast, …);                // full rebuild of the entire Solid subtree
});
```

`MarkdownBlock` (`frontend/app/view/agent/components/MarkdownBlock.tsx:42`) feeds it `props.node.content`, which the streaming reducer **replaces with a longer string on every frame** (~60×/s, RAF-batched). So a streaming response re-parses + re-syntax-highlights + rebuilds the **whole growing message every frame** → **O(n²)** over the turn. The cost climbs with message length → the escalating long-tasks → keystroke RAFs can't fire → typing lags. It compounds the longer/larger the response.

---

## 3. Ruled out (so we don't chase ghosts)

- **The composer** — uncontrolled `<textarea>`, coalesced RAF, **no signal/atom/dispatch per keystroke**. Optimal already (the #1146 "ultra-silky input" work). It's starved, not slow.
- **Reactive cascade / re-subscription leak** — `recordDispatch` is `untrack`-wrapped and not even on the typing path; typing does not dispatch to the reducer or write any draft atom.
- **sysinfo channel (#1142 cascade-freeze)** — not wired into the agent pane; the ~1s `sysinfo` publish does not touch the composer or conversation.
- **The virtualizer / `partition()` memo** — reads only `nodes()`, not any input/draft signal.

---

## 4. Cost anatomy (to confirm with the diagnostic)

A diagnostic is already in place (uncommitted): `markStart/markEnd("agent-markdown-parse")` around the memo — logs parses >100ms to the host log (`muxlog host '\[perf\]'`). Run one streaming-with-code repro to get the split, but the suspected ranking is:

1. **`rehypeHighlight`** (highlight.js over *all* code blocks, every frame) — typically the heaviest for code-heavy output (a coding agent).
2. **`toJsxRuntime`** full subtree rebuild → then **SolidJS reconciles** the whole growing tree every frame (this is also a prime suspect for the `replaceChild` crash — see §10).
3. `processor.parse` + `remarkRehype` + `rehypeSanitize` over the full text.
4. Rebuilding the `unified()` processor + plugin arrays every run (smaller, but free to fix).

The fix must reduce **both** the per-frame **frequency** and the per-parse **cost** for the long-message case.

---

## 5. Fix options

| # | Approach | Fixes O(n²)? | No-timers? | Risk | Notes |
|---|----------|--------------|-----------|------|-------|
| A | **Per-block incremental render** (split into blocks; only the last/growing block re-parses; completed blocks cached as mounted components) | ✅ per-frame → O(last block) | ✅ deterministic | Med (shared `markdown.tsx`) | The proper fix. Recommended. |
| B | Render-throttle (`setTimeout`, ~10×/s) | ⚠️ frequency only; a single long parse still blocks | ❌ timer | Low | Fast, but partial + violates no-timers rule. |
| C | Defer `rehypeHighlight` until block complete | ⚠️ removes the heaviest plugin from the hot path; parse/build still per-frame | ✅ | Low-med | Strong complement to A for in-progress code blocks. |
| D | `streamdown` package | ✅ | ✅ | High | Designed for this, but React-oriented; large refactor. Not now. |

**No-timers consideration:** the user's standing rule is "never add sleep/setTimeout/RAF batching to work around a problem; find the deterministic signal instead." Option A is fully deterministic (keyed on content/block structure, no clock), so it honors the rule **and** is the better fix. Option B is explicitly avoided.

---

## 6. Recommended plan

**Phase 1 — Per-block incremental render (the core fix).** Split the message into top-level markdown blocks and render each via its own `<Markdown>`, so completed blocks stay parsed/mounted and only the **last, growing** block re-parses each frame. Per-frame cost drops from O(message) to O(last block). Deterministic, no timer.

**Phase 2 — Cheap in-progress block.** While the **last** block is still growing (esp. a large/code block with no internal top-level break), render it **without `rehypeHighlight`** (plain/structural only); apply full highlight once it's complete (i.e., once it's no longer the last block). Kills the giant-streaming-code-block worst case.

**Phase 3 — Cleanup (free wins).** Hoist the `unified()` processor + plugin arrays out of the memo (build once per component, reuse) — removes per-frame processor construction.

Phase 1 alone should restore silky typing for the common case; Phase 2 covers the long-single-code-block case; Phase 3 is a small constant-factor win.

---

## 7. Implementation detail (Phase 1)

Scope the change to the agent pane: do the splitting in **`MarkdownBlock.tsx`** (not the shared `Markdown` internals), so other `Markdown` consumers are untouched.

```ts
// Split at top-level paragraph breaks that are NOT inside a ``` fence, so we
// never cut a code block (which would render a broken open fence). O(n) scan
// of cheap char comparisons — negligible vs a parse.
function splitTopLevelBlocks(content: string): string[] {
    const blocks: string[] = [];
    let fenceOpen = false, start = 0, i = 0;
    while (i < content.length) {
        if (content.startsWith("```", i)) { fenceOpen = !fenceOpen; i += 3; continue; }
        if (!fenceOpen && content.startsWith("\n\n", i)) {
            blocks.push(content.slice(start, i)); // a completed block
            i += 2; start = i; continue;
        }
        i++;
    }
    blocks.push(content.slice(start)); // the last / in-progress block
    return blocks;
}
```

```tsx
// In MarkdownBlock, replace `<Markdown text={props.node.content} />` with:
const blocks = createMemo(() => splitTopLevelBlocks(props.node.content));
// <Index> keys by POSITION: completed positions get the SAME string each frame
// → their inner Markdown memo (=== on text) skips re-parse. Only the last
// position (growing) + any new position re-parse. (NOT <For>: avoids
// duplicate-block-string key collisions.)
return (
    <div class="agent-markdown-block">
        <Index each={blocks()}>
            {(block) => <Markdown text={block()} scrollable={false} />}
        </Index>
    </div>
);
```

**Why it works:** the inner `renderedMarkdown` memo is keyed on its `text` prop (`resolvedText = createMemo(() => props.text)`, default `===`). A completed block's string is identical frame-to-frame → that block's memo does **not** re-run. Only the last (growing) block changes → only it re-parses. Per-frame cost = O(last block).

**Integration concerns to verify:**
- Pass `scrollable={false}` (agent messages flow in the conversation; avoid per-block `OverlayScrollbars`).
- Two+ stacked `.markdown` containers: check inter-block spacing/margins (`markdown.scss`) — may need a wrapper rule so blocks join seamlessly.
- Preserve the **canceled-thinking** branch in `MarkdownBlock` (it renders `<Markdown text={props.node.content} />` collapsed — that path can stay whole; only the live `fallback` branch needs the split).
- `props.node` access must stay non-destructured (streaming replaces the reference — the existing comment at `MarkdownBlock.tsx:18-23` warns about this).

**Phase 2 detail:** mark the last block as in-progress (it's the last element of `blocks()` while a turn streams). Render it with a reduced pipeline (`rehype={false}` or a highlight-less plugin set) until it's no longer last → then it re-renders fully highlighted exactly once.

---

## 8. Verification

1. **Before:** with the `agent-markdown-parse` diagnostic, repro a long streaming response **with code blocks** + type during it → expect `[perf] agent-markdown-parse` climbing into hundreds/thousands of ms, lockstep with `agent-input-raf` keystroke lag.
2. **After:** same repro → `agent-markdown-parse` stays small + flat (only the last block); `agent-input-raf` stays ~16ms; typing is silky.
3. Confirm completed blocks render identically to today (no formatting/spacing regressions); confirm code highlighting is correct once blocks complete.
4. Confirm the canceled-thinking and non-streaming (history) paths are unchanged.

---

## 9. Risk & scope

- Touches `MarkdownBlock.tsx` (agent pane only) — **not** the shared `Markdown` internals → blast radius limited to agent messages.
- The split scan is O(n)/frame of cheap char ops (microseconds) vs. the parse it replaces (milliseconds) — net huge win.
- Edge cases: nested/indented fences, inline triple-backticks — the depth heuristic may mis-split rarely; it self-heals as the block completes (worst case a transient odd render, never a crash).
- Decide the fate of the `agent-markdown-parse` diagnostic: **keep** (cheap, useful) or revert before the PR.

---

## 10. Connection to the `replaceChild` crash

`toJsxRuntime` rebuilds the **entire** Solid subtree of the message every frame, which SolidJS must then **reconcile** against the previous tree — enormous per-frame reconcile churn inside the streaming rows. This is very likely a major contributor to the residual `replaceChild`/`reconcileArrays` crash (the one #1101's sticky frontier does **not** cover). **Phase 1 collapses that churn to O(last block)** — so this fix is expected to reduce or eliminate the streaming crash as a side effect. Validate by watching the Monitor after the fix lands (on a clean instance, no shared-component HMR).

---

## 11. Next action

Implement **Phase 1** in `MarkdownBlock.tsx` (per §7), verify per §8, then Phase 2 for the long-code-block case. Land as `perf(agent): incremental streaming-markdown render` with a changeset.
