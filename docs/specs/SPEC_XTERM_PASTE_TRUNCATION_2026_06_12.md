# SPEC: xterm Terminal Paste Truncation Fix

**Date:** 2026-06-12  
**Status:** Proposed (implemented — see note below)
**Area:** Terminal (xterm pane)  
**Severity:** P1 — data loss, silent (fixed)

> **2026-08-07 audit note:** Verified fixed, all three fixes shipped —
> confirmed by reading the current code directly, not just this Status
> field, given the P1/silent-data-loss severity: (1) the input channel is
> now `mpsc::unbounded_channel()` in `shell/controller.rs`, explicitly
> documented as closing this exact truncation bug; (2) frontend chunked
> paste at the same 4KB/5ms values this spec proposed
> (`termViewModel.ts` `PASTE_CHUNK_BYTES`/`PASTE_CHUNK_DELAY_MS`); (3)
> bracketed paste mode now defaults to `true` (`term.tsx`,
> `termBPMAtom() ?? true`). No live bug. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.

---

## Problem

Pasting a large block of text into an AgentMux xterm terminal pane silently truncates the
content. The user sees only the first fraction of what they pasted with no error or warning.
No user-visible feedback is given; the truncation is only visible as a warning in backend
logs (`"input reorder buffer full, dropping"`).

---

## Root Cause

The truncation happens at the backend input channel. The full pipeline is:

```
Clipboard (navigator.clipboard.readText)
  → terminal.paste(text)           [xterm.js paste()]
  → onData event(s)                [xterm may emit multiple chunks]
  → sendDataHandler(data)          [termwrap.ts:450]
  → base64-encode + sendWSCommand  [termViewModel.ts:343-351]
  → WebSocket "blockinput" frame   [one frame per onData chunk]
  → shell.rs send_input()          [websocket.rs:428-445]
  → mpsc::Sender<BlockInputUnion>  [shell.rs:565-589]
  → PTY write loop write_all()     [shell.rs:~790-810]
```

### Critical bottleneck: `SHELL_INPUT_CH_SIZE = 32` + `try_send()`

**`agentmux-srv/src/backend/blockcontroller/shell.rs:44`**
```rust
const SHELL_INPUT_CH_SIZE: usize = 32;
```

The input channel is a bounded `mpsc` with capacity 32. Input is enqueued with
`try_send()` — non-blocking. When the channel is full, `try_send()` returns
`Err(Full)`, the message is **silently dropped**, and a warning is logged to the
backend log only. The user sees nothing.

For a large paste, xterm.js emits multiple `onData` events. Each event generates
one WebSocket message and one `try_send()` call. The PTY write loop is sequential
(one message per loop iteration, OS-limited write bandwidth). If messages arrive
faster than the PTY can consume them, the 32-slot queue fills and later chunks
are dropped — silently truncating the paste.

### Secondary bottleneck: xterm bracket-paste mode disabled by default

**`frontend/app/view/term/term.tsx:160-162`**
```typescript
const termAllowBPM = termBPMAtom() ?? false;
// ...
ignoreBracketedPasteMode: !termAllowBPM,
```

Bracketed paste mode is disabled by default. When enabled, the shell receives the
entire paste wrapped in `\x1b[200~`…`\x1b[201~` and can handle it atomically.
When disabled, the shell sees raw keystrokes and may process each chunk
independently, which can cause misinterpretation of multi-line content.

---

## Data Flow Details

### Frontend (TypeScript)

| File | Lines | Role |
|------|-------|------|
| `termViewModel.ts` | 412–420 | Ctrl+Shift+V handler: `clipboardReadText()` → `terminal.paste(text)` |
| `pane-actions.ts` | 185–193 | Context menu paste: same path |
| `termwrap.ts` | 710–723 | Native paste event: `pasteActive` flag only, delegates to xterm |
| `termwrap.ts` | 229, 439–469 | `onData` → `sendDataHandler(data)` |
| `termViewModel.ts` | 343–351 | `base64(data)` → WebSocket `blockinput` command |

xterm.js version: **`^6.0.0`** (`package.json:91`).  
xterm's `paste()` in `@xterm/xterm/src/browser/Clipboard.ts:51-56`:
1. Normalises `\r?\n` → `\r`
2. Optionally wraps with bracket-paste sequences
3. Calls `coreService.triggerDataEvent(text, true)` — no size limit

### Backend (Rust)

| File | Lines | Role |
|------|-------|------|
| `websocket.rs` | 428–445 | Receives `blockinput`, decodes base64, calls `blockcontroller::send_input()` |
| `shell.rs` | 44 | `SHELL_INPUT_CH_SIZE = 32` |
| `shell.rs` | 565–589 | `send_input()`: `try_send()` — **silently drops on Full** |
| `shell.rs` | 1038–1042 | Reorder buffer cap: also 32 — same drop behaviour |
| `shell.rs` | ~790–810 | PTY write loop: sequential `write_all()` |

---

## Failure Scenarios

### Scenario A — Channel overflow (most common)
xterm emits paste in N onData chunks. N WebSocket messages arrive before the PTY
write loop can drain the channel (32 slots). Slots 33..N are dropped. Paste is
truncated at roughly the 32nd chunk boundary.

### Scenario B — WebSocket frame size (large pastes only)
Axum's WebSocket has a default max frame size (~64 KB per message). A paste
larger than ~48 KB base64-decoded could be split across multiple WebSocket
frames. If frame reassembly is not handled, only the first frame is processed.
*(Less likely — axum handles multi-frame messages — but worth confirming.)*

### Scenario C — Bracket-paste off → shell misparse
With BPM disabled, a multi-line paste hits the shell as a stream of characters.
Some shells (fish, zsh with multi-line bindings) will execute partial lines before
the full paste is received, splitting the paste at newlines. This is a separate
semantic corruption, not data loss per se.

---

## Fix Plan

### Fix 1 — Replace `try_send` with async backpressure (REQUIRED)

**File:** `agentmux-srv/src/backend/blockcontroller/shell.rs`

Change `send_input()` from fire-and-forget `try_send()` to awaited `send()`.
The WebSocket handler is already async; blocking on the channel send applies
natural backpressure that prevents drops.

```rust
// BEFORE (drops silently)
match tx.try_send(input) {
    Ok(()) => {}
    Err(TrySendError::Full(_)) => { tracing::warn!("input channel full, dropping"); }
    Err(TrySendError::Closed(_)) => { /* block stopped */ }
}

// AFTER (backpressure, no drops)
match tx.send(input).await {
    Ok(()) => {}
    Err(_) => { /* block stopped — channel closed */ }
}
```

Increase `SHELL_INPUT_CH_SIZE` from 32 → 256 to reduce the pressure during
normal bursts, while keeping backpressure as the safety net:

```rust
const SHELL_INPUT_CH_SIZE: usize = 256;
```

### Fix 2 — Frontend paste chunking + flow control (BELT-AND-SUSPENDERS)

**File:** `frontend/app/view/term/termViewModel.ts`

For pastes above a threshold (~16 KB decoded), split into fixed-size chunks
(e.g. 4 KB) and send them sequentially, waiting for a configurable inter-chunk
delay (~5 ms). This prevents a single large paste from flooding the channel even
if Fix 1 is not yet deployed.

```typescript
const PASTE_CHUNK_BYTES = 4 * 1024;  // 4 KB per frame
const PASTE_CHUNK_DELAY_MS = 5;

async function sendLargePaste(text: string) {
  const encoder = new TextEncoder();
  const bytes = encoder.encode(text);
  for (let offset = 0; offset < bytes.length; offset += PASTE_CHUNK_BYTES) {
    const chunk = bytes.slice(offset, offset + PASTE_CHUNK_BYTES);
    const str = new TextDecoder().decode(chunk);
    sendDataToController(str);
    if (offset + PASTE_CHUNK_BYTES < bytes.length) {
      await new Promise(r => setTimeout(r, PASTE_CHUNK_DELAY_MS));
    }
  }
}
```

Hook into the existing paste path in `sendDataToController()` or the `handleTermData()`
`pasteActive` branch in `termwrap.ts:439-469`.

### Fix 3 — Enable bracketed paste mode by default (RECOMMENDED)

**File:** `frontend/app/view/term/term.tsx`

Change the default for `term:allowbracketedpaste` from `false` → `true`.
Modern shells (bash 4+, zsh, fish) all support BPM. Enabling it:
- Prevents the shell from executing partial lines mid-paste
- Lets the shell handle the full text as an atomic unit

```typescript
const termAllowBPM = termBPMAtom() ?? true;  // default ON
```

Provide a per-pane setting to disable for shells that don't support BPM.

### Fix 4 — User feedback on paste size (UX)

**File:** `frontend/app/view/term/termwrap.ts` or a toast/notification layer

For pastes above ~32 KB, show a non-blocking toast:  
_"Large paste (N KB) — sending in chunks…"_  
Clears automatically when done. This prevents the user from thinking the paste
succeeded when only part of it arrived.

---

## Implementation Phases

| Phase | Fix | Risk | Effort |
|-------|-----|------|--------|
| **A** | Fix 1: `send().await` + `SHELL_INPUT_CH_SIZE=256` | Low — async fn, no API change | S |
| **B** | Fix 2: Frontend chunked send | Low — additive path | M |
| **C** | Fix 3: BPM default ON | Medium — may break shells without BPM support | S |
| **D** | Fix 4: Paste progress toast | Low | S |

Phase A alone fixes the data loss. Phases B–D are defence-in-depth and UX polish.

---

## Testing

1. **Unit test (shell.rs):** Send 500 rapid `BlockInputUnion::data` messages to
   `send_input()` concurrently; verify all 500 reach the PTY write loop (no drops).
2. **Integration test:** Paste 1 MB of text into a `cat` session; verify the full
   1 MB appears in output.
3. **Threshold test:** Binary-search the paste size at which truncation currently
   occurs (expected: ~32 × average xterm chunk size).
4. **BPM regression:** Enable BPM and paste multi-line content; verify no
   premature execution of partial lines.

---

## Files Changed

```
agentmux-srv/src/backend/blockcontroller/shell.rs   (Fix 1)
frontend/app/view/term/termViewModel.ts              (Fix 2)
frontend/app/view/term/termwrap.ts                   (Fix 2 hook, Fix 4)
frontend/app/view/term/term.tsx                      (Fix 3)
```

---

## References

- xterm.js `Clipboard.ts` paste(): `node_modules/@xterm/xterm/src/browser/Clipboard.ts:51-56`
- `SHELL_INPUT_CH_SIZE`: `agentmux-srv/src/backend/blockcontroller/shell.rs:44`
- `send_input()` drop site: `agentmux-srv/src/backend/blockcontroller/shell.rs:565-589`
- Bracketed paste mode: `frontend/app/view/term/term.tsx:160-162`
- Related upstream Claude Code issue: `anthropics/claude-code#59622`
