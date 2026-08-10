# Agent shell drawer: font-size seed race causes zoom jerk on open

**Date:** 2026-08-10
**Status:** Proposed (investigation complete, fix not yet implemented)
**Owner:** Agent3
**Area:** Agent pane / shell drawer (`AgentShellSubblock`), terminal zoom

---

## 1. Problem

Opening the Shell drawer inside an Agent pane (the collapsible terminal
drawer under the composer, not the standalone "Terminal" widget) shows a
visible jerk in font size / zoom shortly after it opens, and sometimes opens
at a very small size. The user's expectation — and the intended design — is
that each shell remembers and restores its own last-set zoom, and paints at
that size immediately, with no flash or snap.

This is specific to the drawer's shell (`AgentShellSubblock.tsx`). The
standalone Terminal widget (`view: "term"`) does not have this problem.

## 2. Current behavior (root cause)

### 2.1 Zoom is already persisted correctly — the bug is timing, not storage

Font size *is* correctly persisted per-shell, across drawer close/reopen and
app restarts. It is **not** lost on drawer close (`DeleteSubBlockCommand` is
only fired when the whole agent pane closes —
`frontend/app/view/agent/agent-view.tsx:579-581` — not when just the drawer
is collapsed). So "each open uses a default" is not what's happening; the
correct value exists and is fetched, it just doesn't win the race to be
applied before first paint.

Font size computation (`frontend/app/view/agent/components/AgentShellSubblock.tsx`):

```
:79   const BASE_FONT_SIZE = 13;
:99-107
      const termZoom = createMemo(() =>
        clamp(subBlockAtom()?.()?.meta?.["term:zoom"] ?? 1.0, 0.5, 2.0));
      const termFontSize = createMemo(() =>
        round(BASE_FONT_SIZE * termZoom() / agentPaneZoom()));
```

- `term:zoom` lives on the shell **sub-block**'s own `meta` (a distinct
  object from the parent agent pane's block, which has its own separate
  `term:zoom` applied as CSS `zoom` on `.agent-view` —
  `agent-view.tsx:1648-1653,1713`). The shell's font size divides out
  `agentPaneZoom()` so the two zooms don't compound (comment at
  `AgentShellSubblock.tsx:32-39`).
- Write path: Ctrl+Wheel → `RpcApi.SetMetaCommand` immediately
  (`AgentShellSubblock.tsx:128-141`), not debounced to "on close."
- Read path: `subBlockAtom` (`AgentShellSubblock.tsx:94-97`) is
  `WOS.getWaveObjectAtom<Block>(...)`.

### 2.2 The race

`WOS.getWaveObjectAtom` (`frontend/app/store/wos.ts:220-229`, backed by
`createWaveValueObject`, `wos.ts:152-187`) is **asynchronous** — it seeds a
signal to `{ value: null, loading: true }` and resolves later via an HTTP
round-trip (`GetObject`, `wos.ts:158-169`). `AgentShellSubblock` calls
`getWaveObjectAtom` directly rather than `useWaveObjectValue`, so it never
bumps `refCount` — making it less likely to still be warm in
`waveObjectValueCache` (5s hold, `wos.ts:295-305`) between drawer closes.

Sequence on drawer open:

1. Drawer mounts fresh — **every open is a full remount** of
   `AgentShellSubblock`, not an incremental show/hide
   (`<Show when={...}>` in `agent-view.tsx:2100-2138`).
2. `onMount`'s async IIFE (`AgentShellSubblock.tsx:145-249`) does one or two
   RPC round-trips (`ControllerResyncCommand` or `CreateSubBlockCommand`),
   **then** constructs the terminal:
   `new TermWrap(id, containerRef, { fontSize: termFontSize(), ... }, ...)`
   at `:192-216`. `termFontSize()` is read **once, synchronously**, at that
   exact moment.
3. **Concurrently**, the `subBlockAtom` meta fetch is racing the RPCs above.
   If it hasn't resolved yet when `new TermWrap(...)` runs, `termZoom()`
   reads still-`null` meta → falls back to `1.0` → font size defaults to
   `BASE_FONT_SIZE = 13` (further shrunk if the parent `agentPaneZoom() >
   1`, since it's divided out). This is the "opens at a very small zoom"
   symptom.
4. `TermWrap.init()` calls `this.terminal.open(this.connectElem)`
   (`frontend/app/view/term/termwrap.ts:221`) immediately — the terminal
   **paints at that (possibly wrong) size right away**.
5. A correction effect exists (`AgentShellSubblock.tsx:111-117`):
   ```
   createEffect(() => {
       const fs = termFontSize();
       if (termWrap?.terminal && termWrap.loaded) {
           termWrap.terminal.options.fontSize = fs;
           termWrap.handleResize();
       }
   });
   ```
   When the meta fetch resolves later, `termFontSize()` changes reactively,
   this effect re-fires, and — if the guard passes — mutates
   `terminal.options.fontSize` live and calls `handleResize()`, which
   internally (`termwrap.ts:659-662`) does `core._renderService.clear()`
   then `terminal.resize(...)`. **This is the visible jerk**: a small-font
   paint, then an in-place font-size mutation + forced render clear +
   resize, a hard visual snap (confirmed no CSS transition wraps the
   drawer/terminal — `_composer-strip.scss:517-557` — so this is a real
   render-state change, not a CSS artifact).
6. **A secondary, worse latent bug in the same effect**: the guard
   `termWrap?.terminal && termWrap.loaded` reads two **plain,
   non-reactive** values inside a SolidJS `createEffect` — reading them
   does not subscribe to their future changes. If the meta fetch resolves
   in the narrow window *after* `new TermWrap(...)` is constructed but
   *before* `termWrap.loaded` flips true (`termwrap.ts:285`, set only
   after `loadInitialTerminalData()` resolves inside `init()`), the effect
   fires, sees `loaded === false`, and **no-ops permanently** — since
   `termFontSize()` won't change again on its own, the correction is
   silently dropped and the shell can stay stuck at the wrong (small) font
   size for the rest of the session, until the user manually
   Ctrl+Wheel-zooms again. This explains reports of the bug being
   "jumpy/inconsistent" rather than reliably wrong in one direction — which
   race window is hit varies run to run.

### 2.3 Contributing factor (not itself the zoom bug)

`handleResize()` → `customFit()` (`termwrap.ts:635-663`) can also change
`cols`/`rows` (triggering a PTY `SIGWINCH` via `sendTermSize()`).
`TermWrap.init()` separately does its own one-time re-fit via
`requestAnimationFrame` shortly after first paint (`termwrap.ts:349-365`,
documented there as a font-loading-race workaround) — a second, unrelated
resize common to *both* terminal implementations. This adds to the general
"things move right after open" feel but is not the font-size jump described
here.

## 3. Why the standalone Terminal widget doesn't have this bug

The parent **agent pane's own** zoom (a different, independent `term:zoom`,
on the agent block rather than the shell sub-block) had this exact race
class historically and was fixed by seeding the value **before** block
creation:

- `69de7d5e2` "fix(agent): seed term:zoom from ui:zoom when opening agent
  pane" — `frontend/app/store/command-registry.ts:416-450` fetches the
  persisted zoom (`RpcApi.GetAgentContentCommand(..., content_type:
  "ui:zoom")`) **before** `createBlock(...)`, passing it as part of the
  block's **initial** meta. The block's reactive atom is therefore correct
  from its very first read — no async race, no post-hoc correction, no
  jerk.

This was the last of several iterations (`3f0f80c2a`, `dd09b4ed3`/
`727966a68`, `430e261d9` — explicitly "resolves 4 rounds of partial
zoom-persistence PRs", `196521af9`, `7c55a8a80`, `6b3632d4a`) fixing the same
problem for the agent pane's zoom.

The Shell drawer (`AgentShellSubblock`, introduced later — `ad1fa14bd`,
explicitly labeled a "Phase 0 spike" in its own header comment,
`AgentShellSubblock.tsx:4-16`) never received the equivalent treatment: it
creates/resolves the sub-block first, then separately and asynchronously
fetches its meta afterward, then patches the live terminal — the exact
*broken* pattern the agent-pane fixes above replaced.

## 4. Goal & requirements

1. **No visible jerk on open.** The terminal's first paint must already be
   at the correct, persisted font size for that shell sub-block — not a
   default followed by a correction.
2. **No "stuck at wrong size" failure mode.** Eliminate the non-reactive
   `termWrap.loaded` guard race described in §2.2.6 — whatever replaces it
   must not be able to silently drop a legitimate late-arriving correction.
3. **Live Ctrl+Wheel zoom while the shell is already open must keep
   working** exactly as today — this spec only changes the *initial mount*
   path.
4. **No regression to per-shell scoping.** Zoom stays keyed to the shell
   sub-block's own identity (not the parent agent pane, not global) — this
   is already correct today (§2.1) and must not change.

## 5. Proposed fix direction

Apply the same "seed before construct" pattern already proven for the
parent agent pane's zoom (`command-registry.ts:416-450`):

- Resolve the shell sub-block's `term:zoom` **before** constructing
  `TermWrap`/`Terminal` in `AgentShellSubblock.tsx:192-216` — either:
  - (a) have the `ControllerResyncCommand` / `CreateSubBlockCommand` RPC
    response (already awaited before this point, §2.2.2) include or be
    followed by an awaited fetch of the sub-block's meta, so
    `termFontSize()` is known before line 192 runs; or
  - (b) explicitly `await` the underlying `GetObject` fetch behind
    `WOS.getWaveObjectAtom` for this sub-block id before constructing
    `TermWrap`, rather than relying on the reactive atom to arrive
    eventually.
- Do not render/mount the terminal container until that value is resolved,
  to avoid a FOUC-style flash at the wrong size (the drawer itself has no
  open/close animation to hide behind, per `_composer-strip.scss:517-557`,
  so there's nothing else masking an interim state). **Use the existing
  `BrainSpinner` stand-in** (`frontend/app/element/BrainSpinner.tsx`) as the
  loading placeholder for this interim state, rather than a blank drawer or
  a new bespoke spinner — this is the codebase's established pattern for
  exactly this class of problem (avoid painting wrong/empty content while
  a value that will change the paint is still in flight), already used by:
  - The **agent pane's own** blank-load case
    (`frontend/app/view/agent/agent-view.tsx:715-716,1721-1725` — see
    `docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md`,
    which is the same problem category this spec addresses, one level up).
  - The **browser pane** (`frontend/app/view/browser/browser-view.tsx:81-118,189-193`),
    which is the closer structural match: two local signals
    (`spinnerMounted`/`spinnerFading`) driven by the real loading source of
    truth (there, `model.loadingAtom()`; here, "has the sub-block meta
    fetch resolved"), with a `setTimeout(LOADING_SPINNER_FADE_MS = 200)`
    keeping the node mounted long enough for the CSS opacity fade to finish
    before unmount (`BrainSpinner`'s contract: caller owns unmount timing).
    Reduced-motion users skip the timeout and unmount instantly
    (`BrainSpinner` reads `atoms.prefersReducedMotionAtom` itself for the
    pulse/fade animation, but the *hold-time-before-unmount* is the
    caller's responsibility either way).
  - Implementation shape: a positioned overlay div (own
    `.agent-shell-loading-overlay` CSS, `position:absolute; inset:0`,
    matching `_loading-overlay.scss`'s / `browser-view.scss`'s existing
    overlay rules — including `data-pane-overlay` if this drawer needs the
    same native-HWND clip-hole handling the browser pane's overlay tags for,
    tbd during implementation) wrapping `<BrainSpinner fading={...} />`,
    shown via `<Show when={showLoadingOverlay()}>` while the seed fetch is
    in flight, `fading` flipped true once the value resolves, unmounted
    ~200ms later.
- Keep the existing `createEffect` (`AgentShellSubblock.tsx:111-117`) for
  **live** updates (Ctrl+Wheel while open) — the fix scopes to the initial
  mount value only.
- If the effect is kept for any post-mount correction path, fix the
  non-reactive `.loaded` guard (§2.2.6) so a late-arriving update can never
  be silently dropped — e.g. re-check once `loaded` reactively transitions,
  rather than reading it as a snapshot inside an effect keyed only on
  `termFontSize()`.

## 6. Non-goals / out of scope

- Changing how zoom is scoped (per-shell vs per-pane vs global) — already
  correct.
- The unrelated `requestAnimationFrame` re-fit in `TermWrap.init()`
  (`termwrap.ts:349-365`) — a separate, pre-existing font-loading
  workaround shared by both terminal implementations, not part of this bug.
- The standalone Terminal widget (`view: "term"`) — unaffected, already
  correct.

## 7. Open questions

- Which of the two seed options in §5 (await inside the existing RPC chain
  vs. explicit `GetObject` await) is cheaper — does
  `ControllerResyncCommand`/`CreateSubBlockCommand` already return enough of
  the block to read `meta["term:zoom"]` directly from its response, avoiding
  a second round-trip entirely? Needs a look at the RPC response shape
  server-side (`agentmux-srv`) before committing to an approach.
- ~~Should the terminal container be hidden (not just deferred) until the
  seed value resolves~~ — resolved: yes, use `BrainSpinner` as the stand-in
  (§5), matching the existing agent-pane/browser-pane pattern rather than
  leaving a blank drawer or a new bespoke loading state.
- Does this drawer's overlay need the `data-pane-overlay` tag the browser
  pane's uses (for `pane-overlay-auto.ts`'s native-HWND clip-hole handling),
  or is that specific to panes with a native (non-DOM) content layer under
  them? The shell drawer's content is a plain xterm.js canvas, not a native
  view, so this may be unnecessary here — confirm during implementation
  rather than assume either way.
