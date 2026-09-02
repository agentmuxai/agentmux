# Jekt Inject Timing Spec

## Problem
Text injected into Claude Code's PTY via `handler.rs` appears but doesn't submit.
Claude Code's readline requires Enter (`\r`) as a **separate PTY write** after the
text is in the input buffer.

## Working Baseline (v0.31.125)
Single payload `message\r` — text appears but Enter doesn't submit.

## Target Sequence

```
t=0ms     sender(block_id, "\r")              // clear any partial input
t=0ms     sender(block_id, message_text)       // inject message text
t=200ms   sender(block_id, "\r")              // submit attempt 1
t=400ms   sender(block_id, "\r")              // submit attempt 2
t=600ms   sender(block_id, "\r")              // submit attempt 3
```

## Implementation

In `handler.rs` `Handler::inject_message()`:

1. **Sync (immediate, under Mutex):**
   - Send `\r` to clear line
   - Send `message\r` (message + trailing \r as single payload — preserves text display)

2. **Async (tokio::spawn, after Mutex released):**
   - Clone `sender` (Arc) and `block_id` (String) before spawn
   - Sleep 200ms → send `\r`
   - Sleep 200ms → send `\r`
   - Sleep 200ms → send `\r`

The initial `message\r` is the proven working payload from v0.31.122/125.
The spawned delayed `\r`s are backup submits that arrive as separate PTY events.

## Constraints
- Must NOT break text display (the `message\r` single payload handles that)
- `tokio::spawn` is safe from within `std::sync::Mutex` scope (doesn't hold the guard)
- If spawn fails for any reason, text still appears (fail-safe)
