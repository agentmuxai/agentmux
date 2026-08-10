# SPEC: Restore the ghost-text suggestion when the composer is cleared back to empty

**Date:** 2026-08-10
**Status:** implemented.
**Amends:** `docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md` §4.3,
specifically its second bullet ("must clear the instant the user starts
typing"). Everything else in that spec — the AMC gateway wiring, the
Haiku prompt, guard 1 (clear on `Submitting`) and guard 4 (clear on session
end) in `useNextPromptSuggestion.ts` — is unchanged; see §2.

---

## 0. Ask

> another tweak on the haiku suggestions in the agent pane .. if you begin
> typing but then delete it all, the suggestion ghost text currently does
> not come back, it just goes back to "Send message too..." ... we need it
> to go back to the suggestion when the user clears the input .. write a
> seperate spec to file

---

## 1. Current behavior (audited against source, 2026-08-10)

The "ghost text" is the native HTML `<textarea placeholder>` attribute
(`frontend/app/view/agent/components/AgentFooter.tsx` line 933), driven by
a `createMemo` (lines 475-484):

```ts
const placeholder = createMemo(() => {
    const vm = props.viewModel;
    const suggestion = vm?.blockAtom()?.meta?.["term:next_prompt_suggestion"] as string | undefined;
    if (suggestion) return suggestion;
    // ...falls through to "Speak to..." or "Send message to <agent>..."
});
```

This memo is correct and reactive — it would show the suggestion again the
instant `term:next_prompt_suggestion` meta is non-null and the box is empty
(the browser only renders `placeholder` on an empty value; no extra code
needed for that half). **The bug is that the meta gets permanently nulled
out long before the user deletes back to empty**, at two separate points:

1. **`handleInput`, lines 587-598** — the first edit into a previously-empty
   box (tracked via a `boxWasEmpty` local, lines 512-528) unconditionally
   writes `"term:next_prompt_suggestion": null` to block meta:

   ```ts
   const newValue = textareaRef?.value ?? "";
   if (boxWasEmpty && newValue.length > 0) {
       const vm = props.viewModel;
       if (vm?.blockAtom()?.meta?.["term:next_prompt_suggestion"]) {
           fireAndForget(() =>
               ObjectService.UpdateObjectMeta(makeORef("block", vm.blockId), {
                   "term:next_prompt_suggestion": null,
               } as any)
           );
       }
   }
   boxWasEmpty = newValue.length === 0;
   ```

   This is the code path that fires for the reported bug: type one
   character → meta nulled immediately → delete back to empty →
   `placeholder()` has nothing left to read → falls through to
   `"Send message to <agent>..."`.

2. **Tab/Right-Arrow accept, lines 776-788** — accepting the suggestion into
   the real input also immediately nulls the same meta key, for the same
   apparent reason (so it doesn't come back if the user deletes what they
   just accepted).

Both are deliberate, not accidental — the `handleInput` block is a direct
implementation of `SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md` §4.3's
"must clear the instant the user starts typing their own message" bullet.
§2 explains why that requirement, as written, over-reaches what it actually
needed to guarantee.

---

## 2. Root cause: one spec bullet conflated two different guards

§4.3 of the original ghost-text spec bundles two distinct concerns into one
bullet:

> Must clear the instant the user starts typing their own message —
> independent of turn phase. If the suggestion RPC (1-3s round trip)
> resolves *after* the user has already started typing, the write must be
> dropped entirely (check "is the composer still empty" at write time, not
> just "is this still the current generation").

Read closely, this is actually describing **two separable requirements**,
and the codebase already implements them as two separate mechanisms:

- **"The write must be dropped if the RPC resolves after typing started"**
  — this is a real race and is already fully handled, independently, by
  guard 3 in `useNextPromptSuggestion.ts`:

  ```ts
  if (result.suggestion && isComposerEmpty()) {
      fireAndForget(() => ObjectService.UpdateObjectMeta(/* write suggestion */));
  }
  ```

  `isComposerEmpty()` reads the *live* textarea state at the moment the RPC
  response arrives (`AgentFooter.tsx` line 627:
  `props.isComposerEmptyRef?.(() => (textareaRef?.value.length ?? 0) === 0)`).
  This check is entirely independent of whether `term:next_prompt_suggestion`
  was ever nulled earlier — it would correctly refuse to overwrite a
  suggestion the user is actively typing over regardless of what
  `handleInput` does.

- **"Must clear the instant the user starts typing"** — this is the *other*
  half of the bullet, and it's what `handleInput`'s `boxWasEmpty` check
  actually implements. But nothing in the stated rationale (the RPC race)
  requires this — the RPC race is already closed by the guard above. The
  only actual effect of clearing on first keystroke is "the suggestion can
  never come back once the user starts typing," which is precisely the
  behavior the ask wants reversed.

**Conclusion: the RPC race this bullet worried about is already closed by a
guard that doesn't depend on clearing meta on every first keystroke.**
`handleInput`'s clear (and the Tab-accept path's equivalent clear) is a
second, unneeded belt-and-suspenders mechanism whose only visible effect is
the reported bug. It's safe to remove without reopening the race the
original spec was protecting against — that protection lives entirely in
`isComposerEmpty()`, untouched by this change.

---

## 3. Design

### 3.1 Stop clearing the suggestion on edit; only clear it on lifecycle events

Remove both clear points from §1. The suggestion in
`term:next_prompt_suggestion` is no longer tied to composer edit history at
all — it's cleared **only** by the two lifecycle events that already,
correctly, own that responsibility in `useNextPromptSuggestion.ts`:

- **Guard 1** (unchanged): cleared the instant a new turn starts
  (`Submitting`) — a suggestion from turn N is meaningless once turn N+1
  has begun.
- **Guard 4** (unchanged): cleared on session end, via
  `useBlockActivity.ts`'s `clearActivity`.

With both edit-time clears removed, the composer's placeholder becomes a
pure function of "is `term:next_prompt_suggestion` set, and is the box
currently empty" — exactly what `placeholder()` already computes, with zero
changes needed to that memo. Typing hides it (native `<textarea
placeholder>` behavior — no code involved); deleting back to empty shows it
again, because nothing ever destroyed the underlying value.

### 3.2 `handleInput` — delete the clear block and its supporting state

```ts
// Remove entirely (was lines 578-597, and the boxWasEmpty local + its
// upkeep in writeComposerValue, lines 512-528):
const newValue = textareaRef?.value ?? "";
if (boxWasEmpty && newValue.length > 0) {
    // ...clear term:next_prompt_suggestion...
}
boxWasEmpty = newValue.length === 0;
```

`boxWasEmpty` has no other reader in the file (confirmed — it's written in
`writeComposerValue` and read/written only in `handleInput`); once this
block is gone it's entirely dead and should be deleted, not left behind:

```ts
// writeComposerValue simplifies to:
const writeComposerValue = (text: string): void => {
    if (!textareaRef) return;
    textareaRef.value = text;
};
```

`handleInput` keeps everything else unchanged — history-cursor reset,
`escClearedDraft = null`, autocomplete update, the RAF-debounced
`onTyping` callback. None of that is related to the ghost-text bug.

### 3.3 Tab / Right-Arrow accept — same treatment, for consistency

Remove the explicit null-out at lines 782-786:

```ts
if (suggestion) {
    e.preventDefault();
    setComposerValue(suggestion);
    // Remove: the ObjectService.UpdateObjectMeta(...null...) call that follows.
    return;
}
```

See §5 point 1 for why this should get the same treatment as §3.2 rather
than being left as a special case — accepting the suggestion and then
deleting it should behave the same as typing over it and then deleting it;
both are "the user changed their mind," and both should bring the same
suggestion back.

---

## 4. Edge cases

| Case | Behavior after this change |
|---|---|
| Type one character, delete it | Suggestion reappears immediately (native placeholder, box is empty again) |
| Type a full message, delete it all with backspace | Same — suggestion reappears |
| Type a full message, delete it all, then submit an *empty* box | N/A — empty submit is a no-op elsewhere in the composer; not affected by this change |
| Accept via Tab, then delete the accepted text | Suggestion reappears — §3.3, §5 point 1 |
| Accept via Tab, then submit as-is | `Submitting` fires → guard 1 nulls the suggestion → next turn starts clean, same as today |
| Type a message, submit it (never deleted back to empty first) | `Submitting` clears it via guard 1 before any new turn — unaffected by this change, since the box was never empty again before submission |
| Suggestion RPC resolves while the user is actively typing (non-empty box) | Still dropped — `isComposerEmpty()` guard 3 in the hook is untouched by this spec (§2) |
| User deletes back to empty, walks away, a *new* turn's suggestion RPC resolves later | Guard 1 already cleared the old suggestion when that new turn's `Submitting` fired, so the new RPC's result (if the box is still empty when it resolves) simply overwrites with the fresh suggestion — no stale leftover |
| Session ends with a suggestion sitting in meta, box empty | Guard 4 (`clearActivity`) still clears it — unaffected by this change |
| User sends a message while a suggestion is showing | Composer clears synchronously, but guard 1's clear is an async RPC — without §9's fix, the box briefly re-shows the just-sent turn's stale suggestion until that RPC lands |

---

## 5. Resolved design decisions

1. **Should Tab-accept get the same "never actually clears on edit" treatment
   as typing, or keep its own explicit clear? — resolved: same treatment.**
   Considered leaving Tab-accept's clear in place (on the theory that
   explicitly *accepting* a suggestion is a more deliberate action than
   incidentally typing over one, so maybe it should be "consumed" for
   good). Rejected for consistency: a user who accepts via Tab and then
   deletes the text has, behaviorally, ended up in the exact same state — an
   empty box, no message sent — as a user who typed the same text by hand
   and deleted it. Treating those two paths differently would be a special
   case with no clear benefit and one extra thing to explain; §3.1's single
   rule ("only lifecycle events clear it") is simpler and covers both.
2. **Does removing `handleInput`'s clear reopen the RPC race §4.3 of the
   original spec was worried about? — resolved: no.** §2 traces that race
   to `isComposerEmpty()` (guard 3 in `useNextPromptSuggestion.ts`), which
   is untouched by this spec and was always the mechanism actually closing
   it — `handleInput`'s clear was redundant for that purpose, not load-
   bearing.
3. **Does the suggestion need a new expiry (e.g. don't show it if it's been
   sitting unaccepted for N minutes)? — resolved: no, out of scope.** Not
   asked for, and the two existing lifecycle clears (new turn, session end)
   already bound how long a stale suggestion can persist to "at most one
   turn." Revisit only if that turns out to feel stale in practice.

---

## 6. Non-goals

- No change to the AMC gateway, the Haiku prompt, or when the suggestion
  RPC is triggered (`turnJustEndedAtom`) — entirely orthogonal to this fix.
- No change to `isComposerEmpty()` / guard 3's late-RPC-write protection —
  confirmed independent and untouched (§2).
- No change to guard 1 (clear on `Submitting`) or guard 4 (clear on session
  end) in `useNextPromptSuggestion.ts` — **their firing conditions are
  unchanged; their write bodies gained a paired generation-counter bump as
  of §10.** Originally written as "no useNextPromptSuggestion.ts changes at
  all," which stopped being true once §10's fix landed — noted here
  explicitly rather than left to drift, per the same lesson §9 already
  documents about stale claims in this file.
- No new UI affordance (e.g. an explicit "dismiss suggestion" button) — the
  suggestion simply follows the box's emptiness now, same interaction
  model as today, just without the premature one-way clear.

---

## 7. Files touched

- `frontend/app/view/agent/components/AgentFooter.tsx` — remove the
  `boxWasEmpty`-gated clear block in `handleInput` (§3.2), remove the
  `boxWasEmpty` local and its upkeep in `writeComposerValue` (§3.2), remove
  the explicit meta-null after Tab/Right-Arrow accept (§3.3). Update the
  doc comments at lines 512-523 (currently explaining `boxWasEmpty`'s
  purpose) and 578-586 (currently citing the old §4.3 requirement this spec
  amends) to reflect the new, simpler rule. §9/§10 (post-review): new
  `suggestionGenMaskedAtSend` signal + its read in `placeholder` (compared
  against a generation number, not suggestion text — §10) + its write in
  `handleSend`.
- `frontend/app/view/agent/hooks/useNextPromptSuggestion.ts` — §10
  (post-review): new module-level `suggestionGenCounter` + `writeSuggestionMeta`
  helper, used by both `clearSuggestion` (guard 1) and the RPC
  success-write, pairing every write to `term:next_prompt_suggestion` with
  a bump to a new `term:next_prompt_suggestion_gen` meta key. Guards'
  firing conditions unchanged.
- No `agentmux-srv` (Rust) changes. No wire-format changes — this is a
  pure frontend edit-state fix.

---

## 8. Test plan

**Unit** (`AgentFooter.test.tsx`, if it covers the composer's ghost-text
behavior — confirm exact file/coverage at implementation time):

- With `term:next_prompt_suggestion` set in block meta: simulate typing one
  character into the composer, then deleting it. Assert the meta key is
  still set (not nulled) and the placeholder reflects the suggestion once
  the box is empty again.
- Same, but typing a full sentence and deleting it via repeated backspace.
- Accept via simulated Tab keydown on an empty box with a suggestion set;
  assert the composer now contains the suggestion text and the meta key is
  *still* set; then clear the composer and assert the placeholder shows the
  suggestion again.
- Simulate `Submitting` phase (or the equivalent call into
  `useNextPromptSuggestion`'s guard 1) with a suggestion present in meta;
  assert it's nulled, matching today's unchanged behavior.
- Simulate the late-RPC race directly against `useNextPromptSuggestion.ts`
  (not `AgentFooter.tsx`): resolve the RPC after `isComposerEmpty()` would
  return `false`; assert no write happens — regression guard confirming §2's
  claim that this protection doesn't depend on anything in `AgentFooter.tsx`.

**Manual / integration:**

- `task dev`, let a turn finish so a suggestion appears as placeholder text.
  Type a character, delete it — confirm the suggestion reappears instead of
  falling back to "Send message to `<agent>`...".
- Same, but type a full message and delete it all with backspace instead of
  one character at a time.
- Press Tab to accept the suggestion, then delete the accepted text back to
  empty — confirm the same suggestion reappears (§3.3, §5 point 1).
- Accept via Tab and actually submit the message — confirm the *next* turn
  starts with no stale suggestion showing before its own RPC resolves.
- Type a message and submit it without ever deleting back to empty first —
  confirm normal behavior, unaffected by this change.

---

## 9. Post-review fix: stale suggestion flashing after send (P1, caught by reagentx)

Removing `handleInput`'s clear-on-first-keystroke closed the reported bug,
but reopened a narrower, real one: **`handleSend` clears the composer
synchronously; `useNextPromptSuggestion.ts` guard 1 clears the meta key
asynchronously** (a fire-and-forget `ObjectService.UpdateObjectMeta` RPC,
triggered off the `Submitting` phase transition). Before this spec, that
gap was invisible — `handleInput` had already nulled the meta the moment
the user's first keystroke landed, long before they got around to hitting
Send. With that early clear gone, every send now has a real window where
the box is empty (native placeholder wants to render) and the *previous*
turn's suggestion is still sitting in meta, briefly flashing as placeholder
text until the RPC round-trips.

**Fix:** `AgentFooter.tsx` gained a `suggestionMaskedAtSend` signal.
`handleSend` snapshots whatever suggestion is currently in meta into it,
right alongside its existing synchronous composer-clear; the `placeholder`
memo skips band 1 whenever the live suggestion still equals that snapshot.
Pure value comparison, no explicit reset needed — the moment the backend's
async clear (or a later turn's fresh suggestion) actually lands, the live
value no longer matches the snapshot and band 1 resumes on its own.

**A first attempt at this fix silently didn't work**, caught only by its
own regression test, not by inspection: it used a plain `let
suggestionMaskedAtSend` mutated directly inside `handleSend`. That mutation
never touches Solid's reactive graph, so the `placeholder` `createMemo` —
which only re-runs when one of *its own* tracked signal reads changes —
never re-evaluated when the plain variable changed. The fix looked correct
reading the diff; it took an actual test asserting the placeholder's live
value after a simulated send (not just that the right values were
assigned) to reveal it did nothing. Fixed by making it a real
`createSignal`, read inside `placeholder` (so it's a tracked dependency)
and written via its setter in `handleSend`. Lesson for this class of bug:
a "capture at write time, compare at read time" pattern only closes a race
if the write side actually participates in whatever reactivity the read
side depends on — a plain closure variable is invisible to a `createMemo`
no matter how correctly timed the write is.

**Test plan addition**, folded into the file already covered above: two
cases render an `AgentFooter` with a `viewModel` mock, type and send a
message, and assert `ta.placeholder` immediately after — one with a
never-updating mock `blockAtom` (models the RPC never landing, the worst
case) confirming the default placeholder shows instead of the stale
suggestion, and one with a `createSignal`-backed mock confirming a
genuinely new suggestion arriving later still displays normally (the mask
doesn't shadow it forever).

---

## 10. Second post-review fix: text-value masking collides on a repeated suggestion (P1, caught by reagentx)

§9's fix compared the *text* of the masked suggestion against the live
text. That has a real collision the re-review caught: if a later turn's
genuinely fresh suggestion happens to be the exact same string as the one
masked at the previous send — a plausible repeat, e.g. Haiku predicting
"Run the tests" after two unrelated turns — `suggestion !==
suggestionMaskedAtSend()` evaluates `false` for a suggestion that is not
actually stale, and band 1 stays wrongly suppressed until the *next* send
resets the mask. The regression test added in §9 only exercised a
differently-worded follow-up suggestion ("Check the logs"), so it didn't
exercise this path at all.

**Root problem:** text-value equality was always the wrong proxy for "has
this been re-written since I captured it." What's actually needed is
identity of the *write event*, not identity of its *payload*.

**Fix:** `useNextPromptSuggestion.ts` now pairs every write to
`term:next_prompt_suggestion` — both the RPC success-write and guard 1's
clear — with a bump to a new `term:next_prompt_suggestion_gen` meta key, via
a shared `writeSuggestionMeta` helper and a module-level monotonic counter
(shared across every pane's hook instance; still strictly increasing and
still unique per write when shared, and sharing it avoids per-instance
bookkeeping AgentFooter.tsx never needed — each block only ever compares
its own gen against its own earlier snapshot). `AgentFooter.tsx`'s mask
(`suggestionGenMaskedAtSend`, renamed from `suggestionMaskedAtSend`) now
snapshots and compares the generation number instead of the text. Any real
write — same text or not — bumps the counter, so the collision above is
structurally impossible now, not just less likely.

**One accepted edge case, not fixed:** a suggestion already sitting in meta
with no `_gen` key at all (e.g. left over from a session running the
pre-§10 build) reads as `suggestionGen === undefined`, which matches the
mask's own initial `undefined` default — so on the very first send after
an in-place upgrade, band 1 could be wrongly suppressed once, the same
failure mode as before this fix, not the P1 bug reappearing. Judged
acceptable: narrow (one suggestion, one upgrade transition), and it fails
toward under- rather than over-showing, consistent with this feature's
existing risk posture (guard 3 already prefers dropping a suggestion over
risking a stale/wrong one).

**Test plan addition:** a new case sets an identical suggestion string with
an incremented generation number after a simulated send, and asserts the
placeholder shows it — pinning the exact collision the text-based version
missed.
