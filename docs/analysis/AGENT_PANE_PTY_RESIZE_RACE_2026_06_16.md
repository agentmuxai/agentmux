# Agent-Pane PTY Resize Race — "resize to N cols failed after 3 attempts"

**Date:** 2026-06-16
**Status:** Implemented 2026-06-16 (Layers A+B+C below). See changeset `.changesets/1781651181-fix-agent-seed-pty-size-at-spawn-*.md`.
**Component:** `frontend/app/view/agent/hooks/usePtyWidth.ts` ↔ `agentmux-srv/src/backend/blockcontroller/shell.rs`
**Related:** `docs/analysis/AGENT_PANE_PTY_WRAP_2026_05_23.md` (the dynamic-resize feature this race lives inside)

---

## 1. Symptom

When a **new agent pane loads**, the activity log occasionally shows:

```
[pty] resize to 77 cols failed: controller is not running (retrying in 500ms)
[pty] resize to 77 cols failed: controller is not running (retrying in 1000ms)
[pty] resize to 77 cols failed after 3 attempts: controller is not running
```

It is logged at **`warn` level — not fatal**. The visible consequence: the PTY keeps its
fallback width (see §3) instead of the pane's real width, so the agent CLI and its child
tools (`git`, `ls`, `claude`) wrap output at the wrong column count until the user manually
resizes the pane (which fires a fresh, now-succeeding send).

---

## 2. What the code is *trying* to do

The agent pane is a **custom UI, not an xterm.js terminal**, so the PTY hosting the agent CLI
never receives a `fitAddon`-style resize. `usePtyWidth` compensates:

1. A `ResizeObserver` watches the pane element.
2. Pixel width → columns: `floor((width − 16px) / (fontSize × 0.6))`, floored at `MIN_COLS = 40`.
3. Debounced by `DEBOUNCE_MS = 150` so a drag emits one RPC per gesture.
4. Sends `RpcApi.ControllerInputCommand` with `termsize: { rows, cols }`.

The backend routes that to `master.resize(...)` on the PTY. ("77 cols" is just the computed
width for the current pane.)

---

## 3. Root cause — a **two-layer** startup race

The code comment in `usePtyWidth.ts` blames *controller-not-yet-registered*. That is real, but
the trace shows a **second, deeper** ordering bug that makes even "wait for the controller to be
running" insufficient.

### Layer A — the PTY is born at the wrong size (the actual root cause)

`ShellController::start()` opens the PTY with a **hardcoded default**, never the real width:

```rust
// agentmux-srv/src/backend/blockcontroller/shell.rs:424
let pty_size = PtySize { rows: 25, cols: 200, pixel_width: 0, pixel_height: 0 };
let pair = pty_system.openpty(pty_size) ...;   // shell.rs:431
```

The comment at `shell.rs:420-422` even concedes: *"the PTY uses this default for the very first
batch of output."* So the resize RPC is **the only mechanism** that ever corrects the width — and
it runs *after* spawn, across a process boundary, on a debounce timer. Every byte the agent emits
before that resize lands is wrapped at 200 cols. **If the resize fails (Layer B), the width is
simply never corrected for that session.** The entire design hinges on a post-spawn resize
winning a race it sometimes loses.

### Layer B — `"running"` is published *before* the input channel exists

`send_input` rejects with the exact error string when the input channel is absent:

```rust
// shell.rs (send_input)
let tx = match &inner.input_tx {
    Some(tx) => tx.clone(),
    None => return Err("controller is not running".to_string()),
};
```

But inside `start()` the status broadcast happens **before** `input_tx` is created — verified
directly:

```rust
// shell.rs:354  set_status(RUNNING)
// shell.rs:357  self.publish_status();        // ← broadcasts controllerstatus = "running"
// shell.rs:361  let (input_tx, input_rx) = mpsc::unbounded_channel();
// shell.rs:364  inner.input_tx = Some(input_tx);   // ← only NOW can send_input succeed
```

The actual `master.resize(...)` runs even later, in the spawned input-drain loop:

```rust
// shell.rs:915-922
if let Some(ref size) = input.term_size {
    let pty_size = PtySize { rows: size.rows as u16, cols: size.cols as u16, .. };
    if let Err(e) = master.resize(pty_size) { ... }
}
```

**Consequence:** a frontend that (sensibly) waited for the `controllerstatus: "running"` event and
*then* sent the resize could **still** hit the `None` branch, because `"running"` is published in
the window between `set_status` and `input_tx = Some(..)`. The `input_tx` channel is *unbounded*,
so once it exists a queued resize is safe even before the drain loop starts — but there is **no
event that fires at the moment `input_tx` becomes available.**

### Timeline

```
T0     frontend: launch-flow Phase 3 → ControllerResyncCommand RPC
T0+ε   backend:  register_controller()  → controller visible in registry
T0+ε   backend:  start(): set_status(RUNNING)
T0+ε   backend:  publish_status()  ──────────────►  "running" event in flight
T0+ε   backend:  input_tx = Some(..)        ← gap: send_input fails if hit before here
T0+ε   backend:  openpty(200×25); spawn child; spawn input-drain loop
...
T0+150ms frontend: usePtyWidth debounce fires → ControllerInputCommand({termsize})
T0+150ms backend:  send_input(): input_tx None?  → "controller is not running"
```

### Current retry behavior (the backstop that sometimes runs out)

```
attempt 1: ~T0+150ms          (debounce)
attempt 2: +500ms  → ~T0+650ms
attempt 3: +1000ms → ~T0+1650ms
fail:      "resize to N cols failed after 3 attempts"
```

A fixed **~1.65 s** window with **no jitter**. On a busy machine (cold agent CLI spawn, auth,
Tokio scheduling pressure) the controller's input path can come up later than that, and all three
attempts land in the dead window.

### Key file references

| What | File:line | Verified |
|---|---|---|
| Hardcoded spawn size `200×25` | `shell.rs:424-426` | ✅ direct |
| `openpty` | `shell.rs:431` | ✅ direct |
| `set_status(RUNNING)` | `shell.rs:354` | ✅ direct |
| `publish_status()` (before input_tx) | `shell.rs:357` | ✅ direct |
| `input_tx = Some(..)` | `shell.rs:361-367` | ✅ direct |
| `master.resize()` in drain loop | `shell.rs:915-922` | ✅ direct (grep) |
| `"controller is not running"` | `shell.rs` (send_input) | trace |
| `controllerinput` RPC handler | `websocket.rs:751-759` | trace + grep |
| `send_input` dispatch | `blockcontroller/mod.rs:271-276` | trace |
| `resync_controller` (register ~407, start ~408) | `blockcontroller/mod.rs:334-409` | trace |
| `EVENT_CONTROLLER_STATUS = "controllerstatus"` | `blockcontroller/mod.rs:459-473`, `wps.rs` | trace |
| Frontend status subscribe | `useControllerStatusEvents.ts:22-40` | trace |
| Launch Phase 3 `ControllerResyncCommand` | `flows/launch-flow.ts:277-299` | trace |
| `send`/retry/backoff | `usePtyWidth.ts:91-118` | ✅ direct |
| Initial debounced send | `usePtyWidth.ts:136-140` | ✅ direct |

> Line numbers marked "trace" come from an automated code trace and may drift by a few lines; the
> "verified" rows were read directly for this report.

---

## 4. Best practices (researched)

### 4.1 The canonical fix: **set the size at spawn, don't race a post-spawn resize**

This is the strongest and most consistent finding. PTY libraries are explicitly designed to take
the size up front:

- **node-pty**: `pty.spawn(shell, [], { cols: 80, rows: 30, ... })` — size is a spawn argument;
  `resize()` is only for *later* changes. ([node-pty](https://github.com/microsoft/node-pty))
- **creack/pty (Go)**: `StartWithAttrs` "will resize the pty to the specified size **before**
  starting the command if a size is provided" — precisely to avoid the startup race.
  ([creack/pty](https://pkg.go.dev/github.com/creack/pty))
- **`portable-pty`** (the crate AgentMux uses) takes `PtySize` in `openpty()` — AgentMux just
  hardcodes it to `200×25` instead of using the known pane width.

The matching failure mode is documented: if you spawn at a default size and resize ~tens of ms
later, the child may not have installed its `SIGWINCH` handler yet and **the resize is silently
lost**; `bash` keeps `COLUMNS=80` until `checkwinsize` fires (only after external commands, not
builtins), so it "never self-corrects."
([R. Koucha — SIGWINCH](http://www.rkoucha.fr/tech_corner/sigwinch.html))

A **near-identical real-world bug and fix** exists: *hermes-ide #113* — "PTY/xterm column mismatch
causes garbled history navigation and stale remnants." Their fix is the template for ours:
1. `create_session` accepts `initial_rows`/`initial_cols`; the **frontend estimates dimensions
   from window size + font settings** and passes them in.
2. **Re-send resize on `shell_ready`** as a safety net.
3. **NaN/`isFinite()` guard** before using proposed dimensions.
([hermes-ide #113](https://github.com/hermes-hq/hermes-ide/issues/113))

xterm.js maintainers note the async resize race "cannot be avoided completely, but can be narrowed
to a small buffer window" — seeding at spawn is what narrows it.
([xterm.js #1914](https://github.com/xtermjs/xterm.js/issues/1914))

### 4.2 Readiness signal vs. retry/backoff

- **No universal winner.** Event-driven readiness is precise but introduces ordering hazards that
  must be guarded with conditional state checks (the classic PostgreSQL startup-signal race).
  ([Event-Driven.io](https://event-driven.io/en/dealing_with_race_conditions_in_eda_using_read_models/))
- **Retry must be disciplined:** bounded, *failure-classified* (retry transient like
  "not running"; never retry permanent errors), and **jittered** to avoid synchronized retry
  storms. ([AWS Builders' Library](https://aws.amazon.com/builders-library/timeouts-retries-and-backoff-with-jitter/))
- **Most resilient designs combine both:** a signal-aware consumer backed by bounded, jittered
  retries — exactly the shape we want here (seed at spawn → gate on a *true* ready signal →
  bounded jittered retry as a backstop).

### 4.3 Frontend dimension hygiene

- Guard `proposeDimensions()`/computed cols against `NaN` and non-positive values — `NaN < 10` is
  `false`, so naive floors don't catch it. `usePtyWidth.computeCols` is mostly safe
  (`Math.floor`, `Math.max(1, ..)`, `MIN_COLS` floor) but `readFontSizePx` should keep its finite
  check. ([hermes-ide #113](https://github.com/hermes-hq/hermes-ide/issues/113))
- Coalesce drag resizes (already done via the 150 ms debounce). ([xterm.js #1914](https://github.com/xtermjs/xterm.js/issues/1914))

---

## 5. Recommended fix (layered)

Implement in order; **(A) alone eliminates the symptom for the common case.** (B) and (C) make the
remaining edge cases correct and quiet.

### (A) Seed the PTY size at spawn — *primary fix*

The frontend already knows the width at launch (Phase 3 runs after the pane element exists, and
`computeCols` is pure). Thread the initial cols/rows down to `openpty`:

- **Frontend** (`flows/launch-flow.ts:277-299`): before `ControllerResyncCommand`, compute
  `{rows, cols}` from the pane element (reuse `usePtyWidth`'s `computeCols`/`readFontSizePx`,
  exported via `__test__`) and pass them into the resync — either through `rt_opts` or as
  `term:rows`/`term:cols` block-meta keys.
- **Backend** (`shell.rs:320-431`): `start()` currently **ignores** its options arg (note the
  `_rt_opts` underscore). Read `rows`/`cols` from there (or from `block_meta`), clamp to sane
  bounds, and use them for `pty_size` instead of the hardcoded `200×25`. Fall back to the default
  when absent (headless/programmatic spawns).

Result: the PTY is **born at the right width**; the post-spawn resize becomes a *correction for
later changes*, not a load-bearing race.

### (B) Make `"running"` a truthful readiness signal — *correctness*

Move `self.publish_status()` (`shell.rs:357`) to **after** `inner.input_tx = Some(..)`
(`shell.rs:367`). Because `input_tx` is unbounded, a resize sent the instant `"running"` arrives is
then guaranteed to enqueue safely (it drains when the loop starts). This closes Layer B with a
two-line move.

> ⚠️ Audit consumers of `controllerstatus: "running"` first. Today it means "process status set to
> running"; after the move it means "running **and** ready for input." `useControllerStatusEvents`
> only logs on `"running"`, so the risk is low — but confirm nothing treats the earlier semantics
> as "process is alive" for liveness/timeout purposes.

Then, in `usePtyWidth`, **gate the initial send on the `controllerstatus: "running"` event** (via
the existing `waveEventSubscribe` pattern used by `useControllerStatusEvents.ts:22-40`) instead of
firing blindly at `mount + 150 ms`. User-driven `ResizeObserver` sends remain immediate.

### (C) Harden the retry backstop — *defense in depth*

Once (A)+(B) land, retries should rarely fire. Keep them as a backstop but:
- **Classify:** only retry on transient errors (`"controller is not running"` / `"no controller
  for block"`); do **not** retry permanent failures.
- **Jitter:** add randomized jitter to the 500/1000 ms steps to avoid synchronized retries when
  many panes mount together. (Scripts can't call `Math.random()` in workflows, but app runtime
  can — use `Math.random()` here.)
- Optionally widen to ~3–4 attempts with a small cap; with (A)+(B) this is belt-and-suspenders.

### Why not "just widen the backoff"?

It's the option the original comment reaches for, but it only shrinks the dead window — it never
closes it, it adds latency before the correct width appears, and it leaves the PTY at 200 cols for
the whole retry window on every cold start. (A) removes the race; widening only manages it.

---

## 6. Suggested change set

| Order | File | Change |
|---|---|---|
| A1 | `frontend/app/view/agent/hooks/usePtyWidth.ts` | Export a reusable `computeColsForElement(el)`; keep `computeCols` pure. |
| A2 | `frontend/app/view/agent/flows/launch-flow.ts` | Compute `{rows, cols}` pre-resync; pass via `rt_opts`/block-meta to `ControllerResyncCommand`. |
| A3 | `agentmux-srv/src/backend/blockcontroller/shell.rs` | Read rows/cols from options/meta in `start()`; clamp; use for `pty_size` (replace hardcoded `200×25`); default-fallback. |
| B1 | `agentmux-srv/src/backend/blockcontroller/shell.rs` | Move `publish_status()` (357) to after `input_tx = Some(..)` (367). |
| B2 | `frontend/app/view/agent/hooks/usePtyWidth.ts` | Gate the *initial* send on the `controllerstatus:"running"` event; keep observer-driven sends immediate. |
| C1 | `frontend/app/view/agent/hooks/usePtyWidth.ts` | Classify retryable errors; add jitter; small cap. |

Tests: extend `usePtyWidth` `__test__` unit checks (cols math + NaN guard); add a backend test
that `start()` honors a passed-in size and that `"running"` is published only after `input_tx`
exists.

---

## 7. References

- node-pty — spawn-time `cols`/`rows`: https://github.com/microsoft/node-pty
- creack/pty — resize-before-start: https://pkg.go.dev/github.com/creack/pty
- R. Koucha, "Playing with SIGWINCH" — lost-signal-at-startup: http://www.rkoucha.fr/tech_corner/sigwinch.html
- hermes-ide #113 — near-identical PTY/column mismatch bug + fix: https://github.com/hermes-hq/hermes-ide/issues/113
- xterm.js #1914 — resize roundtrip race, "narrow not eliminate": https://github.com/xtermjs/xterm.js/issues/1914
- AWS Builders' Library — timeouts, retries, backoff, jitter: https://aws.amazon.com/builders-library/timeouts-retries-and-backoff-with-jitter/
- Event-Driven.io — race conditions in EDA / guarded transitions: https://event-driven.io/en/dealing_with_race_conditions_in_eda_using_read_models/
