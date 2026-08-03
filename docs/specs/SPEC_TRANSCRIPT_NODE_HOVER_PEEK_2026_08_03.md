# Spec: hover-to-peek on tool calls and thinking clumps

**Status:** Implemented (PR #2392) — tool calls + thinking clumps per the original trigger below, PLUS regular user-input messages, added in a follow-up round of the same PR per explicit user request ("we also need it for user input nodes", 2026-08-03). See §2.5's scope note and §6, updated accordingly — this is no longer scoped to exactly two node kinds. Positioning also changed from the originally-planned "reuse the Session Context Tooltip-component code" to a new shared `PeekOverlay.tsx` (Portal-rendered, top-anchored to the entry) — the virtualized transcript's per-row CSS stacking contexts made a plain in-DOM absolute overlay (what "Session Context" originally did) paint UNDER later rows instead of over them; see that file's doc comment.
**Trigger (verbatim):** "we want to introduce hover to peek on each entry in the agent pane (each tool call, and each clump of thinking) .. we already have a 'Session Context' line that shows up, can u reuse that code? for all we want the time (and time ago) and the best stats like estimate token cost and other stuff you can think of. on tool calls, we want the time and the word-wrapped tool call in the hover peek."

## 0. Read this first — this is the third pass at broadly this idea

Before designing anything, the audit turned up two prior specs that each **removed** a version of exactly this feature, plus one that designed it and was apparently superseded before shipping as drafted. Any new design has to be explicit about not repeating what got rejected.

| When | Spec | What it did | Why |
|---|---|---|---|
| 2026-04-15 | `docs/specs/node-timestamp-hover.md` | Designed a per-node hover popover: absolute-positioned pill, shown via CSS `:hover`, on every node row (`tool`, `markdown`, `section`, etc.), showing `HH:MM:SS.M`. Marked "Ready to implement." | — (superseded by the next two before/without fully shipping as drafted) |
| 2026-05-28 | `docs/specs/SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md` | **Removed** `ToolBlock`'s hover-driven behavior: a browser-native title tooltip + a full log-panel auto-expand + fast expand/collapse were all firing on hover simultaneously. Reduced to click-to-pin only, plus one narrow surviving tooltip (word-wrapped command text, read-only, no state change). | Direct user complaint, quoted in the spec: *"There should be NO expand/collapse on hover... We want the small popup... to be at the top of the larger popup, so it's just 1 popup."* Hover was reintroducing flicker/"the row dances around my cursor." |
| 2026-06-15 | `docs/specs/SPEC_REMOVE_NODE_HOVER_STRIP_2026_06_15.md` | **Deleted** `NodeHoverStrip` — the generic per-row hover pill (timestamp + expand/collapse toggle) that the April spec's model had become. Timestamp was **dropped entirely**, not relocated. | *"It's the last remnant of the legacy 'popups on hover' model... floats over line content."* |

**What survived and is proven acceptable:** `ToolBlock.tsx`'s own narrow `<Tooltip>` (`ToolBlock.tsx:297-305`) — wraps just the tool-name text, shows only the word-wrapped command/args, no expand/collapse, no other side effect, suppressed the instant the row is already expanded. This is the one hover surface nobody has complained about or removed.

**Design implication for this spec:** extend that surviving, narrow pattern — do not reintroduce a generic per-row floating strip (killed twice) or multiple overlapping hover surfaces on one node (killed once, with an explicit user quote against it). Concretely: one `<Tooltip>` per node, attached to a specific existing element (not the whole row), purely informational, no expand/collapse coupling, suppressed while the node's own panel is already open.

## 1. Audit — current state

### 1.1 The "Session Context" hover the user is pointing at

Traced to `AgentComposerStrip.tsx:57-67` (`contextTitle()`) + `:253-264` (usage) — the context-fill text (`12.1k / 64k`) in the composer strip. **This is a plain native HTML `title=` attribute**, not a custom component:

```tsx
<span class={`agent-composer-strip-ctx ${ctxClass()}`}
      title={props.contextTokens != null ? contextTitle(props.contextTokens, props.contextWindow) : undefined}>
    {ctxText()}
</span>
```

There's no "code" here to reuse beyond the idea of building a multi-line string — no positioning, animation, or Portal logic. The literal string "Session Context" doesn't appear near this UI at all (it only appears as a markdown heading in agent startup payloads and a Bundle-picker modal label — unrelated).

**The actually-reusable mechanism is `frontend/app/element/tooltip.tsx`'s `Tooltip` component** — floating-ui positioned (`computePosition` + `autoUpdate`, offset/flip/shift middleware), Portal-rendered, hover-driven with a configurable delay (default 300ms), already proven in production on `ToolBlock.tsx`. **Recommend reusing `Tooltip`, not the native-`title=` pattern** — it's animated, positions correctly near viewport edges, and is the one hover surface with a track record of not getting ripped out.

`Tooltip`'s props (`tooltip.tsx:17-28`): `children` (anchor content), `content` (arbitrary JSX, not just a string), `placement` (`"top"|"bottom"|"left"|"right"`), `forceOpen`, `disable`, `divClassName`/`divStyle`/`divOnClick`, `delayMs`.

### 1.2 Tool calls — roughly half of this already exists

`ToolBlock.tsx:270-305`:
```tsx
const cmdText = createMemo(() => extractToolDetail(props.node.tool, (props.node.params as Record<string, any>) ?? {}));
...
<Tooltip
    disable={expanded() || !cmdText()}
    delayMs={150}
    placement="bottom"
    divClassName="agent-tool-name-tooltip-anchor"
    content={<div class="agent-tool-cmd-tooltip">{cmdText()}</div>}
>
    <span class="agent-tool-name">{props.node.summary}</span>
</Tooltip>
```

This is **already** "the word-wrapped tool call in the hover peek" — it's just missing the time. `props.node.timestamp` is reliably stamped at `tool_call` time (`stream-parser.ts:461`, `toolCallToNode()`: `timestamp: Date.now()`). `props.node.duration` (provider-reported seconds) is already shown inline next to the tool name (`ToolBlock.tsx:307`) — no need to repeat it in the tooltip. No per-tool-call token/cost data exists anywhere.

### 1.3 Thinking clumps — no hover today, and the timestamp isn't even stamped yet

Consecutive `thinking` stream deltas already clump into one `MarkdownNode` (`metadata.thinking: true`) — `stream-parser.ts:385-393`, `thinkingToNode()`. "Each clump" the user describes already = one node; no new grouping logic needed.

**Gap found during this audit**: unlike `toolCallToNode()`, `thinkingToNode()` never stamps a `timestamp` at all:
```ts
private thinkingToNode(event: ThinkingEvent): DocumentNode {
    this.currentTextNode = null;
    if (!this.currentThinkingNode) {
        this.currentThinkingNode = { type: "markdown", id: this.nextIdOf("node"), content: event.content, metadata: { thinking: true } };
    } else {
        this.currentThinkingNode = { ...this.currentThinkingNode, content: this.currentThinkingNode.content + event.content };
    }
    return { ...this.currentThinkingNode };
}
```
No hover mechanism exists on `MarkdownBlock.tsx` today. A canceled/orphaned thinking clump gets a collapsed "⏹ Canceled — partial thought" header (`MarkdownBlock.tsx:97-111`) but nothing timing-related. There's no captured "end" moment for a thinking clump (it just stops accumulating when a non-thinking event arrives or the turn ends), so a duration analogous to `ToolNode.duration` isn't available directly — resolved in §4 (derive from the next node's timestamp instead of adding new stream-side stamping).

### 1.4 Token/cost data — the real constraint on "best stats"

Only turn/session-level aggregates exist:
- `TurnTokens { input: number; output: number }` — live running total for the in-flight turn.
- `SessionStats { cost_usd?, duration_ms?, input_tokens?, output_tokens?, num_turns? }` — snapshotted once per completed turn.

Both are consumed only by `AgentComposerStrip.tsx` (pane-level status strip), never attached to an individual `ToolNode` or thinking `MarkdownNode`. **There is no code path today that can report "this specific tool call" or "this specific thinking clump" cost/tokens** — `message_delta`'s output-token count is a running total for the whole turn, not attributable to one node.

Two ways forward, not mutually exclusive:
- **True per-node accounting** — snapshot the running `TurnTokens` counter at each node's creation/completion and diff. Real instrumentation change (reducer + stream-parser), non-trivial, a separate phase (§6).
- **Client-side estimate, available now, zero backend changes** — a chars÷4 heuristic (the standard rough ratio for English/code text) computed directly from content already in memory: the thinking clump's accumulated `content.length`, or a tool call's `JSON.stringify(params)` (+ result, once available) length. Must be clearly labeled as an estimate (e.g. "~340 tok (est.)"), never presented as real API-reported usage. This is what "estimate token cost" in the request most likely means, given it explicitly said "estimate."

### 1.5 Time-ago formatting — 6+ duplicate implementations, none shared

No shared utility exists; every call site copy-pastes the same `<60s→"Ns ago" / <1h→"Nm ago" / <1d→"Nh ago" / else→"Nd ago"` shape:
`MyAgentsList.tsx:65-67` (tested — `MyAgentsList.test.tsx`), `AgentLaunchModal.tsx:324-326`, `AgentDisconnectedBanner.tsx:43-47`, `swarm-view.tsx:440-443`, `usenotification.tsx:93-97`, `warden.tsx` (`formatAge`), plus two devtools-only copies. This session already established `frontend/util/format-time.ts` (PR #2386, merged) as the shared home for elapsed/relative time formatting — the natural place to add a `formatTimeAgo` here too, sourced from `MyAgentsList.tsx`'s already-tested version, rather than adding a 7th duplicate.

The unshipped `node-timestamp-hover.md` also sketched an exact-time formatter (`HH:MM:SS.M`, tenths precision) — worth reviving for the tooltip's absolute-time line, same file.

## 2. Proposed design

### 2.1 Principle

One `<Tooltip>` per node, wrapping a specific existing element (not the whole row, not a new absolute-positioned strip), read-only content, no expand/collapse coupling, suppressed while that node's own panel is already open — i.e., exactly `ToolBlock.tsx`'s existing guard shape (`disable={expanded() || !cmdText()}`), extended with richer `content=`, not a new mechanism.

### 2.2 Shared building blocks (new)

- **`frontend/util/format-time.ts`** (existing file) gains:
  - `formatTimeAgo(ms: number): string` — relative time ("3m ago", "2h ago", …), consolidating the 6 duplicates listed in §1.5.
  - `formatExactTime(ms: number): string` — `HH:MM:SS` local time (tenths optional), adapted from `node-timestamp-hover.md`'s draft.
- **`frontend/util/format-count.ts`** (existing file, from this session's earlier consolidation) gains:
  - `estimateTokenCount(text: string): number` — `Math.ceil(text.length / 4)`, paired with `formatCompactNumber` for display (e.g. `~${formatCompactNumber(estimateTokenCount(text))} tok (est.)`).

### 2.2a Presentation — large/readable, like the native tooltip looks, NOT the existing cramped command-tooltip styling

Confirmed directly (`_document-nodes.scss:1796`, `.agent-tool-cmd-tooltip`): today's ToolBlock command tooltip renders at **11px monospace, tight padding, `max-width: 480px`** — visually cramped next to the browser-native "Session Context" tooltip's larger, more generous default rendering. User feedback on seeing the existing (pre-existing, unrelated-to-this-spec) command tooltip in 0.54.8: reuse the `Tooltip` **component**'s reliable positioning/timing machinery (§1.1's recommendation stands — native `title=` still can't do Portal-level z-index, viewport-edge flipping, or rich JSX content), but give the **new, richer content** its own larger, more readable CSS treatment rather than inheriting the old narrow tooltip's cramped sizing.

Concretely: a new class (e.g. `.agent-node-peek-tooltip`, distinct from `.agent-tool-cmd-tooltip` — the old class stays as-is if anything still uses it standalone) with a noticeably larger base font size (e.g. 13-14px vs. the existing 11px), more generous padding than the current `px-2 py-1`, and a wider `max-width` if the added time/stats lines call for it. The goal is that hovering a tool call or thinking clump reads as comfortably as the native tooltip does today, not as a shrunk-down technical readout.

### 2.3 Tool call hover — extend the existing Tooltip, no new mechanism

`ToolBlock.tsx`'s existing `content=` gains a header line, prepended above the current `cmdText()`:

```
14:32:07 · 3m ago
~120 tok (est.)
Bash: npm run build --workspace=frontend ...
```

- Time: `formatExactTime(props.node.timestamp)` + `formatTimeAgo(props.node.timestamp)` (live-updating "ago" text needs a tick source — reuse the pane's existing `useTick(1000)`, already imported in most of these components, only while the tooltip is actually open to avoid needless reactivity).
- Estimate: `estimateTokenCount(JSON.stringify(props.node.params) + JSON.stringify(props.node.result ?? ""))`.
- No change to `disable`/suppression logic, no new signals, no new component.

### 2.4 Thinking-clump hover — new, mirrors 2.3 exactly

1. **Stamp a timestamp** in `thinkingToNode()` (`stream-parser.ts`) — one line, set only on first creation of the clump (the `!this.currentThinkingNode` branch), never touched on subsequent appends (a clump's "time" is when it started, not when it was last extended).
2. **Wrap the thinking block's header** in `MarkdownBlock.tsx` with the same `<Tooltip>` component, `content=` showing exact time + time-ago + `~N tok (est.)` from the clump's own accumulated `content.length`, plus a derived duration when available (§4, resolution 1 — `nextNodeTimestamp − this node's timestamp`, omitted for the trailing node).
3. Suppress while the block itself is in any expanded/interactive state, mirroring `ToolBlock`'s guard — confirm at implementation time whether `MarkdownBlock` has its own expand/collapse notion for thinking blocks to hook into (the "⏹ Canceled" header suggests some collapsed-state handling already exists; re-use its signal rather than adding a new one).

### 2.5 Explicit non-goals / guardrails (carried forward from the two rejection specs)

- **No hover-triggered expand/collapse, anywhere.** The tooltip is read-only; clicking to pin/expand remains the only way to open a node's full panel.
- **No new GENERIC per-row hover strip.** This still holds — nothing hovers on a BARE row regardless of kind. Every peek surface is anchored to one specific, already-existing, kind-aware element (tool-name span, thinking-block wrapper, user-message row), with kind-specific content (time/estimate/command for tools, time/estimate for thinking, time/estimate/+full-body-on-hover for user messages). `PeekOverlay.tsx` is a SHARED RENDERING MECHANISM (Portal + top-anchored positioning) reused across those three specific surfaces, not a single generic overlay applied uniformly to every row — that distinction is the one this bullet exists to protect.
- **No fabricated exact token/cost.** Anything token/cost-shaped is a clearly-labeled estimate (`~`, `(est.)`) until/unless Phase 2 (§6) instrumentation lands.
- ~~Scoped to exactly two node kinds~~ — **superseded** (2026-08-03 follow-up): extended to regular user-input messages per explicit user request ("we also need it for user input nodes"). Section headers and other node kinds remain out of scope; see §6.

## 3. Files touched (this phase)

| File | Change |
|---|---|
| `frontend/util/format-time.ts` | Add `formatTimeAgo`, `formatExactTime`; migrate `MyAgentsList.tsx` to the shared one (bonus DRY win, matches this session's established pattern) |
| `frontend/util/format-count.ts` | Add `estimateTokenCount` |
| `frontend/app/view/agent/stream-parser.ts` | Stamp `timestamp` in `thinkingToNode()` on first creation only |
| `frontend/app/view/agent/types.ts` | No type change needed — `MarkdownNode.timestamp?: number` already exists in the union, just unset for thinking today |
| `frontend/app/view/agent/components/ToolBlock.tsx` | Extend existing `Tooltip`'s `content=` with time + estimate lines, switch content wrapper to the new larger-format class (§2.2a) |
| `frontend/app/view/agent/components/MarkdownBlock.tsx` | Wrap thinking-block header in a new `<Tooltip>`, same content shape, same new class |
| `frontend/app/view/agent/styles/_document-nodes.scss` | New `.agent-node-peek-tooltip` rule (§2.2a) — larger font/padding than the existing `.agent-tool-cmd-tooltip` |

## 4. Open questions — resolved

1. **Thinking-clump duration** — **resolved: derive it client-side at render time, no stream-parser/backend change.** Rather than skip it or add new stream-side stamping, compute an approximate duration as `(next sibling node's timestamp − this node's timestamp)`, where "next sibling" is only knowable at the level that already holds the full ordered node array (the virtualization list / `AgentDocumentView`, not `MarkdownBlock` itself, which only receives its own node today) — so this needs one small prop threaded down (`nextNodeTimestamp?: number`), not a stream-parser change. Omit the duration line entirely when this is the trailing node (turn still streaming, no successor yet) or when the next node has no timestamp, rather than showing a misleading figure.
2. **Token estimate heuristic** — **resolved: ship the chars÷4 estimate for v1**, clearly labeled "(est.)". It's cheap (no new architecture), gives immediate value, and — per `SPEC_PER_NODE_TOKEN_ACCOUNTING_2026_08_03.md` (written as a direct follow-up to this question) — doubles as the fallback path for every case where real per-node accounting isn't derivable (non-Claude providers, compaction-straddled rounds, history-replayed nodes). One estimate mechanism serves both specs; see that doc's §6.2 for the same resolution stated from the other side.
3. **Composer strip consistency** — **resolved: skip.** Not requested, adds surface area beyond the ask. Revisit only if it comes up on its own.

## 5. Verification plan (once implemented)

- Hover a completed tool row: tooltip shows time/ago above the existing word-wrapped command text; suppressed once the row is expanded/pinned (existing guard, unchanged).
- Hover a thinking clump (mid-turn and after turn end): tooltip shows time/ago + estimate; no expand/collapse triggered by the hover itself.
- Hover rapidly across several rows: no flicker, no overlapping tooltips (delay/suppression already proven on `ToolBlock`'s existing usage).
- `grep -rn "NodeHoverStrip"` still returns nothing — confirms this doesn't quietly resurrect the deleted component.

## 6. Out of scope / follow-up phases

- **True per-node token/cost** via diffing `TurnTokens` at node boundaries — real instrumentation change, own phase, only worth it if the chars÷4 estimate (§2.4/§4.2) turns out to be unsatisfying in practice.
- ~~Extending hover-peek to other node kinds (regular assistant text, user messages, sections, shell blocks) — not requested~~ — **user messages done** (2026-08-03 follow-up, explicit request). Regular assistant text, section headers, and shell blocks remain out of scope; extending to those would still need its own pass through the same "does this repeat a killed pattern" lens.
- **Upgrading `AgentComposerStrip`'s tooltip** to the shared `Tooltip` component (§4.3) — nice-to-have, not core.
