# PLAN: browser pane form filling, fixes from the first real multi-step form

**Status:** active — Step 1 built in #5089: §2.1 (limits shown, over-long fills refused, a warning when the page changed a value), §2.2 (`key` reads back what has focus), §2.4 (review sections get references; tool guidance), §4.1 (screenshots black out secret fields in every frame, fail closed), plus §3.6 (`about:` frames inherit their parent's origin). Steps 2 and 3 not built.
**Date:** 2026-10-17
**Author:** Korp@narko, at the operator's request
**Trigger (operator, 2026-10-17):** "for the 350 characters, the agent filling the form could have known that. so is that something we can fix for the future?" … "were there other learnings from this? lets put it all into a fix plan, write to file"
**Builds on:** `SPEC_BROWSER_PANE_FORMS_IN_FRAMES_AND_VISUAL_FALLBACK_2026_10_16.md` (frames, F2 shipped in v0.59.31), `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` (snapshot, act by reference, secret guard, approval).

---

## 1. What happened

An agent filled a real eight-step government form for the operator in AgentMux 0.59.31, the first real use of iframe support. The whole wizard sits in an iframe; the user signed in (hand-off), typed the two identification numbers (secret fields), checked the review and submitted. The agent filled everything else by reference.

It worked, and the filing went through. But:

- **One long text answer was cut short.** The field held 350 characters; the agent's text had 383. The filed form ends mid-word. The agent was told the fill succeeded.
- Several steps took workarounds: a date field set by pressing arrow keys one at a time and checking screenshots, a `wait` that timed out although the text was on screen, a review page whose sections the agent couldn't open, and an approval banner for a download link.

This plan lists every gap found, its cause in the code, and the fix, in priority order. Every item has a fixture test; the test page in §5 covers all of them.

## 2. P1: the agent was told something false, or couldn't check its work

### 2.1 Length limits are invisible, and fill silently overflows them

- **What happened:** a textarea with `maxlength=350`. The snapshot line showed no limit. `fill` inserted 383 characters; the browser kept 350 (inserted text respects `maxlength`); the read-back differed, so `fill` fell back to the native `value` setter (`act.rs` fill, `SET_VALUE`), which ignores `maxlength`, and reported the full text as the value. On submit the site kept its 350.
- **Fix:**
  1. The snapshot shows the limits a person sees or the browser enforces: `[maxlength=350]`, `[minlength=…]`, `[pattern]`, and the field's `inputmode`, on every value field (`snapshot.rs` `line_for`, from `DomFacts.attrs`, which `DOM.describeNode` already returns).
  2. `fill` refuses, before typing, text longer than the field's `maxlength`: "this field holds at most 350 characters; your text has 383: shorten it and fill again". Nothing is typed.
  3. The native-setter fallback is never used to get past a limit: if the read-back is a prefix of the text and the field has a `maxlength`, report it as cut, don't force it.
  4. Any read-back that differs from the text (a mask reformatted it, a script trimmed it) comes back as a warning in the result, with both values, not as plain success.
- **Test:** fixture field with `maxlength=350`, fill 383 characters: refused, field unchanged. Fill 350: accepted. A field whose script trims to 100 characters without `maxlength`: result warns with both values.

### 2.2 `key` reports "Dispatched" whether or not anything happened

- **What happened:** typing digits into a native date input with `key` text returned "Dispatched"; the field stayed empty (the date segments read key events, not inserted text). Only a screenshot showed it.
- **Fix:** `key` returns the focused element's read-back after the keys (value, checked, validation message), like `fill`, and says so when the value didn't change ("the focused field's value didn't change"). Secret fields stay redacted (`redact_secret`).
- **Test:** focus a date input, `key` text `10152026`: result shows the value unchanged and says so.

### 2.3 `wait` for text doesn't see text inside frames

- **What happened:** `wait until_text "Step"` timed out after 30 s while "Step 2 of 8" was on screen, inside the iframe. `wait_for_route` evaluates `document.body.innerText` in the top frame only (`act.rs` around line 1064).
- **Fix:** `until_text` checks the top frame and every frame `frames::current_frames` finds (each through its own session), and says which frame matched. `until_gone` accepts frame references (today refused).
- **Test:** text present only in a cross-origin frame: `wait` succeeds and names the frame.

### 2.4 The review page couldn't be opened, so the cut text wasn't caught before signing

- **What happened:** the wizard's review step shows each section as a collapsible `<summary>`. The snapshot rendered them as `DisclosureTriangle "…" [expanded=false]` with no reference, so the agent couldn't open them and compare the review to what it had entered. The cut in §2.1 would have shown there.
- **Fix:**
  1. Add `DisclosureTriangle` (and `details`/`summary`) to `INTERACTIVE_ROLES` (`snapshot.rs`), so it gets a reference and can be clicked.
  2. Browser tool guidance (`tool_schemas.rs`): before a committing click, read the site's own review or summary page and compare it to what you entered; report any difference to the user.
- **Test:** fixture review page with collapsed sections: the agent opens each by reference and reads the values.

### 2.5 The agent can't tell an approved click from one that never asked

- **What happened (in testing):** a test agent twice reported "no approval banner appeared" when the log showed the banner and the user's Approve; the click simply blocked until then, and the agent can't see wall-clock time.
- **Fix:** a click that waited for the user returns `approved_by_user: true` and `waited_ms`; a bypassed one already says `bypassed`. The tool text says "the user approved this click after 12 s".
- **Test:** committing click, approve after a delay: result carries both fields.

## 3. P2: the agent had to work around it

### 3.1 Native date, time and month inputs can't be filled

- **What happened:** the snapshot showed the date control as `Date "…"` (no reference) with three `spinbutton` segments (`value="0"`); `fill` refuses a spinbutton; inserted text is ignored. Setting the date took a click, then 25 arrow-key presses checked by screenshots.
- **Fix:** the date control itself gets a reference and shows its value (`value="2026-10-15"`). `fill` on it accepts common forms (`2026-10-15`, `10/15/2026`, `October 15, 2026`), converts to the input's own format and sets it through the native setter with `input` and `change` (spec §4.4, step 1). The read-back shows what the page holds.
- **Test:** fill `October 15, 2026` into `type=date`: read-back `2026-10-15`, and the form's own validation passes.

### 3.2 A link whose text contains "confirm" needs the user's approval

**Owner:** Naki-2, in their browser small-fixes PR (agreed 2026-10-17), as written here, plus: classification reads the label's full text, so a later line can only make a click ask, never stop it asking.

- **What happened:** a download link reading "This acknowledgement … does not confirm the accuracy … Click here to download a PDF" was treated as committing (`COMMIT_INFO` verb list, `act.rs` line 193, matches anywhere in the name) and the user had to approve or refuse a download.
- **Fix:** for links outside a form, match the verbs only in a short name (at most about six words) or at its start; a link with a `download` attribute or to a file URL is never committing. Buttons keep today's rule. The approval banner says why a click counted as committing.
- **Test:** the long link above: no approval. A link "Confirm order": approval.

### 3.3 Scoped snapshots don't work inside frames

- **What happened:** `scope` on a frame reference is refused (known F2 limit), so checking one field meant a full snapshot or a screenshot.
- **Fix:** support `scope` on frame references (render that frame's subtree; references `f2e17.e1`), as spec §4.2 item 6 already proposes.

### 3.4 Misleading errors after the page changed

- **What happened:** after a survey popup closed itself, and after the user changed the page during a hand-off, clicks on old references failed with "the element is not visible", which reads like a layout problem.
- **Fix:** when an element is detached or zero-size, check whether the frame's document changed since the snapshot; if so say "the page changed since the snapshot (the element is gone); take a new snapshot".

### 3.5 Long `select` lists flood the snapshot

- **What happened:** two state dropdowns printed all 59 options on every snapshot (about 120 lines per step).
- **Fix:** a closed native `select` shows `value=` and `[59 options]`; its options are listed only when it is expanded, scoped, or has 12 options or fewer. `select` by label still works without them.

### 3.6 Frames with an inherited origin show as `origin=unknown cross-origin`

- **What happened:** the wizard's frame (an `about:srcdoc` or `about:blank` frame) had no security origin of its own, so it was labelled `origin=unknown cross-origin`, although it inherits its parent's origin.
- **Fix:** an `about:` frame inherits its parent frame's origin for the label and for the coming frame origin rules (spec §5.3).

### 3.7 An empty `scope` is treated as a reference

- **What happened:** `snapshot` with `scope: ""` failed with `unknown reference ""`.
- **Fix:** an empty `scope` means no scope (MCP side, `browser.rs`).

## 4. P3: safety items made urgent by real use

### 4.1 Screenshots don't hide secret fields yet

The agent took many screenshots while the user's identification number sat in a masked field. A page's show/hide button would have put it in the agent's context. Spec §5.1 item 4 (black out every secret field's box before a screenshot reaches the agent) moves up to build next, with §2.1.

### 4.2 Business identification numbers are treated as secret

A field labelled "Service recipient TIN" (a company's employer number) was marked secret, so the user typed a number that isn't private. This is the accepted false positive of the tax-ID guard (spec §5.1 item 2); keep it, but the refusal message should say "if this is a business's EIN, the user can type it; it isn't secret, but AgentMux can't tell".

## 5. Fixture and order of work

Extend the frames fixture (`frames` test page: top frame, same-origin and cross-origin frames, nested frames) with:

- a `maxlength=350` textarea and a field a script trims without `maxlength`;
- a native `type=date` input;
- a review page with `<details>`/`<summary>` sections that echo the entered values;
- a long link containing "confirm" with a `download` attribute;
- a 59-option `select`;
- an `about:srcdoc` frame holding a form;
- a popup that closes itself after a few seconds.

| Step | Items | Why first |
|---|---|---|
| 1 | §2.1 limits, §2.2 key read-back, §2.4 review sections, §4.1 screenshot blackout | the agent was told something false, or saw what it shouldn't |
| 2 | §2.3 wait in frames, §2.5 approval result, §3.1 date fill | the workarounds that cost the most time |
| 3 | §3.2 link verbs, §3.3 scope in frames, §3.4 errors, §3.5 select lists, §3.6 origins, §3.7 empty scope, §4.2 message | polish |

## 6. Other learnings (not browser code)

- **Agents can't see wall-clock time.** A test agent's "it returned at once" was wrong twice; timing claims must come from logs (`browser_attention_resolve decision=…` in the host log). §2.5 puts the facts in the tool result.
- **The approval banner's Approve is easy to hit by mistake** (the user approved three times when asked to cancel during testing). Consider making Cancel the first, focused button for clicks that submit, or naming the button for the action ("Submit form").
- **The operator types secrets; the agent checks the result.** The split worked well: the user signed in and typed the two identification numbers and pressed the final Submit; the agent filled the rest and verified computed totals on screen before moving on. Write it into the Browser tool's guidance as the pattern for forms with identity numbers.
