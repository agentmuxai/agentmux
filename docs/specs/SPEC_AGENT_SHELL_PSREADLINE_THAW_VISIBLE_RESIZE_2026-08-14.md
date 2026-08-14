# Agent shell drawer: PSReadLine thaw resize causes a visible ~9px width blip ~300-350ms after open

**Date:** 2026-08-14
**Status:** Implemented and empirically verified — see §7.
**Owner:** Agent1
**Area:** Agent pane / shell drawer terminal (`TermWrap`, `termwrap.ts`)

---

## 0. History — a first theory was wrong, disproven by review before landing

Original report: "in the agent pane, when opening the shell, the zoom level
twitches one level every time it opens, after about 500ms or so, happens
every time."

A first investigation pass (PR #2578, not merged — closed after review)
theorized this was the same class of bug as
`SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md` (PR #2522): that the shell's
font-size formula's second input, the parent agent pane's own zoom
(`props.agentPaneZoom()`), could still be resolving asynchronously when the
shell mounts. A fix was written and unit-tested, but a codex review on that
PR correctly identified — and this was independently re-verified by reading
`frontend/app/block/block.tsx:252-274` directly — that the parent block's
`useWaveObjectValue` fetch is *already guaranteed resolved* by the time
`AgentShellSubblock` can even mount at all: `Block`'s own ViewModel-creation
effect (which is what eventually renders the agent pane, and everything
nested inside it) doesn't run until that same block's data has loaded. The
proposed fix's tests only passed because they rendered `AgentShellSubblock`
in isolation, bypassing that real gating — a state the actual app can never
produce. That PR was closed without merging. This spec documents the
corrected investigation.

## 1. Root cause (empirically verified, not just theorized)

### 1.1 The mechanism: `termwrap.ts`'s Windows-only PSReadLine "thaw"

`TermWrap.init()` (`termwrap.ts:396-431`) schedules, 250ms after init
reaches that point, a synthetic resize cycle **gated to `PLATFORM ===
PlatformWindows`**:

```ts
this.thawTimeoutId = setTimeout(() => {
    // ...
    const baseCols = this.terminal.cols;
    const baseRows = this.terminal.rows;
    const targetCols1 = baseCols + 1;
    this.terminal.resize(targetCols1, baseRows);
    this.sendTermSize();
    this.thawRafId = requestAnimationFrame(() => {
        // ...
        this.terminal.resize(baseCols, baseRows);
        this.sendTermSize();
    });
}, 250);
```

This exists for a real, documented reason (issue #1042,
`docs/analysis/archive/TERM_JUMBLE_STRUCTURED_2026_05_25.md` §7a): a
terminal that never gets a subsequent resize after its initial default-80→
final-cols transition leaves PSReadLine's tracked cursor position
desynced from xterm's actual cursor, on Windows/ConPTY specifically. The
fix replays one synthetic `cols+1` → `cols` resize cycle to force PSReadLine
to re-sync, split across two `requestAnimationFrame` ticks because xterm
coalesces same-frame resizes into a single SIGWINCH.

**This resize is real and visible** — `this.terminal.resize(cols, rows)`
changes xterm's actual rendered grid (not just an internal counter), and
the renderer reallocates its canvas accordingly.

### 1.2 Live verification via CDP

This machine runs Windows (`process.platform === "win32"`), and had a live
`task dev` instance running with CDP enabled (`127.0.0.1:9223`). Connected
directly (`ws` npm package, already a project dependency) to the live page,
and captured `.agent-shell-subblock canvas` pixel widths on every animation
frame for 3 seconds after clicking the real "Show the shell" button
(`title="Show the shell"`, `AgentComposerStrip.tsx:302`) — the actual
production UI trigger, not a synthetic test harness.

**Result, 4/4 runs, essentially identical:**

| Run | Canvas settles at | Blip starts | Blip ends | Delta |
|---|---|---|---|---|
| 1 | ~93ms (`1359px`) | 343ms | 356ms (13ms later) | `1359 → 1368 → 1359` (+9px) |
| 2 | ~93ms (`1359px`) | 347ms | 360ms | `1359 → 1368 → 1359` (+9px) |
| 3 | ~78ms (`1359px`) | 314ms | 327ms | `1359 → 1368 → 1359` (+9px) |
| 4 | ~84ms (`1359px`) | 337ms | 350ms | `1359 → 1368 → 1359` (+9px) |

`.agent-view`'s CSS `zoom` inline style was read alongside every sample and
stayed at `"1"` (untouched) throughout every run — confirming this is a
**grid/canvas-width change**, not a CSS zoom-property change, consistent
with §1.1's mechanism and ruling back out any remaining doubt about a CSS
`zoom` cascade being involved.

The blip window (314-360ms across runs) is a plausible match for "about
500ms" from a user's casual real-time estimate — sub-second UI timing is
notoriously hard to eyeball precisely, and the actual delay is sensitive to
how long font-loading/setup takes before the 250ms timer is scheduled
(§1.1's `setTimeout(..., 250)` fires 250ms after `init()` *reaches that
point*, not 250ms after the drawer opens — total elapsed time from click
includes the preceding `loadInitialTerminalData()` + font-load steps,
observed here settling around 78-93ms, landing the full blip around
314-360ms in this environment; a slower machine or cold font cache would
push this later, plausibly into the ~500ms range the report described).

The delta is a clean **9px**, consistent with one character cell's width at
this terminal's font size/DPI — i.e., exactly the shape of a `cols+1` →
`cols` grid resize, not a fractional/font-metric-driven change. A full
xterm resize typically involves clearing and redrawing the visible text
buffer at the new grid dimensions, which is very plausibly what reads as a
"twitch" to a user even though the underlying pixel delta is small and
brief (~13ms, about one frame) — the *redraw*, not just the width number,
is the visible event.

### 1.3 Why "one zoom level," specifically

The user's own framing ("zoom level twitches one level") is understandable
given the visual shape (a brief, one-step size change that reverts) even
though the actual mechanism is a column-count change, not a font-size/zoom
value change. Nothing in this investigation found any genuine `zoom`
value or CSS `zoom` property change anywhere in the ~3s post-open window —
confirmed directly via the live trace (§1.2).

## 2. Why this can't simply be deleted

The thaw fixes a real bug (PSReadLine cursor desync on Windows) for
terminals that never receive any other post-init resize. Removing it
outright would very likely reintroduce that regression for exactly the
population of users this was shipped for (#1042). Any fix here needs to
keep delivering the two-step resize *to the backend/PTY* while eliminating
the *visible* frontend side effect.

## 3. Proposed fix direction

`sendTermSize()` (`termwrap.ts:750-758`) always reads `this.terminal.rows`/
`this.terminal.cols` directly — there's no way today to tell the backend a
size without the frontend's own rendered grid actually being at that size
first:

```ts
private sendTermSize() {
    const termSize: TermSize = { rows: this.terminal.rows, cols: this.terminal.cols };
    // ...
    sendWSCommand(wsCommand);
}
```

The thaw's actual goal is narrower than "resize the terminal" — it's
"deliver two SIGWINCH-equivalent notifications to the backend PTY,"
which is what actually forces PSReadLine to re-sync. The frontend's own
rendered grid never needs to change at all for that to work. **Confirmed**
(not just assumed) by reading the actual handler chain:
`agentmux-srv/src/server/websocket.rs:554-570`'s `"setblocktermsize"` match
arm deserializes the WS message's `termsize` field and forwards it directly
as `blockcontroller::BlockInputUnion::resize(ts)` — no comparison against
any previously-known frontend size, no validation beyond basic
deserialization. It has no inherent need to agree with whatever xterm is
currently rendering.

**Direction:** give `sendTermSize()` an optional explicit-size parameter,
defaulting to the current behavior (`this.terminal.rows`/`cols`) for every
existing call site, and have the thaw call it directly with synthetic
dimensions **without ever calling `this.terminal.resize(...)`**:

```ts
private sendTermSize(override?: TermSize) {
    const termSize: TermSize = override ?? { rows: this.terminal.rows, cols: this.terminal.cols };
    // ... unchanged
}
```

```ts
// Thaw, rewritten — no this.terminal.resize() calls at all:
this.thawTimeoutId = setTimeout(() => {
    // ...
    const baseCols = this.terminal.cols;
    const baseRows = this.terminal.rows;
    if (baseCols < 4) return;
    this.sendTermSize({ cols: baseCols + 1, rows: baseRows });
    this.thawRafId = requestAnimationFrame(() => {
        // ...
        this.sendTermSize({ cols: baseCols, rows: baseRows });
    });
}, 250);
```

This eliminates the visible blip entirely (the frontend canvas never
resizes, so there's nothing to redraw) while still delivering the same two
backend notifications PSReadLine's resync depends on.

## 4. Trade-offs / risk

- **Frontend/backend size can theoretically disagree for ~1-2 frames.**
  During the ~16-33ms between the two `sendTermSize` calls, the backend
  briefly believes the PTY is `cols+1` wide while the frontend still
  renders at `cols`. This is a pre-existing kind of gap in spirit (the
  *original* implementation already had this exact disagreement, just
  paired with a matching-but-visible frontend resize) — the risk is
  whether the shell process could emit output during that narrow window
  that's line-wrapped assuming the wrong width. Low probability (resize
  events don't typically provoke shell output on their own — the point is
  to correct PSReadLine's *next* natural prompt render) but worth a
  deliberate test pass during implementation: type immediately after a
  shell opens and confirm no jumbled output around the ~300-500ms mark.
- ~~Does the backend's `setblocktermsize` handler actually apply the size
  unconditionally~~ — **resolved** (§3): confirmed via
  `websocket.rs:554-570`, no validation/diffing against any prior size.
  Traced only to the WS-handler → `BlockInputUnion::resize` boundary, not
  all the way to the OS-level ConPTY resize syscall
  (`agentmux-srv/src/backend/blockcontroller/persistent.rs` and beyond) —
  low-risk residual gap given the handler layer already shows no
  size-consistency assumption, but worth a final glance during
  implementation rather than assumed with full certainty.

## 5. Non-goals / out of scope

- The two other resize-adjacent mechanisms considered and ruled out in
  earlier passes of this investigation: the parent-pane zoom race (§0,
  disproven) and the general font-load re-fit
  (`termwrap.ts:349-365`, fires on the very next rAF after init, not at
  ~300-500ms, and only when `customFit()` actually detects a dimension
  change — not implicated by the live trace, which showed the canvas
  already stable well before the blip).
- Changing when/whether the thaw fires (still Windows-only, still gated
  the same way) — only *how* it delivers the resize changes.

## 6. Open questions

- ~~Confirm the Rust-side `setblocktermsize` handler accepts an
  independently-varying size without requiring it to match the frontend's
  last reported dimensions (§4).~~ **Resolved** — confirmed via
  `websocket.rs:554-570`, no such validation exists.
- Should the live CDP-trace methodology used here (§1.2) be written up as
  a reusable debugging note for future "something visibly flickers/resizes
  shortly after X" reports? This was considerably more conclusive than
  static analysis alone for this class of bug.

## 7. Fix implemented and empirically re-verified

Implemented exactly the §3 direction: `sendTermSize(override?: TermSize)`,
and the thaw now calls it directly with synthetic `{cols, rows}` values,
never touching `this.terminal.resize()`.

**A first re-verification pass gave a false negative.** Re-running the same
live-CDP-trace methodology from §1.2 against the already-open CDP target
from the original investigation showed the *identical* blip, unchanged in
every measured characteristic, after the fix landed. Root cause of the
false negative: that CDP target (port 9223, the conventional dev debug
port) belonged to a *different* AgentMux agent's dev instance (`loap-06183`,
running from its own workspace clone), not this one — a leftover connection
from earlier in the investigation that was never re-validated. Confirmed by
fetching the live-served `termwrap.ts` module from that instance directly
(`fetch('/frontend/app/view/term/termwrap.ts')`): it was the pre-fix,
545-line version, still calling `this.terminal.resize()` in the thaw —
proving the trace was exercising old code the whole time, not this fix.
`AGENTMUX_VITE_PORT`/CEF's debug-port auto-fallback (`agentmux-cef/src/lib.rs`)
means the conventional 9223/5177 ports are only a safe assumption when
you're certain no other agent instance is already running — worth
remembering for any future live-CDP debugging session, and a good candidate
for the reusable debugging note raised above.

Re-ran the trace against this workspace's own dev instance (freshly built,
own auto-selected ports) instead, with `Error.stackTraceLimit` raised to 50
and a canvas-`width`-setter interceptor added to get full call stacks. This
first *confirmed* the mechanism precisely on the stale/pre-fix instance —
the two blip-causing `HTMLCanvasElement.width` writes traced directly back
through `Terminal.resize()` to the exact two `this.terminal.resize(...)`
call sites inside the old thaw block, exactly as this spec's §1.1
theorized. Then, against the corrected instance:

| Run | Canvas settles at | Post-settle changes through +3000ms | Thaw WS events (still fire, backend-only) |
|---|---|---|---|
| 1 | 37ms (1215×252) | none | `cols:136` @286ms, `cols:135` @299ms |
| 2 | 68ms (1215×252) | none | `cols:136` @308ms, `cols:135` @319ms |

The thaw's two `setblocktermsize` WS notifications still fire on the same
~250ms+rAF schedule as before (preserving the PSReadLine backend resync
this mechanism exists for), but the frontend canvas no longer changes size
in response — no blip, 2/2 clean runs. Confirms the fix.
