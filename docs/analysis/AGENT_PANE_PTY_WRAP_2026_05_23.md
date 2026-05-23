# Analysis — agent-pane live-log wraps at a fixed ~80 chars

**Date:** 2026-05-23
**Author:** AgentA
**Severity:** Low (functional — readability annoyance, no data loss)
**Area:** `agentmux-srv/src/backend/blockcontroller/shell.rs`, the agent pane's frontend wiring.

---

## Symptom

The agent pane's live-log panel renders captured tool output with hard
line-wraps at ~80 characters. The wrap point does **not change** when the
agent pane is resized wider or narrower. Long lines (e.g. `git log
--oneline` output for commits with long subject lines) break mid-word.

```
99818cd feat(live-log): PTY-backed tool streaming + auto-expand inline panel (#
888)
633b1c3a feat(agent-pane): hover-expand tool blocks + show bash output (#825)
```

CSS `white-space: pre-wrap` does not help — the `\n` after `(#` is **in
the captured stream**, not a display-time wrap.

## Root cause

The agent pane's PTY is opened with a **hardcoded column count**:

```rust
// agentmux-srv/src/backend/blockcontroller/shell.rs:405-410
let pty_size = PtySize {
    rows: 25,
    cols: 80,
    pixel_width: 0,
    pixel_height: 0,
};
```

The agent CLI (Claude Code) runs inside this PTY. When it shells out to
tools (`git`, `ls`, etc.), the children inherit the PTY's column count
via the kernel and `$COLUMNS`. Tools that respect terminal width (most
CLIs) wrap their output to fit 80 chars — embedding real `\n` characters
into the byte stream that the live-log subscriber captures.

## Why pane resize does not help

The backend **already supports** PTY resize. `shell.rs:842-852` handles a
`term_size` field on incoming input messages and calls
`master.resize(pty_size)`. The standard **terminal pane** uses this via
xterm.js's `fitAddon` (see CLAUDE.md `termWrap.handleResize()` — "PTY
gets SIGWINCH and CLI reflows").

But the **agent pane** is a custom UI, not an xterm.js terminal. Its
render path never observes panel width and never sends a `term_size`
update. The agent's PTY therefore stays at 80×25 for its entire
lifetime regardless of how the user sizes the pane.

## Fix path

In the agent pane's container component:

1. `ResizeObserver` on the log-panel element.
2. Convert pixels → cols using the panel's monospace cell width
   (`font-size × ~0.6`, or read a CSS variable).
3. Send `ControllerInputCommand` with `term_size: { rows, cols }`
   whenever the computed value changes (debounce by ~150 ms).

Backend already accepts this — no Rust changes. CLIs that respect
`SIGWINCH` / `$COLUMNS` (the majority) will reflow on their next
invocation.

## Caveat (set expectations)

**Already-captured lines stay wrapped.** Reflowing historical PTY output
would corrupt anything that used the original width for layout
(`git log --graph` ASCII art, aligned tables). Only output produced
AFTER the resize matches the new width. The live-log buffer carries
content from possibly many widths over the agent's lifetime; that
history cannot be retroactively re-flowed.

## Quick-win alternative

A one-line bump of the initial `cols: 80` → `cols: 160` (or `200`) in
`shell.rs:407` drops the wrap frequency dramatically without touching
the frontend. Tradeoff: very wide tool output (`ls -l` on a directory
with long names) may look sparser on narrow panes. Pick this if the
proper dynamic resize is not worth the frontend wiring right now.

## Recommendation

Do the proper dynamic resize. The frontend code is small and contained
(one ResizeObserver, one debounce, one `ControllerInputCommand` per
size change), and it brings the agent pane in line with the terminal
pane's behavior. Schedule as a small follow-up PR.
