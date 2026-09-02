# SPEC: Always Respond to User Actions

**Date:** 2026-04-15  
**Status:** Draft  
**Author:** AgentA

---

## Problem

Users frequently perform an action in the agent pane (send a message, click a button, paste a code, press Enter) and receive no visible feedback. The action silently fails or is dropped. This erodes trust and leads to repeated attempts, duplicate messages, and confusion.

Examples observed:

- Typing a message and pressing Enter — no visible echo, no acknowledgement
- Submitting an auth code — no response, user doesn't know if it worked
- Clicking "Submit" on an empty input — button does nothing, no tooltip
- Slash command selected with Enter — silently consumed, no confirmation
- Agent finishes a turn — status line goes blank with no summary
- Login completes — no in-pane confirmation, user doesn't know it's ready

---

## Principle

**Every user action that can succeed or fail must produce a visible response within 200ms.**

This is non-negotiable. Silence is never acceptable feedback.

---

## Rules

### R1 — Input submission always echoes

When the user sends a message (Enter or Send button):

- The message MUST appear in the document immediately, before the agent responds
- The composer MUST clear
- The status line MUST switch to "Working…" within one animation frame

### R2 — Buttons must acknowledge clicks

Every clickable button in the agent pane must:

1. Visually respond on click (active/pressed state via CSS `:active` or a `loading` class)
2. Change label or disable while the action is in-flight (e.g. "Submit" → "...")
3. Show a result when complete — either success text or an error message

Never: button that does nothing when clicked with no feedback.

### R3 — Auth code submit

The auth code paste/submit flow must:

1. Disable the Submit button while processing (already: `disabled={pasting()}`)
2. Show "Code accepted — waiting for confirmation…" on success
3. Show a red error message if the call throws — never silently swallow errors
4. Clear error state when the user edits the input field again

Current gap: `setProviderAuth` writes to the AgentMux config, not the Claude CLI's stdin. This is a known bug tracked separately. Once fixed, the response text must accurately reflect whether the code was actually delivered.

### R4 — Async operations show progress

Any operation that takes >200ms must show a loading indicator immediately (not after the delay). Examples:

- CLI installation: log lines streaming in real-time (already implemented)
- Auth check: "checking authentication…" log line (already implemented)
- Controller registration: "registering…" log line (already implemented)
- Auth code submit: button label changes to "..." (already implemented)

New gap: login polling — if the 2s poll cycle fails transiently, the user sees no change. A heartbeat log line ("still waiting…" every 30s) would reassure the user.

### R5 — Login completion shows in-pane message

When login polling succeeds:

- A markdown node MUST be appended to the document: `✓ Logged in as **email**`
- The log panel MUST show `auth: authenticated as email`
- The auth URL box MUST disappear

This is already implemented via `onLoginSuccess` callback. Do not remove it.

### R6 — Status line is never empty while working

While `loading=true`:

- The status line MUST show "Working…" (or the active tool name)
- The blue pulse dot MUST be visible

When a turn completes:

- The status line MUST show "Worked · $cost · Xs · N turns" (or subset if data unavailable)
- This state persists until the NEXT user message is sent

When idle (no stats):

- The status line renders an empty placeholder (`<span class="agent-status-line" />`) — this is intentional and acceptable. It reserves vertical space so the layout doesn't jump.

### R7 — Error states are always surfaced

No error path in the agent pane may silently drop the error. Every catch block must either:

1. Append a visible error node to the document, OR
2. Append a log line with `level: "error"`, OR
3. Set a visible error state in the UI

Swallowing errors in a catch with only `console.warn` is not acceptable in any user-facing code path.

---

## Implementation Checklist

- [x] Message echo: `UserMessageNode` appended in `useAgentCommands.sendMessage`
- [x] Status line cycling phrase while loading
- [x] Status line summary on turn complete (stats + "Worked")
- [x] Auth URL box with Copy button
- [x] Auth code Submit button with loading state
- [x] Login success message appended to document
- [ ] Auth code actually delivered to CLI stdin (requires CEF host fix — separate PR)
- [ ] Auth code submit error surfaced as visible message (currently shows only result text, no error display)
- [ ] Login poll heartbeat log line every 30s ("still waiting for browser login…")
- [ ] Submit button error state: red text below input

---

## Testing

For each of the gaps above, manual test:

1. Trigger the action
2. Verify visible feedback appears within 200ms
3. Verify the feedback accurately describes what happened (not a lie)
4. Verify error cases show a clear message (trigger by e.g. passing a bad auth code)
