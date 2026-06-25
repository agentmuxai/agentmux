# Retro: Haiku per-turn activity summary never appears in pane header

**Date:** 2026-06-24  
**Feature:** Per-turn live mini-summary in the agent pane top header  
**Status of feature:** Fully wired frontend + backend, but silently broken — header always blank

---

## What was supposed to happen

On every completed agent turn, a short Haiku-generated phrase (≤12 words) describing
the work just done should appear in the agent pane header — populated by the
`term:activity` block meta key. The header text is read by `viewText()` in
`agent-model.ts`. On the next turn's `Submitting` phase the header clears; on `Done`
it is repopulated.

Pipeline:
```
TurnPhase.Done
  → useAgentActivitySummary (frontend hook)
    → AgentActivitySummaryCommand RPC
      → register_session_activity_summary (app_api.rs)
        → reads recent output lines
        → invoke_cli_for_activity (Haiku, 15s timeout)
        → returns summary text
      → frontend writes "term:activity" = summary
        → viewText() picks it up → header shows phrase
```

## What actually happens

The pane header is always blank. The feature silently short-circuits at the
backend before Haiku is even invoked.

---

## Root cause

`register_session_activity_summary` (`app_api.rs:2152`) reads its input data from the
**WPS ring buffer only**:

```rust
let events = broker.read_event_history(
    crate::backend::wps::EVENT_BLOCK_FILE,
    &scope,
    50,   // "last 50 block file events"
);
```

But `EVENT_BLOCK_FILE` events are published with `persist: 0`:

```rust
// shell.rs:1381 (handle_append_block_file)
let event = wps::WaveEvent {
    event: wps::EVENT_BLOCK_FILE.to_string(),
    persist: 0,   // <-- transient: never stored in ring buffer
    ...
};
broker.publish(event);
```

`Broker::publish` only stores events in `persist_map` when `event.persist > 0`
(wps.rs:330). With `persist: 0`, the event is delivered to active live subscribers but
never retained. So `read_event_history` on `EVENT_BLOCK_FILE` always returns `[]`.

Consequence chain:
1. `all_lines` is empty → `window.is_empty()` is true
2. Early return `ActivitySummaryResult { summary: String::new() }`
3. Frontend: `result.summary` is `""` → falsy → `if (result.summary)` is false
4. `UpdateObjectMeta("term:activity")` is never called
5. Pane header stays blank

Haiku is **never invoked**. The 15-second timeout is never reached. There is no
logged error — the empty-window branch returns `Ok(Some(...))` with an empty string,
so the frontend `.catch()` handler never fires either.

---

## Why the session digest works but this doesn't

`register_session_digest` (`app_api.rs:2010`) reads from the **FileStore** first:

```rust
match filestore.stat(&cmd.block_id, "output") {
    Ok(Some(ref wf)) if wf.size > 0 => {
        filestore.read_file(&cmd.block_id, "output")  // ← actual persistent content
    }
    ...
}
// WPS ring buffer is only a fallback (which also returns empty, but digest never reaches it)
```

The FileStore contains the full accumulated agent output regardless of `persist` values.
`register_session_activity_summary` was written following the ring-buffer-only pattern
that appears in earlier iterations of the digest code, without the FileStore read that
the final digest implementation added. It is a copy-paste regression from a stale code
path.

---

## Why it ended up in the session digest instead

The `SessionDigestBanner` shows `session:digest_summary` — a different key than
`term:activity`. Both features write to different places; neither overwrites the other.
The session digest was not a fallback for the broken per-turn summary. The observation
that "it moved to the session digest" was the digest independently working, not a
mis-routing of the per-turn summary.

The per-turn summary (`term:activity`) was simply never showing because it was never
being written.

---

## Fix

Change `register_session_activity_summary` to read from the FileStore (tail of the
`"output"` file), identical to how `register_session_digest` does it. The activity
summary only needs the last ~30 lines, so a tail read is sufficient.

```rust
// In register_session_activity_summary, replace the ring-buffer read with:
let all_lines: Vec<String> = {
    match filestore.read_file(&cmd.block_id, "output") {
        Ok(Some(bytes)) => {
            let text = String::from_utf8_lossy(&bytes);
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string())
                .collect()
        }
        _ => Vec::new(),
    }
};
// then take last 30 lines as before
```

This requires threading `filestore` into the handler (same as `register_session_digest`
already does).

---

## Files involved

| File | Role |
|---|---|
| `agentmux-srv/src/server/app_api.rs:2152` | Bug is here — `register_session_activity_summary` reads ring buffer instead of FileStore |
| `agentmux-srv/src/server/app_api.rs:2010` | Reference implementation — `register_session_digest` reads FileStore correctly |
| `agentmux-srv/src/backend/wps.rs:28,326` | `EVENT_BLOCK_FILE` constant; `publish()` only persists when `persist > 0` |
| `agentmux-srv/src/backend/blockcontroller/shell.rs:1381` | Block file events published with `persist: 0` |
| `frontend/app/view/agent/hooks/useAgentActivitySummary.ts` | Frontend hook — correct, waiting on a non-empty summary from the RPC |
| `frontend/app/view/agent/agent-model.ts:115` | `viewText()` reads `term:activity` — correct, never reached |

---

## What to watch for in code review

Any new RPC that reads recent agent output via `read_event_history(EVENT_BLOCK_FILE, ...)`
will have the same bug. Block file events are intentionally transient (`persist: 0`) —
they fan out to live WebSocket subscribers but are never buffered. **Agent output must
always be read from the FileStore**, not the broker ring buffer.
