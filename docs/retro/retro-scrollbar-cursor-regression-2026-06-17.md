# RETRO: The scrollbar-cursor "fix" that wasn't — inheritance, not deletion

**Date:** 2026-06-17
**Status:** Post-mortem. The original bug is **still live**, in a new shape, and the
guardrail we shipped now *blocks* the correct fix.
**Scope:** `frontend/` cursor styling on scrollbars, app-wide.
**Subjects:** PR #1453 (`fix(ui): scrollbars use the arrow cursor, not the link hand`)
and PR #1455 (`chore(ui): cursor design tokens + utilities + scrollbar lint guard`),
both landed 2026-06-15.
**Prior art:** `docs/analysis/ANALYSIS_CURSOR_STYLING_2026_06_15.md` (the plan these
two PRs executed).

---

## 0. TL;DR

- We set out to make scrollbars show the **default arrow** instead of the link **hand**.
- The fix **deleted** the `cursor:` declaration from the scrollbar thumb, on the belief
  that "no cursor → inherits the default arrow."
- That belief is **false**. `cursor` is an **inherited** CSS property, and a WebKit
  `::-webkit-scrollbar*` pseudo-element takes the **computed cursor of its scrollable
  host element** — not the OS default. Deleting the declaration didn't pin the arrow; it
  handed the scrollbar whatever cursor its content had.
- Result, today, verified in the current tree:
  - **Main agent-pane conversation scrollbar → text I-beam.** Its host
    `.agent-document` sets `cursor: text` (`_document.scss:16`).
  - **Live-tool log scrollbar → link hand.** Its host `.agent-tool-overlay-log` inherits
    `cursor: pointer` from the clickable `.agent-tool-block`
    (`_document-nodes.scss:141`).
  So the same bug class now manifests **two different wrong cursors** depending on which
  surface you hover. The arrow appears nowhere we intended.
- The **guardrail makes it worse.** The stylelint rule we added forbids `cursor` on any
  `::-webkit-scrollbar*` selector. The *only* reliable fix is to set `cursor` on exactly
  those pseudo-elements. **The rule bans the fix and mandates the failure mode** — and
  its error message teaches the wrong model to the next engineer.

The deletion didn't fix the bug. It **traded** a uniformly-wrong cursor (always the
hand) for a **context-dependently-wrong** cursor (text here, hand there), which is
strictly harder to notice and reason about.

---

## 1. What we shipped vs. what we believed

**PR #1453 — the "fix".** Removed two `cursor: pointer` declarations:

`frontend/app/app.scss:68-75` (current state):
```scss
*::-webkit-scrollbar-thumb {
    // No cursor override: a scrollbar is a scroll affordance, not a link, so it
    // keeps the default arrow. (Was `cursor: pointer`, which showed the link
    // hand on every scrollbar — including agent panes.)
    background-color: var(--scrollbar-thumb-color);
    border-radius: 0;
    margin: 0 1px 0 1px;
}
```

The OverlayScrollbars handle was treated **differently** — it got an *explicit* value:

`frontend/app/app.scss:105-112`:
```scss
.os-scrollbar-handle {
    cursor: var(--cursor-default);   // explicit arrow — CORRECT
}
```

**PR #1455 — the "systemic architecture".** Added `--cursor-*` tokens (`theme.scss`),
`.u-cursor-*` utilities (`app.scss`), and the guardrail:

`.stylelintrc.json:15-22`:
```json
"rule-selector-property-disallowed-list": [
    { "/-webkit-scrollbar/": ["cursor"] },
    { "message": "Scrollbars are scroll affordances, not links — never set `cursor`
       on a `::-webkit-scrollbar*` selector. Remove it so the thumb keeps the default
       arrow (it inherits it). See docs/analysis/ANALYSIS_CURSOR_STYLING_2026_06_15.md." }
]
```

**The belief, stated plainly** (analysis doc §0, §1, §4):
> "Deleting those two declarations fixes the reported issue **everywhere** with zero
> risk." … "The scrollbar then inherits the correct arrow."

Both the code comment and the lint message encode the same assumption: *inherit ⇒ arrow*.
That single assumption is the root cause of this retro.

---

## 2. The actual mechanism (verified)

`cursor` is an **inherited** property (CSS UI spec). A `::-webkit-scrollbar*`
pseudo-element is generated for, and inherits from, the element it scrolls. So:

> **cursor shown over a native scrollbar = computed `cursor` of the scroll-host element.**

With no declaration on the thumb, there is nothing to override that inheritance. The
"default arrow" only appears when the host's computed cursor *happens* to be `auto`/
`default`. On the surfaces users actually scroll, it isn't.

### 2.1 Main conversation scrollbar → text I-beam

The main agent conversation scroll container is a **native** scrollbar (not
OverlayScrollbars):

`frontend/app/view/agent/styles/_document.scss:8-16`:
```scss
.agent-document {
    flex: 1;
    overflow-y: auto;        // ← the main scroll container
    ...
    user-select: text;
    cursor: text;            // ← inherited by ::-webkit-scrollbar-thumb → I-beam
}
```

`.agent-document` legitimately wants `cursor: text` so the conversation reads as
selectable prose. But that cursor bleeds onto its scrollbar, which now has no declaration
of its own to stop it. **→ text I-beam over the main scrollbar.** Confirmed.

> Note: the OverlayScrollbars instances in `markdown.tsx` (per-block content) *are* fixed
> — `.os-scrollbar-handle` sets an explicit `cursor: var(--cursor-default)`. But the
> surface the user calls "the main scrollbar" is the native `.agent-document` one, which
> got the deletion treatment, not the explicit-default treatment. The two scrollbar
> systems were fixed **inconsistently**, and only the one nobody calls "main" is correct.

### 2.2 Live-tool log scrollbar → link hand

`frontend/app/view/agent/styles/_tool-overlay-portal.scss:34-37`:
```scss
.agent-tool-overlay-log {
    overflow-y: auto;        // ← live-log scroll container, sets NO cursor
    ...
}
```
nested inside

`frontend/app/view/agent/styles/_document-nodes.scss:132-141`:
```scss
.agent-tool-block {
    ...
    cursor: pointer;         // ← clickable block; inherited down to the log + its scrollbar
}
```

The live-log container sets no cursor, so it inherits `pointer` from the clickable
`.agent-tool-block` ancestor, and its native scrollbar inherits that in turn.
**→ link hand over the live-tool scrollbar.** Confirmed.

### 2.3 The general shape

Any scrollable region nested under a clickable ancestor (`cursor: pointer`) or a text
surface (`cursor: text`) now shows the wrong cursor on its native scrollbar. The agent
pane is dense with both. We didn't fix a bug; we **distributed** it across every scroll
host according to that host's content cursor.

---

## 3. Why the guardrail is inverted

The lint rule forbids `cursor` on `::-webkit-scrollbar*` selectors. But §2 shows the
**only** way to stop a scrollbar from inheriting its host's cursor is to set `cursor`
**on that very pseudo-element**. Therefore:

- The correct fix — `*::-webkit-scrollbar-thumb { cursor: default; }` (and on the track /
  the bar) — is now a **lint error**.
- The rule **mandates** the exact construct that produces the bug (no declaration → inherit).
- The rule's message — *"Remove it so the thumb keeps the default arrow (it inherits
  it)"* — actively **teaches the false model** to whoever hits it next. A guardrail that
  documents the wrong mental model is worse than no guardrail: it converts a one-time
  mistake into institutional knowledge.

We built a fence and put it on the wrong side of the cliff.

---

## 4. Why it wasn't caught

1. **A plausible-but-wrong CSS mental model went unverified.** "Removing a property
   reverts to the default" is true for many properties and false for inherited ones. No
   one checked the spec or tested the assumption. The word "inherits" appears in the fix
   comment, the analysis doc, *and* the lint message — the wrong model was confidently
   restated three times, which read as corroboration rather than a single unchecked claim.

2. **"Zero risk / fixes everywhere" shut down testing.** The analysis asserted the change
   was risk-free and universal (§0, §4). That framing is exactly what should have
   triggered a 60-second manual check — and instead excused skipping it. The repo ships a
   `/verify` skill and CLAUDE.md emphasizes running the app; neither was used to hover the
   two scrollbars in their two states.

3. **The originating report was treated as one site, not a class.** The report was
   "agent pane scrollbar shows the hand." We fixed *the declaration that produced the
   hand* rather than *establishing what governs a scrollbar's cursor*. Had we asked the
   general question, the inheritance behavior — and the text/pointer hosts — would have
   surfaced immediately.

4. **The two scrollbar systems diverged silently.** OverlayScrollbars got an explicit
   `cursor: default`; native WebKit got a deletion. Nobody flagged that the same intent
   was implemented two contradictory ways, one correct and one not.

5. **Tokens were defined but never reach the bug site.** `--cursor-default` exists, but
   the native thumb can't use it (the lint rule forbids it), so the pixel the user
   complained about is governed by *none* of the new architecture — only by inheritance.
   The "systemic" PR added a system that structurally cannot touch the reported bug.

---

## 5. The correct fix

**Principle:** to force a scrollbar's cursor you must set it **on the scrollbar
pseudo-elements**, because inheritance from the scroll host is otherwise unavoidable.

1. **Set the arrow explicitly on the native scrollbar pseudo-elements**, not just the
   thumb (the track and the bar are separate pseudo-elements and inherit independently):
   ```scss
   *::-webkit-scrollbar,
   *::-webkit-scrollbar-track,
   *::-webkit-scrollbar-thumb,
   *::-webkit-scrollbar-corner {
       cursor: default;   // == var(--cursor-default); see token note below
   }
   ```
   Use `default` (the keyword) if the lint rule's value-allowlist requires a literal;
   otherwise route through `var(--cursor-default)` to match the OS-handle treatment and
   keep both scrollbar systems consistent.

2. **Invert the guardrail.** It must stop *forbidding* `cursor` on scrollbar selectors
   and instead **require** `cursor: default`/`var(--cursor-default)` there (or, minimally,
   **forbid `cursor: pointer`/`cursor: text` specifically**, which were the actual bugs).
   Rewrite the message to state the real rule: *"A scrollbar pseudo-element inherits its
   host's cursor; pin it to the arrow explicitly — do not rely on the absence of a
   declaration."*

3. **Keep the OverlayScrollbars handle as-is** — `.os-scrollbar-handle { cursor:
   var(--cursor-default); }` is already the correct pattern. Now both systems match.

4. **Verify before merge** (this is the non-negotiable step that was skipped): run the
   app, hover (a) the main conversation scrollbar, (b) a live-tool log scrollbar, (c) a
   plain scrollable surface, in light and dark themes. All three must show the arrow.

> Optional hardening: a host-level `cursor: text`/`pointer` is what bleeds through.
> Pinning the pseudo-elements (step 1) is the robust fix and is independent of how many
> hosts set odd cursors — which is why it's preferred over chasing each host.

---

## 6. Lessons

- **"Removing a property = default" is false for inherited properties.** `cursor`,
  `color`, `font`, `visibility`, `white-space`, etc. inherit. To pin one you must *set*
  it, not delete it. Verify inheritance behavior before "simplifying by deletion."
- **A guardrail must encode the *true* rule.** Ours codified the bug. Before shipping a
  lint rule, prove that the construct it mandates actually produces the desired result —
  the rule's own message is documentation that outlives the PR.
- **"Zero risk, fixes everywhere" is a smell, not a reassurance.** Universal + risk-free
  claims should *raise* the bar for a manual check, not lower it.
- **Fix the class, not the line.** Ask "what *governs* this?" before deleting the
  declaration that happens to produce the symptom.
- **Don't implement one intent two ways.** The native-vs-OverlayScrollbars divergence
  (delete vs. explicit-default) was the tell that the model was unsettled.
- **A "systemic architecture" that can't reach the reported pixel isn't a fix.** Tokens
  + utilities + a guard are only worth their churn if they govern the actual bug site.

---

## 7. Action items

| # | Action | Status |
|---|--------|--------|
| 1 | Pin `cursor: var(--cursor-default)` on `*::-webkit-scrollbar`, `-track`, `-thumb`, `-corner` (§5.1) | ✅ done — `app.scss` |
| 2 | Replace the inverted stylelint ban with a value-scoped grep gate that forbids `pointer`/`text` and allows the arrow (§5.2) | ✅ done — removed `.stylelintrc.json` entry; added `scripts/check-scrollbar-cursor.sh`, wired into `task build:frontend` |
| 3 | Correct the `app.scss` comment, the `theme.scss` token comment, and the analysis doc — strike "inherits the arrow" | ✅ done |
| 4 | Manually verify all three surfaces × both themes before merge (§5.4) | ✅ done — verified in `task dev` (2026-06-17): main conversation scrollbar = arrow (was I-beam), live-tool log scrollbar = arrow (was hand), other surfaces = arrow |
| 5 | Note on the cursor token block: "scroll pseudo-elements must pin the cursor; they inherit otherwise" | ✅ done — `theme.scss` |

> **Guard self-test (2026-06-17).** `scripts/check-scrollbar-cursor.sh` was
> verified against a fixture: it flags `cursor: pointer`, `cursor: text`,
> `var(--cursor-interactive)`, and `var(--cursor-text)` on scrollbar selectors
> (including multi-line selectors and nested component rules), and passes
> `var(--cursor-default)`, cursor-less scrollbar blocks, and `pointer` on
> non-scrollbar selectors. It is green on the current tree.

**Files referenced:** `frontend/app/app.scss` (59-112), `frontend/app/theme.scss`
(301-312), `frontend/app/view/agent/styles/_document.scss` (8-16),
`frontend/app/view/agent/styles/_document-nodes.scss` (132-141),
`frontend/app/view/agent/styles/_tool-overlay-portal.scss` (34-37),
`.stylelintrc.json` (15-22),
`docs/analysis/ANALYSIS_CURSOR_STYLING_2026_06_15.md`.
