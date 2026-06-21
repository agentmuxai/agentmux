# SPEC: Context Compaction Notification

**Date:** 2026-06-20
**Issue:** #1553
**Status:** Implementation

---

## Problem

When Claude Code auto-compacts the conversation context, earlier history is
summarized and the context window resets. The context bar already drops (handled
by #1543), but there is a silent transcript discontinuity — no visible marker
explains why the agent's apparent memory reset.

## AgentMux /clear vs Compaction

`/clear` in AgentMux is **frontend-only** (`commands/global/clear.ts`). It
dispatches `UserClear` to the document store and empties the visible transcript.
The Claude Code process and its conversation history are untouched. No token drop
occurs. No marker is needed.

Compaction — triggered by Claude's native `/compact` slash command or by
auto-compact at ~95% context fill — is a real context reset. A marker IS needed.

## Detection

Claude Code emits a `{"type":"system","subtype":"compact_boundary",...}` event in
stream-json mode, but the schema is undocumented and the event has been observed
missing in some versions (GitHub #63015 regression). The heuristic is therefore
the primary detection path, with the wire signal as an optional enhancement.

**Heuristic (primary):** In the reducer's `TokensIn` arm, compare against the
previous `lastContextTokens`:

```
prevTokens > 10_000 AND newTokens < prevTokens × 0.5
```

- Compaction drops 80–95% — well above the 50% threshold
- Normal turn-to-turn growth is monotonically increasing — no false positives
- AgentMux `/clear` does not touch tokens — no false positives
- `prevTokens > 10_000` guards against noise at session start

**Wire signal (future enhancement):** Handle `type:"system"` in
`ClaudeTranslator` — if `subtype === "compact_boundary"`, emit a
`context_compacted` StreamEvent directly. More precise but depends on
undocumented/unstable schema.

## Surface

A transcript marker (divider + pill) rendered as a `ContextCompactedNode`:

```
─────────────── context compacted ───────────────
        Earlier history summarized · 847k → 52k tokens
```

- Not a toast — the marker is most valuable when the user scrolls back to ask
  "why did the agent forget X?"
- Persisted in the agent document (visible on reload and in exports)
- Token counts shown as rounded integers (e.g. `847k → 52k`)

## Files

| File | Change |
|------|--------|
| `frontend/app/view/agent/types.ts` | Add `ContextCompactedNode` interface; add to `DocumentNode` union |
| `frontend/app/store/agent-pane-state/types.ts` | Add `context-compacted` to `AgentPaneEvent` |
| `frontend/app/store/agent-pane-state/reducer.ts` | Detect drop in `TokensIn` arm; emit `context-compacted` event |
| `frontend/app/view/agent/useAgentStream.ts` | Handle `context-compacted` event; append synthetic node |
| `frontend/app/view/agent/virtualization/renderers.ts` | Register + size `context_compacted` node |
| `frontend/app/view/agent/components/AgentDocumentView.tsx` | Render `ContextCompactedNode` |
| `frontend/app/view/agent/styles/_document-nodes.scss` | Divider + pill styles |

## Acceptance Criteria

- [ ] After compaction (context drops ≥50% from >10k tokens), marker appears at the boundary
- [ ] Marker shows pre- and post-compaction token counts in rounded form
- [ ] Marker persists on reload
- [ ] No marker appears for normal turn-to-turn context growth
- [ ] No marker appears when AgentMux `/clear` is used
- [ ] Context bar continues to drop/recover correctly (no regression from #1543)

## Out of Scope

- Wire signal (`compact_boundary`) handling — follow-up
- `autoCompactEnabled` / `compactPrompt` settings UI
- Toast in addition to transcript marker
