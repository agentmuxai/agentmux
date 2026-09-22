# SPEC — Settings commit on blur, not on change; plus the remaining terminal live-apply gaps

**Date:** 2026-09-22
**Status:** implemented in #3509 — see §8 for what shipped and how it was verified
**Author:** AgentO
**Related:**
`docs/reports/REPORT_TERMINAL_SCROLL_SENSITIVITY_NOT_LIVE_2026_09_22.md`
(PR #3509 — fixed the terminal-side consumer wiring for one setting; this
spec is the two things found while verifying that fix live: a much bigger
root cause upstream of it, and four more terminal settings with the exact
consumer-side gap #3509 already fixed for `scrollSensitivity`),
`frontend/app/view/settings/settings-controls.tsx` (`SliderControl`,
whose existing debounce shape this spec reuses rather than invents)

---

## 1. Why this spec exists

Asked to verify #3509 live, then reported: *"scroll sensitivity still not
working in your task dev."* Investigating found the setting itself wasn't
the problem — clicking a terminal pane's tab appeared to be what made it
apply, which read as a reactivity bug in the `createEffect` #3509 added.
Then, once the actual trigger was named precisely (*"currently I need to
actually select the terminal pane for the update to take place... we want
it as reactive as possible with the least amount of addl user effort"*),
a different, much simpler explanation became testable: **the setting
input itself was never committing anything until it lost focus.**
Clicking the terminal pane wasn't "selecting" it in any meaningful
sense — it was just the nearest thing to click, and clicking *anything*
fires the same blur.

Confirmed live, not assumed (§2). The user's closing instruction —
*"that should be the modus operandi for all setting updates"* — sets this
spec's actual scope: not one input field, but the shape every plain
number setting in the app uses.

## 2. Root cause, confirmed live

Every plain `<input type="number">` setting row in this codebase commits
via `onBlur`, not `onInput`/`onChange`:

```tsx
// terminal-section.tsx:237-242 — scroll sensitivity, before this spec
<input
    class="setting-number setting-number--wide"
    type="number" min={0.1} max={10} step={0.1}
    value={(s()["term:scrollsensitivity"] as number) ?? 1}
    onBlur={(e) => {
        const v = parseFloat(e.currentTarget.value);
        if (!isNaN(v) && v >= 0.1 && v <= 10) set("term:scrollsensitivity", v);
    }}
/>
```

Reproduced against the running `task dev` instance over CDP (its own
Input domain — driving keys/clicks through the real browser input
pipeline, not synthetic DOM events, so this is what an actual user's
keystrokes do):

| Step | `terminal.options.scrollSensitivity` |
|---|---|
| Before typing (existing value) | `2.6` |
| Typed `7`, input now reads `7`, **still focused** | `2.6` — unchanged |
| Waited 500ms, **still focused**, no further input | `2.6` — unchanged |
| Tabbed away (blur) | `7` — committed instantly |

This is decisive: the `createEffect` wiring #3509 added is correct and
was applying the value the instant it actually changed — there was
nothing to catch, because nothing had been written yet. The user's
"select the terminal pane to make it apply" observation was accurate
description of the symptom, not a misdiagnosis — clicking the terminal
tab is exactly the kind of click that blurs a currently-focused input.
Any other click would have done the same.

### 2.1 Scope — every affected site

`grep -rln 'onBlur=' frontend/app/view/settings/` turns up six files.
Of those, the ones using a **plain number input specifically** (the
class this spec fixes) are:

| File | Setting | min | max | parse | value transform | class |
|---|---|---|---|---|---|---|
| `terminal-section.tsx` | `term:fontsize` | 8 | 32 | int | — | `setting-number` |
| `terminal-section.tsx` | `term:scrollback` | 1000 | 100000 | int | — | `setting-number setting-number--wide` |
| `terminal-section.tsx` | `term:scrollsensitivity` | 0.1 | 10 | float | — | `setting-number setting-number--wide` |
| `terminal-section.tsx` | `term:predictiveecho:thresholdms` | 0 | — | float | — | `setting-number setting-number--wide` |
| `terminal-section.tsx` | `term:agentmaxruntimehours` | 0 | — | float | — | `setting-number setting-number--wide` |
| `terminal-section.tsx` | `term:agentidletimeoutmins` | 0 | — | float | — | `setting-number setting-number--wide` |
| `appearance-section.tsx` | `window:tilegapsize` | 0 | 20 | int | — | `setting-number` |
| `appearance-section.tsx` | `window:magnifiedblocksize` | 1 | — | float | — | `setting-number setting-number--wide` |
| `appearance-section.tsx` | `window:magnifiedblockblurprimarypx` | 0 | — | int | — | `setting-number` |
| `appearance-section.tsx` | `window:magnifiedblockblursecondarypx` | 0 | — | int | — | `setting-number` |
| `advanced-section.tsx` | `agent:askquestiontimeoutms` | 1 | — | float | display = ms/1000, commit = `Math.round(v*1000)` | `setting-number setting-number--wide` |
| `advanced-section.tsx` | `telemetry:interval` | 1 | — | float | — | `setting-number setting-number--wide` |
| `advanced-section.tsx` | `telemetry:numpoints` | 30 | 1024 | int | — | `setting-number setting-number--wide` |

13 sites. `advanced-section.tsx`'s `dnd:concurrency` (min 1, no max, int,
`parseInt`) is deliberately **excluded** — see §5.

Every guard here reduces to exactly `!isNaN(v) && v >= min && (max ==
null || v <= max)`, and `parseInt` vs `parseFloat` is the only other
axis of variation. No site does bounds-checking beyond that, and no
site (other than the one flagged transform) does anything to the value
between parsing and calling `set()`.

## 3. Why this wasn't caught by the fontSize/transparency precedent

`SliderControl` (`settings-controls.tsx:54-74`) — used for
`term:transparency` and others — already solves exactly this problem,
correctly, and has for a while:

```tsx
export function SliderControl(p: { min: number; max: number; step: number; value: number; onChange: (v: number) => void }): JSX.Element {
    const [local, setLocal] = createSignal(p.value);
    createEffect(() => setLocal(p.value));
    let timer: ReturnType<typeof setTimeout> | null = null;
    onCleanup(() => { if (timer != null) clearTimeout(timer); });
    return (
        <div class="setting-slider">
            <input
                type="range"
                min={p.min} max={p.max} step={p.step}
                value={local()}
                onInput={(e) => {
                    const v = parseFloat(e.currentTarget.value);
                    setLocal(v);
                    if (timer != null) clearTimeout(timer);
                    timer = setTimeout(() => { timer = null; p.onChange(v); }, 180);
                }}
            />
            <span class="setting-slider-val">{Math.round(local() * 100) / 100}</span>
        </div>
    );
}
```

`onInput` (fires continuously while dragging) plus a 180ms debounced
commit, with a `local` signal for immediate visual feedback independent
of the commit timer. This is the right shape, already proven in this
exact file — it just was never extracted into something a plain number
field could reuse, so every number field grew its own `onBlur`-only
input from scratch instead. This spec's fix is almost entirely "give the
number fields the same treatment the slider already has," not a new
design.

## 4. Design

### 4.1 `NumberControl` — new, `settings-controls.tsx`, beside `SliderControl`

```tsx
export function NumberControl(p: {
    min: number;
    max?: number;
    step: number;
    /** parseInt vs parseFloat — must match what the setting itself stores.
     *  Silently switching an integer setting to float parsing would let a
     *  fractional value through where the backend/consumer expects a whole
     *  number (e.g. `term:scrollback` in xterm.js display rows). */
    parse?: "int" | "float"; // default "float"
    value: number;
    onChange: (v: number) => void;
    class?: string;
}): JSX.Element {
    const [local, setLocal] = createSignal(p.value);
    createEffect(() => setLocal(p.value));
    let timer: ReturnType<typeof setTimeout> | null = null;
    onCleanup(() => { if (timer != null) clearTimeout(timer); });
    const parseVal = (raw: string) => (p.parse === "int" ? parseInt(raw, 10) : parseFloat(raw));
    const inRange = (v: number) => !isNaN(v) && v >= p.min && (p.max == null || v <= p.max);
    const commit = (v: number) => {
        if (timer != null) { clearTimeout(timer); timer = null; }
        if (inRange(v)) p.onChange(v);
    };
    return (
        <input
            class={p.class ?? "setting-number"}
            type="number" min={p.min} max={p.max} step={p.step}
            value={local()}
            onInput={(e) => {
                const v = parseVal(e.currentTarget.value);
                if (!isNaN(v)) setLocal(v); // mirror what's actually typed, valid or not
                if (timer != null) clearTimeout(timer);
                // Debounce — an in-range value commits shortly after the
                // user stops typing, an out-of-range/invalid one is
                // silently dropped (same silent-ignore every existing
                // onBlur guard already does; this spec doesn't add
                // validation UX that wasn't there). NOTE — this draft's
                // 180ms (matching SliderControl verbatim) turned out to be
                // too short for typing specifically; shipped as 400ms
                // instead — see §8.1, found live, not caught by review.
                timer = setTimeout(() => { timer = null; if (inRange(v)) p.onChange(v); }, 180);
            }}
            onBlur={(e) => {
                // Flush immediately on blur rather than waiting out the
                // debounce — a user who tabs away mid-window shouldn't
                // have to wait for something already valid to land, and
                // this is the one case SliderControl doesn't need an
                // equivalent for (a range input has no "half-typed" state
                // a user can tab away from).
                commit(parseVal(e.currentTarget.value));
            }}
        />
    );
}
```

### 4.2 Migration

Every row in §2.1's table becomes `<NumberControl min=… max=… step=…
parse=… value=… onChange={(v) => set("key", v)} class=… />`, `parse`
included only when `"int"` (the default covers every float site without
stating it explicitly, matching how most existing sites never state
`parseFloat` either — it's already the assumed default via `parseFloat`
being the fallback everywhere else in this codebase's own number
handling).

`agent:askquestiontimeoutms` keeps its value transform entirely in the
call site's closure — `NumberControl` needs no special-casing for it:

```tsx
<NumberControl
    min={1}
    value={((s()["agent:askquestiontimeoutms"] as number) ?? 30000) / 1000}
    onChange={(v) => set("agent:askquestiontimeoutms", Math.round(v * 1000))}
/>
```

### 4.3 What does NOT change

- Every toggle (`ToggleControl`), slider (`SliderControl`), and the
  `term:theme` `<select>` — all already commit on `onChange`, already
  correct, untouched.
- Text `<input>` fields still on `onBlur` (`window:bgcolor`,
  `term:fontfamily`, `app:defaultnewblock`, the recording-section's
  generic string field, `KeyValueEditor`'s entries) — deliberately out of
  scope, see §5.
- Validation bounds and int-vs-float parsing per setting — carried over
  exactly, not revisited. This spec changes *when* a value commits, not
  *what* is accepted.

## 5. Out of scope

- **`dnd:concurrency`.** Its blank-field-means-`null`-means-"unlimited"
  shape doesn't fit `NumberControl`'s number-in/number-out contract
  without a real extra `allowEmpty`/`emptyValue` mechanism bolted onto a
  component every OTHER site would carry unused weight for, to serve one
  setting. Left as a raw `onBlur` input, unchanged. A live-follow-up can
  add empty-value support to `NumberControl` (or a wrapping component) if
  wanted — not blocking on this pass.
- **Text inputs.** `window:bgcolor` (a color hex string — a half-typed
  `#3f` visibly flashing an invalid/wrong swatch while mid-type is a
  worse experience than waiting for blur, unlike a number field where an
  incomplete value just silently fails the range guard and displays
  nothing changed) and `term:fontfamily`/`app:defaultnewblock` (free
  text) are a different shape of problem with a different right answer
  per field, not the same fix mechanically reused. Flagged for a
  follow-up decision, not silently declared "also fixed."
- **Any change to what a setting validates or accepts** — see §4.3.

## 6. Part B — the terminal-side gap this was originally about

Separately from §2-§5 (a settings-UI-wide fix), the original
investigation (that produced this spec) found #3509 left several more
terminal settings resolved at `TermWrap` construction and never
revisited — exactly the class of bug that spec's own investigation found
and #3509 fixed for `term:scrollsensitivity` alone.

**Correction against the first pass of this audit:** `term:theme` (and
`term:transparency`'s effect on the xterm-internal palette) turned out to
already be live — `termtheme.ts`'s `TermThemeUpdater`, a small standalone
component rendered from `term.tsx`'s JSX (`term.tsx:426`,
`<TermThemeUpdater blockId={blockId} model={model}
termRef={model.termRef} />`), already does exactly what this section was
about to build: a `createMemo` over the same `computeTheme` inputs, a
`createEffect` applying `.terminal.options.theme`. The first draft of
this table listed it as missing because the earlier audit only grepped
`createEffect(...)` calls *inside* `term.tsx`'s own function body and
missed a *separate* component mounted from its JSX — caught before
implementing, by grepping `terminal.options\.` across every file in the
directory instead of one file, which is the check that should have run
the first time. `blockBg`'s own visible-background reactivity (§ the
prior draft's own correct point) is unaffected either way — that part of
the original reasoning stands, just attached to the wrong "is this
live?" conclusion for the palette.

Confirmed against the xterm.js typings the same way #3509 did for the
remaining three: all inside mutable `ITerminalOptions`, not
`ITerminalInitOnlyOptions`.

| Setting | xterm.js option | Live effect exists today? |
|---|---|---|
| `term:theme` (+ `term:transparency`'s effect on it) | `theme` | ✅ already, via `TermThemeUpdater` |
| `term:fontfamily` | `fontFamily` | ❌ |
| `term:scrollback` | `scrollback` | ❌ |
| `term:allowbracketedpaste` | `ignoreBracketedPasteMode` | ❌ |

### 6.1 `term.tsx` (Terminal pane)

- **New pure resolver**, `frontend/app/view/term/termfontfamily.ts` —
  `resolveTermFontFamily(termSettings, connFontFamily?)`, preserving
  `term.tsx`'s exact existing precedence (`ts["term:fontfamily"] ??
  connFontFamily ?? "Hack"` — settings before connection, no per-block
  meta override; note this differs from `term:fontsize`'s own precedence
  order, which is meta-then-connection-then-settings — an existing
  inconsistency, preserved as-is, not silently "fixed" as part of this
  pass).
- `term:scrollback` already has a shared resolver
  (`resolveTermScrollback`, `termscrollback.ts`) — just needs the live
  wiring, no new resolver.
- `term:allowbracketedpaste` already has a reactive accessor
  (`getOverrideConfigAtom(blockId, "term:allowbracketedpaste")`) — same,
  just needs to be read inside an effect instead of once in `onMount`.
- Three new top-level `createMemo`s in `term.tsx`
  (`termFontFamily`, `termScrollbackDepth`, `termAllowBracketedPaste`),
  replacing the equivalent one-shot local `const`s currently computed
  inline inside `onMount` — `onMount` reads these memos instead of
  recomputing them, so construction and the live effects share one
  source of truth.
- Two effects, not one per setting — grouped by whether the change
  affects character-cell geometry (needs `handleResize()`, same as font
  size already does) or not:
  - Extend the existing font-size effect to also apply `fontFamily`
    (font family changes cell width exactly like font size does).
  - Extend the existing scroll-sensitivity effect to also apply
    `scrollback` and `ignoreBracketedPasteMode` — neither affects
    geometry, so no `handleResize()` call joins them.

### 6.2 `AgentShellSubblock.tsx` (agent Shell drawer)

Only `term:scrollback` is in scope here — it's the only one of the three
remaining settings the drawer already resolves at construction (via the
existing shared
`resolveTermScrollback`, added when the drawer's scrollback support was
originally built). One more `createEffect`, mirroring its existing
font-size one exactly (same `wrapLoaded()`-read-unconditionally
requirement — reagentx P2 on #2522 — since this file's `TermWrap`
construction is asynchronous, unlike `term.tsx`'s synchronous one).

**`term:theme`, `term:fontfamily`, and `term:allowbracketedpaste` are
NOT added to the drawer in this pass.** It doesn't resolve any of the
three at construction today — `fontFamily` is hardcoded to `"Hack"`,
`theme` and `ignoreBracketedPasteMode` aren't passed at all
(`allowTransparency: false` there too, a deliberate opaque-by-design
difference from `term.tsx`). Wiring these into the drawer for the first
time is a real feature addition, not a live-update fix for something it
already had — flagged here, deliberately not done silently alongside a
"make settings live" pass, matching this spec's own §4.3 principle.

## 7. Testing

- `settings-controls.test.ts` (new or extended) — `NumberControl`:
  commits after the debounce window with no further input, does not
  commit while still typing within the window, flushes immediately on
  blur, ignores out-of-range/non-numeric input silently (matches every
  site's current guard), `parse: "int"` truncates a fractional value the
  same way every current `parseInt` site already does.
- `termfontfamily.test.ts` (new) — mirrors `termscrollback.test.ts`'s
  shape: default fallback, setting present, connection fallback,
  malformed/absent values.
- Full frontend suite + `tsc --noEmit -p tsconfig.citypecheck.json`.
- Live re-verification of §2's exact repro (CDP, `task dev`): type a new
  scroll-sensitivity value, confirm `terminal.options.scrollSensitivity`
  updates within the debounce window **without** blurring or clicking
  anything else — the literal requirement this spec exists to satisfy.
- Live check of each of §6's three remaining settings the same way, on
  both `term.tsx` and (scrollback only) `AgentShellSubblock.tsx`.

## 8. What shipped, and a debounce correction found while verifying live

Implemented as §4/§6 specified, with one deliberate deviation from §4.1's
first draft: the debounce is **400ms, not 180ms** (§8.1 explains why —
found live, not assumed). Otherwise as designed: `NumberControl` added to
`settings-controls.tsx`, all 13 eligible sites migrated (§2.1's table,
`dnd:concurrency` excluded per §5), `termfontfamily.ts` added,
`term.tsx`/`AgentShellSubblock.tsx` wired per §6.

- 12 unit tests for `NumberControl` (`settings-controls.test.tsx`), 6 for
  `resolveTermFontFamily` (`termfontfamily.test.ts`).
- Full frontend suite (2029 tests across `frontend/app/view/term`,
  `frontend/app/view/agent`, `frontend/app/view/settings`) and
  `tsc --noEmit -p tsconfig.citypecheck.json` clean.
- Every one of the four terminal settings (`scrollSensitivity`,
  `fontFamily`, `scrollback`, `allowbracketedpaste`) verified live against
  the running `task dev` instance over CDP, end to end: real keystrokes
  through the actual Settings UI, checked against
  `terminal.options.*` — not just that the setting committed, but that a
  **live, already-open terminal pane** picked it up with zero additional
  clicks (§8.2 covers a wrinkle in getting an unambiguous reading for
  `scrollback` specifically).

### 8.1 180ms was too short for typing — found live

`SliderControl`'s 180ms debounce is right for what it's tuned for: a drag
gesture fires many events per second, and every intermediate position is
a legitimate value. Typing a number is a different interaction —
intermediate typed states are usually *not* meant to be final values,
just as-you-type keystrokes that happen to parse as valid numbers.

Reproduced live: typing "77000" into `term:scrollback` (min 1000) with
180ms committed early, mid-entry — "7700" alone is already in-range, and
a natural gap between keystrokes (or CDP's own per-keystroke round-trip
latency, which turned out to be closer to real human typing rhythm than
expected) was enough to fire the debounce on the incomplete value before
the final digit landed. Not a hypothetical: caught directly via a CDP
repro before this shipped, not discovered after the fact.

Widened to 400ms — inside the normal range for "commit after the user
stops typing" UI conventions, comfortably covers ordinary inter-keystroke
gaps, while still committing well under half a second after the user
stops. `settings-controls.tsx`'s own comment on `NumberControl` states
this isn't `SliderControl`'s constant copied verbatim, and why.

### 8.2 A verification wrinkle: `MAX_TERM_SCROLLBACK` isn't the UI's own max

While live-testing `scrollback`, several distinct test values (61000,
77000, 88000, 99000) all appeared to produce **no change at all** in
`terminal.options.scrollback` — worth being honest that this looked, for
a while, exactly like a broken live-effect, and was investigated as one:
instrumented `NumberControl`'s own commit path directly (confirming the
debounced RPC fired correctly, at the correct time), confirmed the write
persisted across a full page reload (ruling out the RPC/write path), and
only then found the actual cause: `termscrollback.ts`'s
`MAX_TERM_SCROLLBACK = 50000` — a **pre-existing, deliberate** clamp
(predating this spec, documented in that file's own comment) that is
lower than the Settings UI's own `max={100000}`. Every one of those test
values exceeded 50000 and was correctly clamped to it — which also
happened to be the exact value left over from an earlier test run, making
"clamped" and "unchanged" visually indistinguishable until a value under
50000 (23000) was tried and moved cleanly.

Nothing about this discrepancy was introduced by this spec — `term.tsx`'s
`onMount` construction path already went through the same
`resolveTermScrollback` (and therefore the same clamp) before this spec
touched it; the live-apply effect added here just made the existing,
already-shipped clamp newly *visible* at a moment (immediately after
typing, instead of only after a reopen) where it was mistakable for a
regression. Flagged here rather than silently working around it — whether
the settings UI's own `max` should be lowered to 50000 to match, or the
clamp raised to 100000 to match the UI, is a real product decision this
spec did not make and is out of scope for it; either resolves the
mismatch, and both remain equally correct after this spec's changes.
