# SPEC: Agent Pane Activity Label from Claude CLI OSC Window-Title Sequences

**Date:** 2026-06-18
**Status:** Draft / ready to implement
**Effort:** Medium — ~2–3 days. Rust OSC parser + WPS event (backend) + frontend subscriber. No schema change. No new npm dependency.

---

## 1. Goal

Claude Code CLI emits `OSC 0` escape sequences to update the terminal window title
with a **session-level conversation topic label** — e.g. `\x1b]0;claude - auth refactor\x07`.
Today those sequences are either silently dropped (agent panes) or only visible in
**terminal panes** (via xterm.js OSC handlers → `term:activity` block metadata → tab label).

This spec wires the same signal into **agent panes**, so a running Claude agent's tab
shows the conversation topic the agent is currently working on — matching what Claude
Code CLI already sets in native terminal windows.

> **Research note (2026-06-18):** Based on GitHub issues #21677, #27197, #23355,
> and community investigation, the OSC title is a *session-level topic* derived from
> the conversation (e.g. `"claude - infrastructure"`, `"discussing auth flow"`),
> **not** a per-tool-call status update (e.g. `"Claude: editing auth.rs"`). It updates
> infrequently — once when Claude determines the conversation topic — not on every tool
> call. See §2.1 for details.

---

## 2. Background and Current State

### 2.1 How Claude CLI emits the signal

Claude Code CLI uses two mechanisms to set the terminal window title:

**Mechanism A — `process.title` at startup:**
```
process.title = "claude"      // sets argv[0] on Linux/macOS; calls SetConsoleTitle() on Windows
```
This sets the window title immediately on launch.

**Mechanism B — OSC 0 for the conversation topic:**
```
\x1b]0;claude - auth refactor\x07
```
Claude Code uses **OSC 0** (`ESC ] 0 ; <payload> BEL`) once an LLM-derived conversation
topic is determined. The title format observed in the wild (GitHub issues #21677, #23355):

- `"claude"` — at launch (process.title, no OSC sequence)
- `"claude - <topic>"` — once topic is inferred from the conversation
- `"Claude Code"` — some versions use title-case after the first interaction

The sequence is standard ECMA-48 Operating System Command:
```
ESC  ]   <ps>  ;  <payload>  BEL
0x1b 0x5d "0"  ";" "claude - auth refactor"  0x07
```

Two terminator variants exist:
- **BEL** (`0x07`) — most common, used by Claude Code CLI (confirmed #21677)
- **ST** (`0x1b 0x5c`) — ECMA-48 standard; some PTY libraries emit this

Both must be handled.

**What this is NOT:**
- It is NOT a per-tool-call activity indicator. Claude does not emit `"Claude: editing auth.rs"`
  on every Edit tool call. Title updates are infrequent (session-level).
- OSC 133/633 (shell integration — command-start, command-end, exit-code annotations) is
  **NOT implemented** in Claude Code (confirmed: GitHub issues #27221, #29171 are open
  feature requests; no implementation exists as of June 2026).
- OSC sequences emitted by bash tool subcommands are **NOT forwarded** through Claude Code's
  PTY — they are captured as literal text (GitHub issue #15082). Only Claude's own process
  title updates reach the outer terminal.

**Disabling:** `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` suppresses all title updates.

**No clear-on-exit:** Claude Code does not emit a title restore sequence on exit (bug
tracked in #27197). The AgentMux SessionEnd handling (§3.6) compensates for this.

### 2.2 Terminal panes — already working

`termwrap.ts` registers xterm.js OSC handlers for codes 0 and 2.
`termosc.ts:handleOscTitleCommand()` strips the `"Claude: "` / `"Claude Code: "`
prefix, debounces 300 ms, and calls `UpdateObjectMeta(blockRef, { "term:activity": activity })`.
`termViewModel.ts` reads `block.meta["term:activity"]` for the tab label.

**This is the proven pattern. Agent panes replicate it via a different
capture point (backend, not frontend).**

> **Note on prefix stripping:** `termosc.ts` strips `"Claude: "` and `"Claude Code: "`
> prefixes. Research suggests the current Claude Code format is `"claude - <topic>"` (not
> `"Claude: <topic>"`). The stripping rules should be updated in §3.3 to handle both
> the legacy and current formats — `"claude - "`, `"Claude: "`, `"Claude Code: "`.

### 2.3 Agent panes — currently broken

Agent panes run Claude via the **persistent controller** / PTY. Raw PTY bytes
flow:

```
Claude CLI process → PTY master → blockcontroller PTY read loop
  → FileStore (raw bytes)  ← OSC sequences reach here today, unstripped
```

There is no xterm.js in the agent pane path; OSC sequences are never parsed.
They currently reach the FileStore verbatim (potentially corrupting rendered
output) and are never surfaced as metadata.

### 2.4 Bash tool call path (separate)

For Claude's `Bash` tool calls, the bashwrap subprocess runs the command and
calls `strip_ansi()` before returning output. Additionally, per GitHub issue
#15082, Claude Code itself does NOT forward OSC sequences emitted by bash
subcommands to the outer terminal — they are captured as literal text. Both
layers confirm that bash-subcommand OSC is not relevant to this spec.

### 2.5 Future extension: `terminalSequence` hook field

As of Claude Code v2.1.141, hook responses can include a `"terminalSequence"` field
containing OSC escape sequences, which Claude Code emits to the terminal on the hook's
behalf. This is documented in the hooks reference under universal output fields.

This provides an alternative/complementary path: an AgentMux-managed Claude Code hook
could respond to `PreToolUse`/`PostToolUse` events with `terminalSequence` payloads
that emit per-action status (e.g. `\x1b]0;claude - editing auth.rs\x07`). This would
give real-time per-tool-call activity — something the built-in title mechanism does not
provide.

**This is out of scope for the current spec** (requires a hooks integration layer) but
is the recommended path for true per-action status if that level of granularity is
desired in a future iteration.

---

## 3. Design

### 3.1 Capture point — blockcontroller PTY read loop

The correct capture point is `agentmux-srv/src/backend/blockcontroller/shell.rs`
(or the equivalent persistent controller read loop), where the agent PTY
output is read chunk-by-chunk before being appended to the FileStore.

**Why here and not bashwrap:**
- bashwrap is a separate process handling only tool-call subcommands; it
  does not see Claude CLI's own output
- The blockcontroller has direct access to the WPS broker for event emission
- Keeps all OSC extraction in one place (single responsibility)

**Sequence diagram:**

```
Claude CLI → PTY → blockcontroller read loop
                     |
                     +-- OscExtractor::feed(chunk)
                     |     +-- yields Vec<OscEvent> (OSC payloads)
                     |     +-- returns cleaned chunk (OSC stripped)
                     |
                     +-- For each OscEvent:
                     |     +-- publish WaveEvent "block:activity" { blockId, text }
                     |
                     +-- Append cleaned chunk to FileStore
```

### 3.2 Rust — OscExtractor state machine

New file: `agentmux-srv/src/backend/osc_extractor.rs`

A stateful, allocation-minimal byte-stream parser. Holds cross-chunk state so
sequences split across two PTY reads are correctly assembled.

```
States:
  Idle      — scanning for ESC (0x1b)
  AfterEsc  — saw 0x1b, checking next byte
  InOsc     — inside OSC, accumulating payload bytes

Transitions:
  Idle     + 0x1b        -> AfterEsc
  Idle     + other       -> emit verbatim, stay Idle
  AfterEsc + 0x5d (']') -> InOsc, clear buffer
  AfterEsc + 0x1b       -> emit prior ESC verbatim, stay AfterEsc
  AfterEsc + other      -> emit ESC + byte verbatim, -> Idle
  InOsc    + 0x07 (BEL) -> complete: yield OscEvent, -> Idle
  InOsc    + 0x1b       -> possible ST start; hold, check next byte
  InOsc    + 0x5c ('\') (after 0x1b) -> complete: yield OscEvent, -> Idle
  InOsc    + other      -> push to payload buffer
```

**Cross-chunk buffering:** State and payload buffer persist between `feed()`
calls. A guard caps the payload buffer at 4 KB; if exceeded the partial
sequence is discarded and state resets (prevents unbounded memory growth on
corrupt PTY streams).

**Public API:**
```rust
impl OscExtractor {
    pub fn new() -> Self { ... }

    /// Process one PTY chunk. Returns:
    ///   .0  cleaned bytes with OSC sequences removed (write to FileStore)
    ///   .1  any complete OSC events found in this chunk
    pub fn feed(&mut self, chunk: &[u8]) -> (Vec<u8>, Vec<OscEvent>);
}

pub struct OscEvent {
    pub ps: u16,        // OSC parameter number (0, 2, ...)
    pub payload: String // UTF-8 payload, prefix already stripped (see §3.3)
}
```

Only OSC 0 and 2 are surfaced as events; all others are stripped silently.

### 3.3 Payload normalisation

Strip known Claude Code prefixes from the payload before emitting events.
Handle all observed format variants (research-confirmed and legacy):

| Observed format | Strip prefix | Example result |
|---|---|---|
| `"claude - auth refactor"` | `"claude - "` | `"auth refactor"` |
| `"Claude: editing auth.rs"` | `"Claude: "` | `"editing auth.rs"` |
| `"Claude Code: summary"` | `"Claude Code: "` | `"summary"` |
| `"claude"` (startup/idle) | — | discard (empty after strip or bare "claude") |
| `"Claude Code"` (post-launch) | — | discard (bare product name, no topic) |

Logic:
1. Try stripping each prefix in order; use first match.
2. If result is empty, `"claude"`, or `"Claude Code"` — discard (no event).
3. Non-UTF-8 bytes → `String::from_utf8_lossy` (U+FFFD substitution).

This ensures bare startup sequences don't create empty or misleading activity labels.

### 3.4 WPS event — `block:activity`

New constant in both `wps-events.ts` and `wps.rs`:

```
Event name: "block:activity"
Scope:      "block:<blockId>"
Persist:    0  (transient — last value lives in block metadata anyway)
Payload:    { "blockId": "...", "activity": "auth refactor" }
```

Backend helper (mirrors `publish_install_progress` pattern in `wps.rs`):

```rust
pub fn publish_block_activity(broker: &Arc<Broker>, block_id: &str, activity: &str) {
    broker.publish(WaveEvent {
        event: EVENT_BLOCK_ACTIVITY.to_string(),
        scopes: vec![format!("block:{}", block_id)],
        sender: String::new(),
        persist: 0,
        data: Some(serde_json::json!({ "blockId": block_id, "activity": activity })),
    });
}
```

Called from the PTY read loop for each `OscEvent` yielded by `OscExtractor::feed`.
The 300 ms debounce lives on the frontend so the backend emits every event
without rate-limiting.

### 3.5 Frontend — agent pane subscriber

Inline in `agent-view.tsx` or a dedicated `useBlockActivity(blockId)` hook:

```typescript
// wps-events.ts addition:
BlockActivity: "block:activity"

// In agent-view.tsx onMount / createEffect:
let activityDebounce: ReturnType<typeof setTimeout> | undefined;

const unsub = waveEventSubscribe(
    { eventType: WpsEvents.BlockActivity, scope: `block:${blockId}` },
    (event: WaveEvent) => {
        const activity = event.data?.activity as string | undefined;
        if (!activity) return;
        clearTimeout(activityDebounce);
        activityDebounce = setTimeout(() => {
            ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
                "term:activity": activity,
            });
        }, 300);
    }
);

onCleanup(() => {
    unsub();
    clearTimeout(activityDebounce);
});
```

**Key design choice — reuse `"term:activity"` metadata key.** The tab-label
read in `termViewModel.ts:191` already consumes this key. Agent panes read
it the same way. No new metadata key, no new tab-bar plumbing.

### 3.6 Clear on session end

When the agent session ends (`SessionEndEvent` / `session_end` event), clear
the activity label so a stale topic does not persist on the next launch:

```typescript
// On SessionEnd:
ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
    "term:activity": null,
});
```

**Why this is necessary:** Claude Code itself does NOT clear the terminal title
on exit (confirmed bug in GitHub issue #27197). Without this, the previous
session's topic would appear in the tab label when a new session starts.

### 3.7 Tab label display

The topic label is a **secondary, persistent annotation** — displayed in a muted
style alongside the block name in the tab bar, not replacing it. This matches
the existing terminal-pane behaviour. If no topic has been detected or the
session has ended, the annotation is absent.

Since the title is session-level (not per-action), it will be stable once set
and does not need debouncing urgently — but 300 ms debounce is still correct
as a guard against rapid startup sequences.

Exact visual treatment (font size, truncation, colour) follows the existing
`term:activity` display style already implemented for terminal panes. No new
CSS needed if those rules are not scoped to `.term-*` — verify and widen
selectors if necessary.

---

## 4. Files

### New
- `agentmux-srv/src/backend/osc_extractor.rs`
  Unit tests: BEL terminator, ST terminator, cross-chunk split at every byte
  position, buffer overflow guard, empty payload after prefix strip,
  non-UTF-8 bytes, OSC codes other than 0/2, `"claude"` bare-name discard,
  `"claude - "` prefix strip.

### Modified — Backend
- `agentmux-srv/src/backend/mod.rs` — `pub mod osc_extractor`
- `agentmux-srv/src/backend/blockcontroller/shell.rs` — integrate
  `OscExtractor` into PTY read loop; call `publish_block_activity` per event;
  write cleaned chunk (not raw) to FileStore
- `agentmux-srv/src/backend/wps.rs` — add `EVENT_BLOCK_ACTIVITY` constant
  and `publish_block_activity()` helper

### Modified — Frontend
- `frontend/app/store/wps-events.ts` — add `BlockActivity: "block:activity"`
- `frontend/app/view/agent/agent-view.tsx` — subscribe to `block:activity`,
  debounce 300 ms, call `UpdateObjectMeta`; clear on `SessionEnd`

### No changes
- `agentmux-bashwrap/` — bash tool-call path; out of scope
- `frontend/app/view/term/` — terminal pane path unchanged
- `rpc-api.ts`, `gotypes.d.ts` — no new RPC or schema

---

## 5. Edge Cases

| Case | Handling |
|---|---|
| OSC split across two PTY reads | Cross-chunk state machine buffers payload; assembled on next feed() call |
| BEL terminator | `0x07` in InOsc state completes the sequence |
| ST terminator | `0x1b 0x5c` in InOsc state completes the sequence |
| Payload exceeds 4 KB | Buffer guard discards and resets — no crash, no memory growth |
| Non-0/2 OSC codes (OSC 7, OSC 133, etc.) | Stripped from output, no event emitted; handled separately in future |
| Non-UTF-8 bytes in payload | `String::from_utf8_lossy` — replaced with U+FFFD; event still emitted |
| Bare startup title `"claude"` or `"Claude Code"` | Discarded in normalisation — no WPS event, no metadata update |
| Empty activity after prefix strip | Discarded — no WPS event, no metadata update |
| Agent pane unmounted while streaming | `onCleanup` cancels debounce timer and unsubscribes from `block:activity` |
| SessionEnd before activity clears | Explicit `term:activity: null` on SessionEnd (compensates for Claude's no-clear-on-exit bug) |
| Rapid title updates (edge case) | 300 ms frontend debounce coalesces to last value in window |
| OSC in FileStore today (corruption) | Fixed as side-effect: OscExtractor strips OSC from cleaned chunk before FileStore append |
| `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` set by user | No OSC emitted; feature silently inactive; tab label stays absent |
| tmux passthrough not set | tmux strips OSC before it reaches AgentMux PTY; feature silently inactive in tmux setups |

---

## 6. Out of Scope

- **Per-action real-time status** ("currently editing auth.rs") — not emitted by Claude
  Code's built-in title mechanism. Requires a hooks integration layer using the
  `terminalSequence` hook output field (§2.5); separate spec.
- **OSC 7** (cwd notification) — useful for `!cmd` working-directory update; separate spec.
- **OSC 133 / 633** shell integration — NOT implemented in Claude Code as of June 2026.
- **Bash tool-call subcommands** emitting OSC — Claude Code strips these (GitHub #15082).
- **Ghost-text / inline suggestions** — separate spec.
- **`vte` crate** — overkill for OSC 0/2 alone; revisit if OSC 133 lands.
- **`CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` configuration UI** — out of scope; users
  who set this env var simply won't see tab labels.

---

## 7. Acceptance Criteria

1. Running a Claude agent whose conversation reaches a topic-detection point causes the
   agent pane tab to show the topic (e.g. `"auth refactor"`) as a secondary activity
   annotation within ~400 ms (300 ms debounce + propagation).
2. OSC sequences are stripped from FileStore — raw escape bytes no longer appear
   in the document renderer.
3. Activity label clears when the agent session ends; no stale text on next launch.
4. Bare startup sequences (`"claude"`, `"Claude Code"`) do not appear as tab labels.
5. Terminal pane behaviour is entirely unchanged.
6. `OscExtractor` unit tests pass: BEL, ST, cross-chunk split at every byte
   offset, overflow guard, empty/non-UTF-8 payloads, non-0/2 codes, all
   prefix-strip variants, bare-name discard.
7. No new npm dependency. No new WPS persist level. No backend schema change.

---

## 8. Research Sources

- GitHub issue #21677 — title uses `\x1B]0;title\x07`; set at startup and on topic detection
- GitHub issue #27197 — title set to `"Claude Code"` on launch, not cleared on exit
- GitHub issue #23355 — confirmed LLM-derived topic label format `"claude - <topic>"`
- GitHub issue #15082 — bash subcommand OSC NOT forwarded through Claude's PTY
- GitHub issue #21409 — open request to use OSC 2 instead of `process.title`
- GitHub issues #27221, #29171 — OSC 133/633 shell integration NOT implemented
- Claude Code docs hooks reference (code.claude.com) — `terminalSequence` hook output field
- Claude Code changelog v2.1.141 — `terminalSequence` added to hook response schema
