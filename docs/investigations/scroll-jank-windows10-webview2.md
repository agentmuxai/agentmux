# Investigation: Terminal Flash Scroll — Windows 10 Only

**Date:** 2026-03-22
**Symptom:** Whole screen does a flash-scroll while Claude Code is writing output.
User is doing nothing — no wheel input, no keystrokes. Sometimes 2 flashes within 250ms.
Absent on Windows 11.
**Prior fixes:** PR #206 (Tier 1 + Tier 2) — reduced macOS rocket scroll, did NOT fix this.
**Status:** Root cause identified. Wheel event handling is irrelevant to this bug.

---

## Exact Mechanism

### Step-by-step what happens during a flash

Claude Code uses **Ink** (React-for-terminals). Every streaming token triggers an Ink
render cycle:

```
PTY writes → msg A: ESC[{N}A        (cursor-up N lines)
PTY writes → msg B: ESC[J {content} ESC[{N}B   (erase, new content, cursor-down)
```

These arrive as **separate WebSocket messages** from agentmuxsrv. Each message goes
directly to `doTerminalWrite()` with no buffering:

```
handleNewFileSubjectData(msg) → doTerminalWrite(chunk) → terminal.write(chunk)
```

Each `terminal.write()` call causes xterm.js to synchronously update the viewport:

1. **msg A processed:** cursor moves up N lines. If N is large enough, cursor is now above
   the visible viewport → xterm.js scrolls UP to show cursor → **flash #1**

2. **msg B processed:** content written, cursor returns to bottom → xterm.js scrolls
   DOWN to follow cursor → **flash #2**

This is why two flashes occur within ~250ms: one up, one down, one Ink render cycle.

### Why only Windows 10

On **Windows 10**, the DWM compositor draws a new frame for each `terminal.write()` call
because each call synchronously flushes the WebView2 compositor layer. The two viewport
positions (cursor-above, cursor-at-bottom) appear as distinct rendered frames.

On **Windows 11**, the updated DWM compositor and newer WebView2 runtime batch these
compositor updates within the same vsync interval. The intermediate "cursor above viewport"
state is never presented to screen.

This is consistent with Windows 11's general improvement of DWM frame scheduling and
matches the VS Code team's observations when building their RAF-batched write system
(microsoft/vscode#63669).

### Why Tier 1 and Tier 2 don't help

- **`scrollOnUserInput: false`** (Tier 1a): prevents scroll on user keystrokes. User is
  not pressing any keys → irrelevant.
- **`smoothScrollDuration: 0`** (Tier 1b): disables animated scroll. The scroll is already
  instant. Making it instant doesn't prevent it from happening.
- **`|deltaY| < 4` wheel blocker** (Tier 2): blocks momentum WheelEvents. There are no
  WheelEvents. User is not scrolling → irrelevant.

---

## Evidence

**VS Code / xterm.js:** VS Code has used RAF-batched terminal writes since 2018 specifically
to prevent this exact artifact. Their `TerminalProcessManager` accumulates data chunks into
a buffer and flushes via `requestAnimationFrame`, ensuring all PTY writes within one frame
are coalesced into a single `terminal.write()` call.
Source: [VSCode #63669](https://github.com/microsoft/vscode/issues/63669),
[VSCode #65351](https://github.com/microsoft/vscode/issues/65351)

**Ink render pattern:** Ink has no alternate-screen support — it uses cursor-up to erase
and redraw in the main buffer. The height of the Claude Code Ink UI routinely exceeds the
visible pane height during tool execution (file trees, diff output, etc.), making cursor-up
sequences large enough to scroll the viewport.
Source: [Ink #450 — Flickering at full height](https://github.com/vadimdemedes/ink/issues/450),
[Ink #222 — No scroll support](https://github.com/vadimdemedes/ink/issues/222)

**Windows Terminal cursor viewport reset:** Windows Terminal has a confirmed bug where
`SetConsoleCursorPosition` scrolls the viewport even when the cursor is already visible.
This compounds the issue on the ConPTY path.
Source: [Windows Terminal #14774](https://github.com/microsoft/terminal/issues/14774),
[Claude Code #34794 — Terminal scrolls to top on Windows](https://github.com/anthropics/claude-code/issues/34794)

**Windows 10 DWM frame scheduling:** Chromium / WebView2 on Windows 10 issues compositor
commits more aggressively than on Windows 11. Each JS microtask that causes a layout change
can trigger a separate compositor frame on Win10, while Win11 coalesces them within the vsync
window.

---

## The Fix: RAF-Batched Writes (Tier 3)

Buffer incoming PTY data and flush it in a single `terminal.write()` per animation frame.
This coalesces the cursor-up chunk and the content chunk from the same Ink render cycle into
one write, so xterm.js processes them atomically. The viewport only updates once — to the
final cursor position (back at the bottom) — and the intermediate "cursor above viewport"
state is never rendered.

### Implementation in `termwrap.ts`

```ts
// ── RAF write buffer ───────────────────────────────────────────────────────────
private rafBuffer: Uint8Array[] = [];
private rafPending = false;

private scheduleRafWrite(data: Uint8Array) {
    this.rafBuffer.push(data);
    if (!this.rafPending) {
        this.rafPending = true;
        requestAnimationFrame(() => {
            this.rafPending = false;
            if (this.rafBuffer.length === 0) return;

            // Coalesce all pending chunks into one write
            const totalLen = this.rafBuffer.reduce((n, b) => n + b.length, 0);
            const merged = new Uint8Array(totalLen);
            let offset = 0;
            for (const chunk of this.rafBuffer) {
                merged.set(chunk, offset);
                offset += chunk.length;
            }
            this.rafBuffer = [];
            this.doTerminalWrite(merged, null);
        });
    }
}
```

Replace the `doTerminalWrite` call in `handleNewFileSubjectData`:
```ts
// Before:
this.doTerminalWrite(decodedData, null);

// After:
this.scheduleRafWrite(decodedData);
```

Also flush the RAF buffer in `flushHeldData()` to avoid stale data at init.

### Expected outcome

- Ink's cursor-up + erase + content + cursor-down sequences arrive as separate WS messages
  but all land in `rafBuffer` within the same ~16ms frame
- One `terminal.write(merged)` processes them atomically
- Viewport updates once to the final cursor position (bottom)
- No intermediate scroll to cursor-above-viewport
- Latency added: ≤ 16ms (one frame), imperceptible during streaming output

### Risks

- **Init path:** `flushHeldData` already drains `heldData` synchronously before the gate opens.
  The RAF path should only be active post-init (`this.loaded === true`).
- **User input echoing:** `handleTermData` sends keystrokes; `terminal.write()` for echo
  (if any) should bypass the RAF buffer for responsiveness. Echo typically comes from the
  PTY, not direct writes — low risk.
- **Binary/large writes:** Merging into one Uint8Array is O(n) in total bytes. At typical
  Claude Code output rates (tens of KB/s), this is negligible.

---

## What to Do With Tier 2

The `|deltaY| < 4` wheel blocker is still valid for macOS trackpad rocket-scroll (a different
bug). Leave it in place. It just doesn't address this symptom.

---

## Summary

| Question | Answer |
|---|---|
| What causes the flash? | Two `terminal.write()` calls — cursor-up chunk, then content chunk — each updating the viewport separately |
| Why two flashes within 250ms? | Up-scroll (cursor tracking cursor-up) + down-scroll (cursor tracking back to bottom) = one Ink render cycle |
| Why only Windows 10? | Win10 DWM commits a compositor frame per write; Win11 coalesces within vsync |
| Why doesn't Tier 1/2 fix it? | Neither addresses PTY-output-driven viewport updates |
| Fix | RAF-batched writes — coalesce chunks per frame so cursor-up + content = one atomic write |
