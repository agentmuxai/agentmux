# Agent shell drawer: zoom twitches ~500ms after every open (parent pane's zoom, not the shell's own)

**Date:** 2026-08-14
**Status:** Draft — root-cause analysis, no code written yet.
**Owner:** Agent1
**Area:** Agent pane / shell drawer (`AgentShellSubblock`), terminal zoom

---

## 1. Problem

Reported: "in the agent pane, when opening the shell, the zoom level
twitches one level every time it opens, after about 500ms or so, happens
every time." Reproducible, not flaky — happens on every open, at a
consistent delay.

This is the **same bug class**, in the **same component**, as
`docs/specs/SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md` (PR #2522,
shipped `1d15a77cb`) — but that fix closed the race for the shell's *own*
persisted zoom (`term:zoom` on the shell sub-block). This is a second,
structurally identical, still-open race one level up: the shell's font-size
formula also depends on the **parent agent pane's** zoom, which was never
given the same seed-before-use treatment.

## 2. Current behavior (root cause)

### 2.1 The formula has two independent zoom inputs, only one of which was fixed

`AgentShellSubblock.tsx:142-145`:

```ts
const termFontSize = createMemo(() => {
    const paneZoom = props.agentPaneZoom() || 1;
    return Math.max(4, Math.min(64, Math.round((BASE_FONT_SIZE * termZoom()) / paneZoom)));
});
```

`termZoom()` (the shell's own `term:zoom`) is now correctly seeded before
`TermWrap` construction — that's what the 08-10 spec fixed. `paneZoom` —
`props.agentPaneZoom`, passed as `agentPaneZoom={zoomFactor}` at the call
site (`agent-view.tsx:2262`) — is **not** seeded by `AgentShellSubblock` at
all. It's read live, straight from the parent's own memo.

### 2.2 The parent's zoom memo has the exact same async-atom shape the 08-10 fix was built to avoid

`agent-view.tsx:1791-1796`:

```ts
const zoomFactor = createMemo(() => {
    const meta = block()?.meta;
    const z = meta?.["term:zoom"];
    if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
    return Math.max(0.5, Math.min(2.0, z));
});
```

`block` here is `model.blockAtom` (`agent-model.ts:67`):
`WOS.getWaveObjectAtom<Block>(\`block:${blockId}\`)` — the identical async
WOS-atom mechanism the shell's own `subBlockAtom` uses, and identical to
what `SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md` §2.2 already
documented in detail for that case: seeds to `null`/loading, resolves later
via a `GetObject` round-trip. Before it resolves, `block()` is falsy →
`zoomFactor()` returns the default `1.0`, regardless of what the pane's
actual persisted zoom is.

### 2.3 Why the parent pane's own rendering doesn't show this (misleadingly, since it made this look "already fixed")

The 08-10 spec's own §3 notes the agent pane's zoom *did* have this race
historically, and was fixed by seeding the value into the block's *initial*
meta before creation (`command-registry.ts:416-450`, commit `69de7d5e2`).
That fix guarantees that **whenever** `block()` first resolves, the value it
carries is already correct — it does not, and structurally cannot, make
`block()` resolve synchronously. The `undefined → default → real value` gap
still exists on every pane mount; it's just that the *pane's own* content
(CSS `zoom` on `.agent-view`) transitioning through that gap is a single
instant snap on a whole pane, with no forced re-render/re-layout of
character cells — far less perceptible than what happens downstream in the
shell (§2.4). That's why this wasn't caught as a bug for the pane itself:
the race is real there too, just invisible.

### 2.4 Why it's visible and jarring specifically in the shell

Same correction-effect mechanism the 08-10 spec documented for the shell's
own zoom, still present by design for live Ctrl+Wheel updates
(`AgentShellSubblock.tsx:153-168`):

```ts
createEffect(() => {
    const fs = termFontSize();
    const loaded = wrapLoaded();
    if (termWrap?.terminal && loaded) {
        termWrap.terminal.options.fontSize = fs;
        termWrap.handleResize();
    }
});
```

When `block()` (parent pane) resolves shortly after the shell has already
mounted and painted at the default-`paneZoom` font size, `termFontSize()`
recomputes, this effect fires, and — same as the original bug —
`handleResize()` internally forces `core._renderService.clear()` then
`terminal.resize(...)`: a real render-state mutation, not a CSS artifact,
with no transition to soften it (`_composer-strip.scss` has none for this
drawer, per the 08-10 spec's own confirmation). One firing, one visible
"twitch," exactly the reported shape.

### 2.5 Why it's consistently ~500ms, not flaky

The shell drawer is typically opened shortly after the agent pane itself,
which means the pane's own `blockAtom` `GetObject` round-trip is often
still in flight when the shell mounts. Unlike the original bug (§2.2.6 of
the 08-10 spec — a genuinely flaky race window), this one resolves at
whatever this environment's consistent WOS/RPC latency is for that fetch —
observed here as a steady ~500ms, not "sometimes wrong, sometimes not."

## 3. Relationship to the 08-10 fix — why this wasn't caught by it

The 08-10 spec's own §4 requirement 4 ("no regression to per-shell scoping
— zoom stays keyed to the shell sub-block's own identity") and its fix
scope were both specific to `termZoom()`/the shell's *own* meta. `paneZoom`
was correctly identified as a separate, independent input (comment at
`AgentShellSubblock.tsx:257-261`: "the pane's zoom... and the shell's own
zoom... are independent controls, and neither should visually leak into the
other") — but "independent" only covered *value* leakage (not compounding
the two zooms together), not *timing* leakage. The parent's async-resolve
gap leaks into the shell's paint timing regardless of the two zooms being
otherwise correctly decoupled in the formula.

## 4. Goal & requirements

1. **No visible twitch on open**, matching the 08-10 fix's own bar — the
   terminal's first paint (and its first live-effect firing, if any) must
   already reflect the FINAL `agentPaneZoom()`, not a default that gets
   corrected after the fact.
2. **No new redundant fetch.** `AgentShellSubblock` must not trigger a
   second `GetObject` for the parent block — `agent-view.tsx` (or whatever
   owns `model.blockAtom`) is presumably already fetching it, same
   reasoning as the 08-10 spec's §5a P1 finding for the shell's own oref.
3. **Don't touch the pane's own zoom-seeding path** (`command-registry.ts:416-450`)
   — out of scope, already correct for its own purpose (§2.3).
4. **Live Ctrl+Wheel zoom** (both the pane's and the shell's own, while the
   shell is open) must keep working exactly as today.

## 5. Proposed fix direction

Extend the *already-proven* pattern from the 08-10 fix — `AgentShellSubblock`
already has `waitForWaveObjectSettled(oref)` (`AgentShellSubblock.tsx:98-105`),
a helper built exactly for "wait for an in-flight WOS fetch to settle
without triggering a new one." It's currently only called for the shell's
own sub-block oref (`AgentShellSubblock.tsx:311-313`, gated on
`isExistingBlock`). Call it a second time for the **parent** block's oref
before flipping `zoomSeeded(true)`:

```ts
await waitForWaveObjectSettled(WOS.makeORef("block", props.parentBlockId));
```

`AgentShellSubblock` already receives `parentBlockId` as a prop
(`agent-view.tsx:2254`), so no new prop plumbing is needed. Unlike the
shell's own oref wait, this one should run **unconditionally** (not gated
on `isExistingBlock`) — the parent pane's zoom fetch is in flight or not
independent of whether this particular shell sub-block is new or reused.
This is cheap in the already-settled case: `waitForWaveObjectSettled`
returns immediately if the loading atom isn't `null` (i.e., most opens,
where the pane finished loading before the user ever reached for the
shell) — the added wait only has teeth in the specific window this bug
report describes.

Run both waits concurrently (`Promise.all`) rather than sequentially, so a
reused shell with its own pending fetch doesn't pay both round-trips
back-to-back.

Everything else from the 08-10 fix (the `BrainSpinner` loading overlay
masking the drawer until `zoomSeeded()`, the `wrapLoaded`-gated live-update
effect) already generalizes for free — the overlay just stays up slightly
longer on the specific opens where the parent fetch is still in flight,
which is exactly the masking behavior wanted here.

## 6. Non-goals / out of scope

- Re-litigating how the pane's own zoom is seeded/persisted
  (`command-registry.ts:416-450`) — correct already, per §2.3.
- The unrelated Windows-only PSReadLine "thaw" resize
  (`termwrap.ts:396-430`, fires ~250-300ms after `TermWrap.init()` on every
  terminal on Windows) and the font-load re-fit
  (`termwrap.ts:349-365`) — both real, both pre-existing, both column/grid
  reflows rather than font-size changes. Considered and ruled out as the
  primary cause: the report specifically says "zoom level," and the timing/
  consistency (steady ~500ms, not flaky, "every time") matches the
  parent-block WOS fetch far better than either of these, which don't
  depend on network/RPC latency. Worth a sanity check during implementation
  (watch `termWrap.terminal.options.fontSize` vs `.cols` at the moment of
  the twitch) but not expected to require its own fix.
- The standalone Terminal widget (`view: "term"`) — reads the pane's own
  zoom directly (it *is* the pane), not a nested child depending on a
  parent's separately-resolving zoom, so this specific race shape doesn't
  apply to it.

## 7. Open questions

- Should `waitForWaveObjectSettled` be renamed/generalized now that it's
  called for two different orefs in the same component (e.g. inline both
  calls under one `Promise.all` with a short comment, vs. extracting a
  `waitForBothSettled(...)` helper)? Minor, decide during implementation —
  doesn't change the fix's correctness either way.
- Is there a third dependency on `props.agentPaneZoom()` anywhere else in
  `AgentShellSubblock.tsx` (e.g. `handleWheel`/live Ctrl+Wheel math) that
  should double-check its own value is post-seed, or was it already
  correctly gated by the existing `wrapLoaded`-based effect? Worth a quick
  audit pass during implementation, not expected to change the fix shape.
