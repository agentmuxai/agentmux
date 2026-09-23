# Typing should go into a pane the moment it opens (editor + terminal)

**Status:** active — terminal pane creation implemented in #3539 (§9), using
a narrower fix than the shared-contract Option B this spec recommends.
Editor, agent, and tab-switching (§4's "probably in") remain unimplemented.
**Date:** 2026-09-16
**Severity:** Medium — no data is lost, but every new editor or terminal pane
costs the user a mouse trip before it can be typed into, on the app's two most
keyboard-driven surfaces.
**Requested by:** repo owner.
**Related:** `SPEC_EDITOR_FIRST_KEYSTROKE_NOT_RENDERED_2026_09_15.md` (a
*different* bug with a similar-looking symptom — see §6, they must not be
conflated).

---

## 1. What we want

Open an editor or terminal pane and **just type**. The pane is already selected
on creation, so the caret should be there too — no click required.

Today: open the pane, reach for the mouse, click into the text area, then type.

## 2. Why it happens — the shared focus contract is one-shot

This is not two view bugs. It is one shared contract with two different
failures on top of it.

Every focus hand-off in the app goes through the same three-line pattern,
duplicated in three places (`focusManager.refocusNode()`,
`block-component-registry.ts`'s focus helper, and `openOrFocusPaneByView()`):

```ts
const ok = bcm?.viewModel?.giveFocus?.();
if (!ok) {
    const inputElem = document.getElementById(`${blockId}-dummy-focus`);
    inputElem?.focus();
}
```

Two properties of that contract cause the symptom:

- **It is a single synchronous attempt.** There is no retry, and no notion of
  "the view isn't mounted yet — try again when it is."
- **Its failure mode is silent and plausible-looking.** When `giveFocus()`
  returns `false`, focus lands on a hidden `*-dummy-focus` input. The pane
  *looks* focused (it is, at the layout level) and the app is in a perfectly
  consistent state — but keystrokes go to a dummy element instead of the
  editor or terminal.

### 2a. Editor — `giveFocus()` is a stub

`frontend/app/view/editor/editor-model.ts:1210`:

```ts
giveFocus(): boolean {
    return false;
}
```

It never focuses CodeMirror. Every focus attempt therefore falls through to the
dummy input, on every pane open, deterministically. This is also consistent
with `editor-view.tsx` never calling `cmView.focus()` anywhere.

### 2b. Terminal — implemented, but racing

`frontend/app/view/term/termViewModel.ts:504`:

```ts
giveFocus(): boolean {
    ...
    if (this.termRef?.current?.terminal) {
        this.termRef.current.terminal.focus();
        return true;
    }
    return false;
}
```

Correct when the terminal exists — but at pane-creation time `termRef.current`
may not be populated yet, since the xterm instance is constructed during view
mount. The call returns `false`, focus goes to the dummy input, and nothing ever
tries again. Same visible symptom, different cause: **a race, not a stub.**

### 2c. It is worse than editor+terminal — the agent pane is a stub too

An earlier revision of this spec claimed `agent-model.ts:930` and
`browser-model.ts:711` both implement `giveFocus()`, and used that to argue the
contract itself was sound. **That was wrong** (reagent P1), and the correction
matters because it changes the diagnosis rather than just a detail:

| View | `giveFocus()` | Status |
|---|---|---|
| Editor (`editor-model.ts:1210`) | `return false` | **stub** |
| Agent (`agent-model.ts:930`) | `return false` | **stub — identical** |
| Terminal (`termViewModel.ts:504`) | focuses xterm when it exists | implemented, **races mount** |
| Browser (`browser-model.ts:711`) | real implementation, with guards | works |

So **three of four pane types do not reliably take the caret**, and two are
outright stubs. Only the browser pane genuinely implements the contract.

That makes the shared-layer fix (§3 Option B) the clear choice rather than a
judgement call: patching per-view would mean writing the same readiness logic
three times, and the next view added would default to the same silent failure.
It also means the acceptance criteria in §7 should cover the agent pane, even
though the original request named only editor and terminal.

### 2d. The creation path never reaches the contract at all

**This is the part that makes §3 harder than it looks** (codex P1). The three
call sites in §2 all run when focusing an *existing* pane. On **creation** —
the case this spec is actually about — none of them runs:

- `openOrFocusPaneByView()` only calls `giveFocus()` on its
  already-exists branch; for a new pane it falls through to
  `await createBlock(...)`.
- `createBlock()` inserts a layout node with `focused: true` and stops there.
- The reducer path that follows does not call `giveFocus()`, because
  `focusManager.requestNodeFocus()` is literally:

```ts
requestNodeFocus(): void {
    // no-op
}
```

So today a newly created pane never has `giveFocus()` invoked at all — the
editor's stub is not even reached. Marking a layout node `focused: true` sets
*selection*, which is why the pane looks and behaves as selected while the caret
is nowhere.

**Consequence for the design:** making the shared contract retry (Option B) is
necessary but **not sufficient**. Retrying a call that is never made changes
nothing, and both primary acceptance cases in §7 would still fail. Creation must
be explicitly connected to the contract — either by making
`requestNodeFocus()` real, or by having the block component signal readiness to
the focus manager when it mounts. Any option chosen in §3 has to say which.

## 3. Design options

**Option A — Fix each view in place.** Implement the editor's `giveFocus()`;
add a readiness check or short retry to the terminal's.

*Pro:* smallest diff, no shared-layer risk.
*Con:* leaves the one-shot contract intact, so the next async-mounting view
re-introduces this. Also leaves the "silently focus a dummy element" failure
mode in place.

**Option B — Make the contract retry until the view is ready (recommended).**
Give view models a way to say "not yet" distinctly from "not supported", and
have the shared helper retry on mount rather than falling straight through to
the dummy input. Concretely, either:

- `giveFocus()` may return `"pending"` in addition to `true`/`false`, and the
  caller re-attempts when the block's component registers; or
- view models expose a `focusWhenReady(): Promise<boolean>`, and the three call
  sites await it.

*Pro:* fixes editor and terminal with one mechanism, and any future view for
free. Makes the silent-dummy-focus fallback a genuine last resort.
*Con:* touches shared focus code used by every pane type — needs care around
not stealing focus (§5). **And on its own it fixes nothing on the creation
path** (§2d): it must be paired with actually invoking the contract when a pane
is created, since `requestNodeFocus()` is currently a no-op.

**Option C — Focus on mount from inside each view**, independent of the focus
manager.

*Pro:* no shared-layer change.
*Con:* two sources of truth for focus; races the focus manager; likely to steal
focus in exactly the cases §5 says it must not.

**Recommendation: B, plus the creation wiring from §2d, with A's editor half
done first.** Implementing the editor's `giveFocus()` is required under every
option and is independently correct, so it can land immediately while the rest
is agreed. But note that neither A nor B alone satisfies §7 — without §2d's
wiring, nothing calls `giveFocus()` on a new pane at all.

## 4. Scope

- **In:** editor panes, terminal panes, on pane *creation*.
- **Probably in:** focusing an existing pane (tab switch, `openOrFocusPaneByView`)
  — same call sites, same fix; needs an explicit decision, see Q2.
- **Also affected, per §2c:** the agent pane's `giveFocus()` is the same stub as
  the editor's. Not in the original request, but it falls out of the shared fix
  for free and should not be left as the one stub still standing.
- **Out:** the browser pane (genuinely implements the contract); changing which
  pane is *selected* on creation — selection already works, this is only about
  where the caret goes.

## 5. Constraints — when NOT to take focus

Auto-focus is a behavior change and can be actively hostile if over-applied:

1. **Never steal focus from a pane the user is already typing in.** A pane that
   opens in the background (e.g. an agent opening a file via `muxsh`) must not
   yank the caret mid-sentence.
2. **Never steal focus from a modal or an open search panel.** The terminal's
   `giveFocus()` already guards on `searchAtoms.isOpen()`; any shared mechanism
   must preserve that.
3. **Restoring a pane on startup is not the same as creating one.** Reopening a
   saved layout with five panes must not have five views fight over the caret.
4. **Read-only or error states** (editor with a load error, dead terminal)
   should probably not take the caret — Q3.

## 6. Not to be confused with the first-keystroke bug

`SPEC_EDITOR_FIRST_KEYSTROKE_NOT_RENDERED_2026_09_15.md` describes a symptom
that also looks like "typing doesn't work until you click": there, the click
*did* focus CodeMirror, the keystroke *was* received, and a reactive rebuild
then destroyed it.

They are independent:

- That one is **fixed and merged** (#3260). An earlier revision of this spec
  asserted that while the PR was still open, which was premature (codex P2) —
  the statement is only made here now that the fix is actually on `main`.
- That one was **data loss**; this one loses nothing — the keystrokes go to a
  dummy input and are discarded before any document exists.
- Fixing this one *first* would have **masked** that one: auto-focus would have
  removed the click, while the first keystroke was still silently dropped and
  the user would simply have lost a character with no visible cause. That is an
  argument for landing them in the order they were found — not a claim that the
  other one is already closed (see above).

## 7. Acceptance

- Open a new terminal pane, type immediately: characters reach the shell.
- Open a new editor pane on a file, type immediately: characters reach the
  document and render.
- Open a pane in the background while typing in another: the caret does not
  move.
- With a modal or the editor's find panel open, opening a pane does not steal
  the caret.
- No pane ever silently leaves the caret on a `*-dummy-focus` element when its
  view is mountable.

## 8. Open questions

- **Q1.** Should `giveFocus()` gain a tri-state return, or should view models
  expose `focusWhenReady()`? The first is a smaller diff; the second is harder
  to get subtly wrong at the call sites.
- **Q2.** Does this apply to *focusing an existing* pane (tab switch), or only
  to creation? Users may expect a tab switch back to an editor to restore the
  caret, which is the same mechanism.
- **Q3.** Should a pane in an error/read-only state take the caret at all?
- **Q4.** Is the `*-dummy-focus` fallback still needed once views focus
  reliably, or is it masking bugs it should be surfacing? It is currently
  indistinguishable from success from the caller's perspective.
- **Q5.** How long should a retry persist before giving up, and should it be
  bounded by attempts or by the block's mount signal? A timer would be a poor
  fit — the registry already knows when a block component mounts.

## 9. What shipped for the terminal — a narrower fix than §3's Option B

Landed only the terminal half, and via a smaller change than the
recommended shared-contract retry (§3 Option B). Recorded here rather than
silently deviating from the spec's own recommendation.

### 9.1 Why terminal-only

Reported directly: opening a terminal pane via the "+" picker and typing
immediately sent every keystroke nowhere — confirmed live, not assumed
(`document.activeElement` stayed on `<body>` after the pane opened;
calling `terminal.focus()` by hand fixed it instantly). This is exactly
§2b/§2d's diagnosis: the terminal's own `giveFocus()` already works once
called, and §2d's "the creation path never reaches the contract at all"
is the actual cause. Editor and agent panes have the same underlying gap
(§2c) but weren't the reported symptom, and fixing them means actually
implementing their stubbed `giveFocus()` bodies — separate, real work
this pass didn't do.

### 9.2 The fix

`term.tsx`'s own `onMount` already had a mechanism for exactly this
shape — `if (wasFocused) setTimeout(() => model.giveFocus(), 10)` — but
gated behind `model.termRef.current != null`, which was written for one
purpose only (preserve focus across a *remount* of an already-existing
pane) and accidentally excluded the *first* construction, which is
precisely §2d's creation path. `model.nodeModel.isFocused()` alone —
without the null check — is true in both cases: on a remount that
preserves existing focus (the original case), and on first creation,
since `createBlock()` already inserts the new layout node with
`focused: true` before `onMount` ever runs. Dropping the guard was the
entire change; the retry timing, the `giveFocus()` call, and its
`searchAtoms.isOpen()` guard were already correct and untouched.

This is deliberately **not** §3's Option B (a tri-state
`giveFocus()`/`focusWhenReady()` contract retried from the three shared
call sites). That's the right long-term shape for covering editor, agent,
and tab-switching in one mechanism — but it touches focus code shared by
every pane type, and §3 itself flags real unresolved risk there (not
stealing focus per §5, Q1's tri-state-vs-promise choice, Q5's
retry-bound question). The terminal's own creation gap had a genuinely
narrower, already-correct-elsewhere fix available, confirmed live to
work, that doesn't touch anything shared. Recommending Option B still
stands for §4's remaining scope; this section is not an argument against
it.

### 9.3 What's still open

Editor and agent panes remain stubbed exactly as §2c describes — opening
either and typing immediately still drops keystrokes into the dummy
focus element. Tab-switch focus restoration (§4's "probably in", Q2) is
untouched. §3's Option B is still the right design for closing those
in one pass rather than three more narrow, per-view patches.
