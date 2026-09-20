# REPORT: Shell drawer zoom — why history alignment feels "all over the place"

**Date:** 2026-09-19
**Status:** proposed — analysis only; this report changes no code.
**Related:**
`docs/specs/SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md` (drawer-only initial-paint zoom bug, fixed),
`docs/specs/SPEC_AGENT_SHELL_PSREADLINE_THAW_VISIBLE_RESIZE_2026-08-14.md` (Windows PSReadLine cursor-tracking thaw, fixed),
`docs/specs/SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md` (the two-tier persistence model this report examines),
`docs/reports/REPORT_ZOOM_BINDINGS_AUDIT_2026_09_06.md` (catalogs the drawer's hand-rolled zoom gesture as one of six duplicated sites),
`docs/archive/TERM_JUMBLE_STRUCTURED_2026_05_25.md` (prior finding that xterm/ConPTY reflow under repeated resize is fragile even with ConPTY's own reflow option enabled).

**User framing (2026-09-19):** "the drawer is a fairly uncomfortable control. Ideally we'd like a smooth experience, where zoom opens normally and if you zoom out/in it will properly align the console history. currently it is all over the place, it could be because of it need an architecture rethink, something more elegant that handles zoom and generalizes across platforms."

---

## 0. Headline

The live zoom→resize→PTY-resize chain is **not** the bug. It is sound, shared between the drawer and the standalone terminal pane, and already cross-platform (`portable_pty` abstracts ConPTY vs. Unix `ioctl` behind one `.resize()` call). Chasing a "PTY doesn't know the new size" theory would be a dead end.

The two real problems are architectural, and both point at the same root cause: **the Shell drawer's terminal (`AgentShellSubblock.tsx`) was built as a standalone spike outside the pane/ViewModel abstraction the rest of the terminal system uses**, not as a lesser copy of the same design.

1. **Scrollback replay has a width-reconciliation gap** (§3) — the most concrete, code-level candidate for "history is all over the place." One of the two persistence tiers reconciles terminal width before replaying old output; the other does not.
2. **The drawer duplicates zoom-gesture logic outside the shared `zoom.ts` module and has no `BlockComponentModel` registration** (§4) — so it drifts from the canonical step size, and it is invisible to "zoom all panes."

Both are fixable without inventing new zoom mechanics — the fix is convergence onto machinery that already exists and already works for the standalone terminal pane, not a new abstraction.

## 1. What is NOT broken: the live resize chain

Zoom (Ctrl+Wheel) changes `term:zoom` on the shell sub-block's own meta (`AgentShellSubblock.tsx:154-163`), which feeds a font-size memo:

```ts
// AgentShellSubblock.tsx:99, 298-301
const BASE_FONT_SIZE = 13;
const termFontSize = createMemo(() => {
    const paneZoom = props.agentPaneZoom() || 1;
    return Math.max(4, Math.min(64, Math.round((BASE_FONT_SIZE * termZoom()) / paneZoom)));
});
```

Applying it goes through the exact same code as the standalone terminal pane (`term.tsx:245-253` is line-for-line the same pattern):

```ts
// AgentShellSubblock.tsx:309-324 / term.tsx:245-253 — identical shape
createEffect(() => {
    const fs = termFontSize();
    if (termWrap?.terminal && loaded) {
        termWrap.terminal.options.fontSize = fs;
        termWrap.handleResize();
    }
});
```

`handleResize()` recomputes cols/rows via `fit-addon`, calls `terminal.resize()` only if they changed, and — only on an actual change — notifies the backend:

```ts
// termwrap.ts:675-683 (handleResize) → :687-715 (customFit) → :808-816 (sendTermSize)
handleResize() {
    const oldRows = this.terminal.rows, oldCols = this.terminal.cols;
    this.customFit();
    if (oldRows !== this.terminal.rows || oldCols !== this.terminal.cols) {
        this.sendTermSize();
    }
}
```

The backend applies it unconditionally, through the platform-agnostic `portable_pty` crate:

```rust
// agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs:1096-1106
if let Some(ref size) = input.term_size {
    let pty_size = PtySize { rows: size.rows as u16, cols: size.cols as u16, .. };
    if let Err(e) = master.resize(pty_size) {
        tracing::debug!("PTY resize error: {}", e);
    }
}
```

`master` is a `portable_pty::MasterPty` — this one `.resize()` call dispatches to ConPTY on Windows or `ioctl(TIOCSWINSZ)` on Unix internally. There is no platform branch in this repo's own code for the resize call itself, and no "frontend renders bigger while the PTY still thinks it's the old size" gap: `sendTermSize()` only fires when `terminal.resize()` actually changed cols/rows, and the backend applies whatever it's told with no staleness check.

**This chain is identical for the drawer and the standalone pane.** Whatever is misaligning history is not a drawer-specific live-resize defect — it is somewhere else.

## 2. The one genuine platform-specific piece — and it isn't the bug either

`termwrap.ts:442` gates a Windows-only "thaw" resize pair (`cols+1` → `cols`, sent to the backend only, never applied to the visible grid) behind `PLATFORM === PlatformWindows`. This exists because PSReadLine's cursor-tracking desyncs after certain resizes on ConPTY specifically — bash/zsh/fish handle `SIGWINCH` cleanly on their own (comment at `termwrap.ts:435-438`). This is legitimate, narrow compensation for a real Windows-only behavioral difference, already shipped and covered by its own spec (`SPEC_AGENT_SHELL_PSREADLINE_THAW_VISIBLE_RESIZE_2026-08-14.md`). It is not evidence the resize pipeline itself needs a cross-platform rethink — the pipeline is already one code path; only this one narrow compensation branches on OS.

## 3. Where the misalignment most likely comes from: replay-time width mismatch

`SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md` documents two persistence tiers, shared verbatim between the drawer and the standalone pane (that spec says so explicitly, §"both... go through the exact same `ShellController`... and the exact same frontend `TermWrap`"):

1. A periodic **snapshot** (`SerializeAddon.serialize()`), tagged with the terminal size it was captured at.
2. Raw **incremental PTY bytes** from an offset onward — literal ANSI/cursor-control sequences the shell emitted at whatever width was live when they were written.

Reading `termwrap.ts:865-896` (`loadInitialTerminalData`) shows these two tiers are **not treated symmetrically**:

```ts
// termwrap.ts:869-887 — snapshot path: reconciles size before AND after replay
const fileTermSize: TermSize = cacheFile.meta["termsize"];
if (fileTermSize.rows != curTermSize.rows || fileTermSize.cols != curTermSize.cols) {
    this.terminal.resize(fileTermSize.cols, fileTermSize.rows);   // resize DOWN to snapshot's width
    didResize = true;
}
this.doTerminalWrite(cacheData, ptyOffset);                        // replay AT that width
if (didResize) {
    this.terminal.resize(curTermSize.cols, curTermSize.rows);      // resize back, letting xterm reflow
}

// termwrap.ts:889-895 — raw delta path: NO reconciliation at all
const { data: mainData, fileInfo: mainFile } = await fetchMuxFile(this.blockId, TermFileName, ptyOffset);
if (mainFile != null) {
    await this.doTerminalWrite(mainData, null);                    // replayed at WHATEVER width is current now
}
```

The raw delta bytes are literal output the shell produced for a specific column width (cursor addressing, line-wrap points, and any width-dependent control sequences are all computed relative to that width at write time). If the terminal's current width at replay time differs from what was live when those bytes were generated, replaying them verbatim has no correctness guarantee — unlike the snapshot path, which explicitly resizes to the recorded width, replays, then resizes back and lets xterm reflow cleanly.

**This is exactly the shape of "zoom out/in and history doesn't align":** a zoom change is persisted via `SetMetaCommand`, which changes `term:zoom` → font size → cols/rows on the *next* resize. If the drawer or pane is closed and reopened (or the controller resyncs) between when some raw delta bytes were written and when they get replayed, they now replay against a different column count than the one they were emitted for, with none of the snapshot path's temp-resize-then-reflow protection. Prior work already found xterm/ConPTY reflow to be fragile under resize even with ConPTY's own `reflowCursorLine: true` enabled (`docs/archive/TERM_JUMBLE_STRUCTURED_2026_05_25.md:51` — enabling it "made it worse"), reinforcing that a wrong-width replay is not something xterm silently self-corrects.

**This gap is not covered by any existing spec.** `SPEC_AGENT_SHELL_ZOOM_SEED_RACE` fixes an initial-paint flash, not history alignment. `SPEC_AGENT_SHELL_PSREADLINE_THAW_VISIBLE_RESIZE` fixes a live cursor-tracking desync on Windows only, not replay. Neither touches `loadInitialTerminalData`.

**Verification status:** I confirmed this asymmetry by reading `termwrap.ts:855-896` directly. I have **not** reproduced the visible misalignment live in a running instance — this is the strongest code-level candidate given the evidence, not a confirmed root cause from observation. Recommend confirming with a repro (§6) before treating it as settled.

## 4. The architectural gap behind "needs a rethink": the drawer isn't a pane

The standalone terminal pane's zoom reads from `TermViewModel` (`termViewModel.ts:79,251-277`), which is a real, registered `ViewModel` — the pane is a first-class citizen of the app's `BlockComponentModel` registry (`block-component-registry.ts`).

`AgentShellSubblock.tsx` embeds `TermWrap` directly with **no ViewModel and no `BlockComponentModel` registration** — confirmed directly (no match for `BlockComponentModel|registerBlockComponentModel|new TermViewModel` anywhere in that file). Its own header comment calls it a "Phase 0 spike" (`:5`), and it never graduated past that.

Concrete consequences of not being registered:

- **`getBlockZoom()` can't find it.** `zoom.ts:88-98`:
  ```ts
  function getBlockZoom(blockId: string): number | null {
      const bcm = getBlockComponentModel(blockId);
      if (!bcm?.viewModel) return null;
      ...
  }
  ```
  `stepAllPanes()` (Ctrl+Shift+Scroll, "zoom all panes") iterates `getAllBlockComponentModelEntries()` — the drawer's shell sub-block is invisible to it, and to any future feature built on that registry.

- **The seed-race bug that needed its own spec** (`SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md`) existed *only* because the drawer couldn't reuse the "seed zoom value before constructing the terminal" pattern the pane's ViewModel lifecycle already provides for free. The fix (`waitForMuxObjectSettled`, `AgentShellSubblock.tsx:113-120`) is a bespoke re-solution of a problem the pane's architecture doesn't have.

- **Duplicated, drifted gesture logic.** Both `term.tsx:231` (the pane) and `AgentShellSubblock.tsx` hand-roll their own Ctrl+Wheel handler with `STEP = 0.1`, instead of calling the shared `zoomBlockIn`/`zoomBlockOut` helpers in `zoom.ts`, whose own wheel constant is `WHEEL_STEP = 0.05` (`zoom.ts:44-45`). **Correction to the initial pass of this investigation:** this drift is not unique to the drawer — the standalone pane makes the identical mistake. `REPORT_ZOOM_BINDINGS_AUDIT_2026_09_06.md` already catalogs both as two of six sites (of 13 total) that duplicate `zoom.ts`'s step/clamp/apply logic instead of calling the shared helpers, and lists consolidating them as an open, unbundled follow-up. So this specific inconsistency is a symptom of a systemic duplication problem the audit already flagged, not something the drawer introduced on its own — but the drawer is one of the sites contributing to it, and unlike the pane, the drawer also lacks the registry membership that would make consolidation straightforward.

## 5. What "architecture rethink" should mean here

Given the evidence, "something more elegant that handles zoom and generalizes across platforms" is not a request for new zoom mechanics — the mechanics (fit-addon, `terminal.resize()`, `sendTermSize()`, `portable_pty`) are already elegant and already platform-agnostic. It is a request to stop the drawer from being a second, parallel implementation of them:

1. **Give the drawer's shell sub-block a real `BlockComponentModel`/ViewModel registration**, even a minimal one, so it participates in `getBlockZoom()`/`stepAllPanes()` like every other zoomable surface, and so future zoom-related work (or any other pane-registry-based feature) doesn't need a bespoke drawer-specific path. This would very likely have prevented the seed-race bug from needing its own spec.
2. **Collapse the drawer's and the pane's Ctrl+Wheel handlers onto `zoom.ts`'s `zoomBlockIn`/`zoomBlockOut`**, matching `REPORT_ZOOM_BINDINGS_AUDIT_2026_09_06.md` §3's already-proposed consolidation (which currently scopes to "System 3" sites in general, not drawer-specific — this would be one instance of executing that existing recommendation).
3. **Fix the raw-delta replay path to reconcile width the same way the snapshot path does** (§3) — resize to whatever width the delta bytes were captured against (this needs to be tracked; today only the snapshot's `termsize` meta is recorded, not a per-delta-write width) before replay, then resize back. This is the change most directly responsible for "history alignment," independent of #1 and #2.

None of the three require inventing a new zoom system. They require making the drawer stop being the one surface that isn't using the one the rest of the app already has.

## 6. Suggested next step (not done here)

This report is analysis only. Before implementing §5, get a live repro of the alignment symptom (zoom mid-session, then either reload the drawer or trigger a resync, and diff the terminal content against what a snapshot-path-only replay would produce) to confirm §3 is the actual mechanism the user is seeing, rather than a second, undiscovered bug in the same neighborhood. The fastest way to falsify or confirm §3: temporarily record termsize on every delta write (not just snapshots) and log a mismatch warning at replay time — if it fires when the symptom is reproduced, §3 is confirmed as (at least one) root cause.

---

# ADDENDUM 2026-09-20 — live instrumented repro; §3 falsified, real root cause found

**Status change:** the §6 "suggested next step" was carried out. A CDP-instrumented
build was run against the live app and the drawer's actual buffer state measured at
mount. **§3's width-reconciliation theory is falsified as the cause of the reported
symptoms**, and a different, simpler defect is confirmed.

**User framing (2026-09-20):** "when it opens it is all black. it needs to open to the
first line. there is something seriously wrong between the window it is placed in and
the terminal ... I cannot scroll up."

## A1. Method

`dlog` was forced always-on (the `debug()` namespace gate reads localStorage at module
load, and CDP `Page.reload()` breaks this CEF app's `window.api` native bridge — see
the note at `termwrap.ts`'s dlog definition). A one-shot CDP `Runtime.evaluate` harness
drove the real repro gestures (drawer close/reopen, wheel scroll, typed input) and
captured console output, against a packaged portable build.

## A2. Measurements

Drawer remount with a *small* history (fresh shell, 320 bytes):

```
delta-replay curSize=80x24 bytes=320   bufLenBefore=24
delta-replay done                      bufLenAfter=24
resize reflow 42x9  lenBefore=24 lenAfter=9  baseYBefore=0 baseYAfter=0
```

Drawer remount with a *real* history (600 printed lines, 31877 bytes):

```
delta-replay curSize=80x24 bytes=31877 bufLenBefore=24
delta-replay done                      bufLenAfter=1208
resize reflow 42x9  lenBefore=1208 lenAfter=1209  baseYBefore=1184 baseYAfter=1200
```

Cold start of the whole app, same drawer:

```
delta-replay curSize=80x24 bytes=32604 bufLenBefore=24
delta-replay done                      bufLenAfter=1208
resize reflow 42x9  lenBefore=1208 lenAfter=1198 baseYBefore=1184 baseYAfter=1189
```

## A3. What these falsify

- **"The mount-time resize truncates history" — false.** With real history the row
  count is preserved across the fit (`1208 -> 1209`). The `24 -> 9` drop in the
  small-history case is xterm trimming *blank* rows, not content. An early reading of
  this investigation's own diagnostic output claimed otherwise; it was wrong.
- **"The drawer has no scrollback" — false.** It reaches `baseY=1189` after a cold
  start and wheel-scrolls correctly through it. The earlier "zero scrollback" reading
  came from `.xterm-viewport.scrollHeight`, which is meaningless in xterm 6: the
  scrollbar is a transform-driven **overlay** reserving no layout (as `customFit`'s own
  comment in `termwrap.ts` already documents), and the WebGL renderer emits no DOM rows,
  so `.xterm-screen` has no text either. **The only valid source of truth for buffer
  state is the `Terminal` object**, not the DOM.
- **§3's raw-delta width mismatch is real but is not the reported symptom.** Replay does
  happen at the wrong width (see A5), but it reflows losslessly.

## A4. Confirmed root cause of "cannot scroll up to the beginning"

`AgentShellSubblock.tsx` hardcoded `scrollback: 2000` and never read the
`term:scrollback` setting, while `term.tsx` resolved it from settings + per-block meta
with a 50000 clamp. So raising the setting deepened terminal panes and did nothing for
the agent Shell drawer.

The measurements quantify the impact: at the drawer's narrow width (42 cols) the 600
test lines occupy **1200 scrollback rows — 2 rows per logical line**. xterm counts
scrollback in *display rows*, so the 2000-row cap holds only ~1000 lines of a real
session. Trimming is also per-row, which is exactly why the topmost reachable line
appears **cut through its middle** rather than at a line boundary — the specific detail
in the original bug report.

**Fix:** `frontend/app/view/term/termscrollback.ts` — one `resolveTermScrollback()`
helper, adopted by both `term.tsx` and `AgentShellSubblock.tsx`, so the two surfaces
cannot drift again. Unit tests in `termscrollback.test.ts`; a drawer-level regression
test in `AgentShellSubblock.test.tsx` that fails against the old hardcode.

Note left in place: the settings UI (`terminal-section.tsx`) accepts up to 100000 while
the resolver clamps at 50000, so a larger configured value is silently reduced. Left
as-is to keep this change behavior-preserving; raising it is a memory-footprint call.

## A5. Secondary defect — replay happens at the default 80x24, then reflows

Every trace above shows `delta-replay curSize=80x24`: `init()` awaits
`loadInitialTerminalData()` *before* the font load and `customFit()`, so history is
always written into xterm's default 80x24 grid and only then resized to the real
geometry. Two consequences:

1. **Performance.** The subsequent `terminal.resize()` reflows the *entire* replayed
   scrollback — ~95 ms for 1208 rows here. It scales linearly, and now that scrollback
   legitimately reaches 50000 rows this becomes a synchronous main-thread stall on
   every mount.
2. **Correctness.** History is wrapped at 80 cols and re-wrapped to the real width,
   rather than being laid out at the real width once — the §3 concern, reached by a
   different route.

The font-load-before-fit ordering is deliberate (see the long comment in `init()`) and
must be preserved, so the fix is to hoist the font-load + `customFit()` block above
`loadInitialTerminalData()`, keeping `sendTermSize()` / `hasResized` /
`resyncController("init")` in their current relative order.

## A6. Not reproduced: "all black on open"

A forced drawer remount and a full application cold start, each with and without
history, both painted content immediately. The one genuinely black observation was a
drawer whose shell had not yet emitted anything, during app startup. The reporting
condition is therefore still unidentified; it should not be assumed to be the same bug
as A4.

## A7. Incidental fix — the Solid component test suite could not run at all

`vite-plugin-solid` sets `needHmr = command === "serve" && mode !== "production" &&
options.hot !== false` (dist/esm/index.mjs:164), all true under Vitest, so it injected
the `/@solid-refresh` virtual module and **every** `.test.tsx` rendering a Solid
component failed at import with `Received 'file:///@solid-refresh'` before running a
single assertion. `vite.config.ts` now passes `solid({ hot: !process.env.VITEST })`.
Full suite after the change: 287 files / 4295 tests passing.

---

# ADDENDUM B 2026-09-20 — "Ctrl+Wheel zoom does nothing in the Shell drawer"

**Status:** root-caused and fixed. The defect is in `MOS` (the object store), not in
the drawer, and it affects any component whose object atom has no other refCount
holder — the drawer is simply the place it reliably bites.

**User report:** "i notice zoom and the scroll bar (in the shell drawer opened) are
not working."

## B1. Root cause

`getMuxObjectAtom(oref)` captured its cache entry once, at atom-creation time:

```ts
const wov = getMuxObjectValue<T>(oref);
const atom = () => wov.getData().value;   // captured
```

`cleanMuxObjectCache()` runs every 30s and deletes any entry with `refCount === 0`
whose `holdTime` has lapsed. `getMuxObjectAtom` deliberately takes **no** refCount —
unlike `useMuxObjectValue`, it has no cleanup hook to release one. So its entries are
permanent eviction candidates.

After the first eviction the two sides diverge:

- the component keeps reading the **evicted** `wov`'s signal, which nothing writes to
  ever again;
- `updateMuxObject(update)` calls `getMuxObjectValue(oref)`, which **re-creates** the
  entry and writes the new value there — and logs `"MuxObj updated <oref>"`.

The store therefore looks completely healthy from the outside (the push arrives, is
accepted, and is logged) while every holder of the stale closure is frozen on the
pre-eviction value, permanently and silently.

## B2. Why the drawer specifically

Zoom is a meta round-trip: the Ctrl+Wheel handler only calls
`RpcApi.SetMetaCommand({"term:zoom": next})` and waits for the backend's
`waveobj:update` echo to drive `termZoom()` → `termFontSize()` → the font-size effect.
Neither `AgentShellSubblock.tsx` nor `term.tsx` nor `zoom.ts` updates MOS
optimistically (`zoom.ts:100-107` documents this explicitly).

The agent Shell drawer's sub-block is **headless** — it is not in the layout, so no
other code path holds it via `useMuxObjectValue`, its refCount stays 0, and it is
evicted within a GC tick or two of being created. A top-level terminal pane's block
*is* held by layout code, keeps `refCount > 0`, is never evicted, and its zoom keeps
working. Reopening the drawer appeared to "fix" zoom only because remounting rebuilt
the atom against the fresh cache entry — which is also why zoom changes made while the
drawer was open showed up on the next open.

## B3. Evidence

Live capture, with `updateMuxObject`'s own logging un-filtered, driving Ctrl+Wheel on
both surfaces in the same window:

```
08:18:01.348  MuxObj updated block:2bbdcfc2…   (drawer sub-block)   → no reaction
08:18:02.260  MuxObj updated block:2bbdcfc2…   (drawer sub-block)   → no reaction
08:18:03.663  MuxObj updated block:1097ce7a…   (terminal pane)
08:18:03.674      customFit sync fontSize=20 … + resize reflow 25x39   (+11ms)
08:18:04.566  MuxObj updated block:1097ce7a…   (terminal pane)
08:18:04.568      customFit sync fontSize=21 …                          (+2ms)
```

Both blocks receive the push; only the pane reacts. That isolates the fault to the
frontend's atom wiring rather than to event delivery, scoping, or the version guard —
each of which was tested and cleared first.

Two independent checks confirmed the drawer's own reactive chain is otherwise sound:
the handler does run (a dispatched Ctrl+Wheel comes back `defaultPrevented: true`,
which only happens past the `subBlockId()` guard), and the drawer's font *does* update
live when `props.agentPaneZoom()` changes (`customFit sync fontSize=17` at 08:12:28) —
so `wrapLoaded()`, the memo and the effect all work. Only the `termZoom()` input was
dead.

## B4. Fix — two halves, both required

**Half 1 — late-bind the read.** `getMuxObjectAtom` resolves the cache entry on every
read instead of capturing it:

```ts
const atom = () => getMuxObjectValue<T>(oref).getData().value;
```

This keeps the read reactive (`getData()` tracks whichever signal is current) and lets
an evicted entry transparently re-fetch, at the cost of one Map lookup per read. `_set`
resolves the same way.

**Half 2 — pin the entry for the caller's reactive lifetime.** Half 1 alone does NOT
fix the reported symptom, and the first attempt at this fix shipped with only half 1
before a live check caught it. A SolidJS memo/effect re-runs only when a signal *it is
subscribed to* changes. Once an entry is evicted and `updateMuxObject` re-creates it,
the new entry carries a **brand new signal** that no existing dependent is subscribed
to — so nothing ever re-triggers them, they never re-read, and late-binding never gets
a chance to help. Preserving signal identity for as long as something depends on it is
what reactivity actually requires:

```ts
const owner = getOwner();
if (owner != null) {
    const pinned = getMuxObjectValue<T>(oref);
    pinned.refCount++;
    onCleanup(() => { pinned.refCount--; });
}
```

Guarded on `getOwner()` deliberately: outside a reactive root there is no cleanup hook
to return the refCount, and an unconditional increment would pin every oref forever,
converting a staleness bug into an unbounded cache leak. Eviction behavior is otherwise
unchanged — entries are still reclaimed once their owners dispose.

**Negative controls** (`mos.test.ts`), each failing against the code without its half:

| Removed | Failing assertion |
|---|---|
| late-binding | `expected 1 to be 1.5` — atom frozen on the pre-eviction value |
| the pin | `expected [ undefined, 1 ] to include 1.5` — dependent effect goes permanently silent after one GC tick |

A third test asserts the pin is released on owner disposal, so the entry remains
collectable.

## B5. Scrollbar — not a defect

The drawer's overlay scrollbar works: sampled during a real wheel event it transitions
`invisible scrollbar vertical fade` → `visible scrollbar vertical`. What makes it feel
broken is proportion — a 9-row viewport over ~1800 rows of scrollback yields a ~13px
slider that moves a fraction of a pixel per wheel tick, and it is hidden whenever idle.
That is a UX question (minimum slider size / persistent scrollbar affordance), not a
bug.

## B6. Investigation note

Five hypotheses were tested and falsified before the real cause: console.log volume
saturating the input queue; a background/occluded window; a wedged input pipeline
(that instance's window was simply **minimized** — `screenX/Y = -32000`); the sub-block
not being subscribed for object updates (`initGlobalEventSubs` subscribes with no
scope, i.e. `allscopes`); and the backend not broadcasting on setmeta (it does, with a
bumped version). The decisive step was noticing the diagnostic capture itself filtered
console output to `zoom-diag|wave:termwrap|error`, so `"MuxObj updated"` could never
have appeared — several "no events arrive" readings were artifacts of the tool, not of
the app. Widening that filter produced the B3 timeline immediately.
