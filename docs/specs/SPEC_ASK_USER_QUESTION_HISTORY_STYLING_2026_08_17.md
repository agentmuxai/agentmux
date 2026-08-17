# SPEC: Answered questions render as user input + inverted user-input surface

**Date:** 2026-08-17
**Author:** Lark
**Status:** Implemented

---

## TL;DR

Two coupled changes to how user-authored content reads in the agent pane's history:

1. **An answered `AskUserQuestion` scrolls into history looking like a normal user
   message, not a collapsed tool row.** Today, once a question is answered, the node
   falls through to the generic `ToolBlock` collapsed-row rendering (`✓ ❓ Answered —
   Yes`) — visually identical to a finished `Read`/`Bash` call. The answer text is,
   substantively, something the user said; scrolling back up should show it the same
   way a typed message shows, using the same `.agent-user-message` treatment.
2. **User input becomes the color negative of normal output, per theme.** Today
   `.agent-user-message` is a subtle amber-tinted variant of the ambient block
   background (`color-mix(var(--user-input-color) 30%, var(--block-bg-solid-color))`)
   with normal foreground text. Change it to a true inverted surface: background =
   the theme's normal *text* color, text = the theme's normal *background* color —
   so on a theme that renders white-on-black, user input renders black-on-white, and
   vice versa on a light theme. This uses tokens every theme already defines
   (`--main-text-color` / `--block-bg-solid-color`), so it inverts correctly across
   all 13 themes with no new per-theme values.

Change 2 is what makes change 1 look right: an answered question rendered with
`.agent-user-message` automatically inherits the new inverted surface, so it reads
as unmistakably "the user's" the moment it's answered.

---

## Why now

Two related complaints about the same rendering gap:

- An answered question is, in effect, user input — the user picked an option or
  typed free text. But once `status` flips to `success`, `AgentQuestionPanel`
  unmounts it (it's filtered out of `pendingQuestions()`) and the node falls through
  to `ToolBlock`'s ordinary completed-tool row: a collapsed line with a checkmark and
  a summary string, identical in kind to a finished `Read` or `Bash` call. Scrolling
  back through history, the user's own answer doesn't read any differently than the
  agent listing a directory.
- Separately, regular typed user input is currently only weakly distinguished from
  everything else in the pane — a soft amber tint at 30% mix and a 3px left border.
  The request here is a much stronger visual signal: full color inversion relative
  to normal output, so user input is unmistakable at a glance regardless of theme,
  the same way a "sent" bubble in a chat client reads differently from "received."

---

## Current state of the code

### Answered-question rendering

`frontend/app/view/agent/hooks/useAgentQuestions.ts:109-138` (`handleAnswer`) flips
the node on answer:

```ts
const flatText = outcome.answer_text.replace(/\n/g, "; ");
const summary =
    outcome.autoFilledCount === 0
        ? `❓ Answered — ${flatText}`
        : outcome.autoFilledCount === outcome.answers.length
          ? `⏱️ Auto-answered (no response in 30s) — ${flatText}`
          : `⏱️ Partly auto-answered (${outcome.autoFilledCount}/${outcome.answers.length} — no response in 30s) — ${flatText}`;
...
updated.push({ ...n, status: "success", question: undefined, summary });
```

`question` is cleared, so `pendingQuestions()` (which filters on
`status === "awaiting_answer"`) drops the node and `AgentQuestionPanel` stops
rendering it (`agent-view.tsx`, `pending={pendingQuestions}`). The node then renders
through `DocumentRow.tsx`'s ordinary `type === "tool"` branch → `<ToolBlock>`, which
has no special case for a resolved `AskUserQuestion` — it renders the same collapsed
`.agent-tool-summary` row (icon + `.agent-tool-name` = `summary`) as any other
completed tool (`ToolBlock.tsx:378-385`).

The only trace that this was ever a question is the decorated `summary` string, and
the flattened text loses real newlines (`\n` → `; `).

### User-input color

`frontend/app/theme.scss:81`:

```scss
--user-input-color: #e07832; // warm amber, complementary to --accent-color
```

`frontend/app/view/agent/styles/_document-nodes.scss:975-980`:

```scss
.agent-user-message {
    padding: 3px var(--space-1);
    background: color-mix(in oklab, var(--user-input-color) 30%, var(--block-bg-solid-color));
    border-left: 3px solid var(--user-input-color);
    border-radius: 0;
    ...
}
```

Text color is not set here — it inherits `--main-text-color`, same as every other
block. Every theme (`frontend/app/themes/*.scss`) independently defines
`--main-text-color` (normal foreground) and `--block-bg-solid-color` (normal solid
background) as a contrasting opaque pair, e.g.:

| Theme | `--main-text-color` | `--block-bg-solid-color` |
|---|---|---|
| default (dark, `theme.scss` root) | `#f7f7f7` | `rgb(0, 0, 0)` |
| light | `#0d0f11` | `#ffffff` |
| high-contrast | `#ffffff` | `#000000` |
| dracula | `#f8f8f2` | `#1e1f29` |
| catppuccin-latte | `#37394b` | `#ffffff` |

(Full set checked: dracula, gruvbox, gruvbox-light, monokai, nord, tokyo-night,
midnight, catppuccin, catppuccin-latte, solarized-light, high-contrast, light — all
13 themes hold this contrasting-opaque-pair invariant. No theme leaves either token
translucent, so both are safe to use as solid fill/text on top of each other.)

---

## Target state

### 1. Inverted `.agent-user-message` surface

Swap the base surface to use the theme's own fg/bg tokens in reverse, instead of a
tint of the ambient block background:

```scss
.agent-user-message {
    padding: 3px var(--space-1);
    background: var(--main-text-color);
    color: var(--block-bg-solid-color);
    border-left: 3px solid var(--user-input-color);
    border-radius: 0;
    ...
}
```

This requires **no new theme tokens** — every theme already supplies both halves of
the pair, so the inversion is automatically correct everywhere: white-on-black
themes get black-on-white user input; the `light` theme (black-on-white normally)
gets white-on-black user input. `--user-input-color` is kept as the left-border
accent only — it's the existing at-a-glance identity marker from
`SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md` and doesn't
conflict with the inversion (the border sits on the edge, not the body surface).

Every other `color-mix(..., var(--block-bg-solid-color))` inside this rule's
subtree must swap its mix base to `var(--main-text-color)` — the block's own
background is no longer `--block-bg-solid-color`, so mixing against it produces the
wrong result. Two call sites in `_document-nodes.scss`:

- The startup hover-expanded deeper background (`&--expanded.agent-user-message--startup`):
  `color-mix(in oklab, var(--user-input-color) 38%, var(--block-bg-solid-color))` →
  `color-mix(in oklab, var(--user-input-color) 38%, var(--main-text-color))`.
- The pin/unpin button hover background:
  `color-mix(in oklab, var(--user-input-color) 35%, var(--block-bg-solid-color))` →
  `color-mix(in oklab, var(--user-input-color) 35%, var(--main-text-color))`.

`.agent-user-message-icon` / `-label` / `-pin` / `-unpin` keep `color: var(--user-input-color)`
directly (not inherited), so they're unaffected by the body's new inherited text
color — amber-on-near-white and amber-on-near-black are both already
established-legible combinations in this codebase (the pre-inversion background was
already a light-on-dark-or-dark-on-light mix toward one of these same two tokens).

### 2. Answered questions render via the user-message treatment

Add a field that survives the answer, holding the real (un-flattened) text:

`frontend/app/view/agent/types.ts` — `ToolNode`:

```ts
/** Set alongside `summary` when an AskUserQuestion is answered
 *  (status flips to "success"). Raw answer text with real newlines
 *  preserved — `summary` is the decorated one-line log form and is
 *  not fit for display as message content. Absent on nodes answered
 *  before this field existed (legacy transcripts) — callers must
 *  guard on presence, not just tool+status, and fall back to the
 *  generic collapsed-row rendering. */
answerText?: string;
```

`frontend/app/view/agent/hooks/useAgentQuestions.ts` (`handleAnswer`) — set it
alongside `summary`, using `outcome.answer_text` directly (real newlines intact,
not the `; `-flattened `flatText` used for the log-line `summary`):

```ts
updated.push({
    ...n,
    status: "success",
    question: undefined,
    summary,
    answerText: outcome.answer_text,
});
```

`frontend/app/view/agent/components/ToolBlock.tsx` — branch before the generic
collapsed-row rendering:

```tsx
const isAnsweredQuestion = () =>
    props.node.toolName === "AskUserQuestion" &&
    props.node.status === "success" &&
    props.node.answerText != null;
```

When true, render a small presentational component (`AnsweredQuestionMessage`,
new file `frontend/app/view/agent/components/AnsweredQuestionMessage.tsx`, or
inlined in `ToolBlock.tsx` if it stays this small) reusing the exact
`.agent-user-message` / `.agent-user-message-content` DOM shape and CSS classes
`UserMessageBlock` uses for regular (non-startup) input — always expanded,
`<pre><LinkifiedText text={props.node.answerText} /></pre>`, `white-space: pre-wrap`.
No tool icon, no collapse/pin/peek chrome, no `ToolElapsedTicker` — those are
tool-call affordances that don't apply once this is just displayed as a message.

For the auto-timeout cases (`outcome.autoFilledCount > 0`), prepend a single small
muted meta line above the `<pre>` (styled like `.agent-user-message-hint`) —
`⏱️ auto-answered (no response in 30s)` or `⏱️ partly auto-answered (n/m)` — so the
transcript doesn't silently misrepresent a timeout fallback as something the user
actually typed. This is the one piece of the original decorated `summary` string
worth preserving visually; the rest (icon, "Answered —" prefix) is redundant once
the block itself unmistakably reads as user input.

`status` values other than `success` (`denied`, `canceled`, `failed` —
e.g. the pane closed before an answer arrived) keep the existing generic
`ToolBlock` rendering: there's no real answer text to show as a message, and the
`awaiting_answer` → terminal-without-answer transition is a different, legitimate
"this didn't get answered" state that should keep looking like a tool outcome.

**Larger text (added 2026-08-17, post-implementation feedback).** This was the
*original* ask behind this feature before it grew the inverted-surface half — the
answer shouldn't just match ordinary user-input styling, it should read larger
than everything else in the pane. `AnsweredQuestionMessage`'s root gets a second
class, `agent-user-message--answered-question`, and `_document-nodes.scss` scopes
a `font-size: 1.25em` / `line-height: 1.4` bump to that variant's `<pre>` only —
em-relative so it scales with the pane's own zoom (`--termfontsize`) rather than
fighting it, and scoped narrowly so regular typed user input keeps its normal
size (this spec never asked to enlarge *that*).

---

## Edge cases

- **Legacy transcripts.** Nodes persisted before this ships have `toolName ===
  "AskUserQuestion"`, `status === "success"`, but no `answerText`. The guard above
  requires `answerText != null`, so these fall through to the existing generic
  rendering unchanged — no broken/empty message bubbles on old history.
- **Multi-select / multi-question answers.** `outcome.answer_text` already joins
  multiple answers with real `\n`; rendered through `<pre>` with
  `white-space: pre-wrap` (matching regular non-startup user input's wrap
  behavior), this reads as a multi-line message the way a pasted multi-line reply
  would, rather than the semicolon-joined single line used in `summary`/logs.
  the AgentQuestionPanel's own colors and layout are untouched (multi-select
  checkboxes, countdown chip, etc.) — this spec only changes what happens *after*
  an answer is recorded.
- **Link/URL contrast on the inverted surface.** Resolved during implementation
  (reagent P1 on PR #2630): `LinkifiedText`'s links were still colored via the
  ambient `--link-color`, which is tuned for contrast against the *normal*
  background and isn't guaranteed to read once inverted (e.g. dracula's light
  cyan link on the new near-white `.agent-user-message` background, or
  high-contrast's bright cyan on white). Fixed by setting
  `a.linkified-url { color: currentColor; }` scoped to `.agent-user-message-content`
  (and its Portal-rendered peek-overlay copy) — `currentColor` resolves to the
  block's own `--block-bg-solid-color` text color, which is guaranteed to
  contrast against `--main-text-color` per theme, the same guarantee the base
  surface inversion already relies on. No new per-theme token needed.

---

## Tests

- `useAgentQuestions.test.ts` — extend the existing `handleAnswer` cases (manual,
  full-auto, partial-auto) to assert `answerText` equals the raw
  `outcome.answer_text` (newlines intact, not `; `-joined), independently of
  `summary`.
- `ToolBlock` (or a new `AnsweredQuestionMessage.test.tsx`) — a node with
  `toolName: "AskUserQuestion"`, `status: "success"`, `answerText` set renders
  `.agent-user-message` (not `.agent-tool-summary`); the same node with
  `answerText` undefined (legacy case) still renders the generic collapsed row.
- Manual/visual: sample `.agent-user-message` across `default` (dark), `light`,
  `high-contrast`, and one accent-heavy theme (`catppuccin-latte` or `dracula`) to
  confirm the inversion reads correctly and the amber border/icon/label stay
  legible on both polarities.

---

## Order of delivery

1. **Color inversion** (`_document-nodes.scss` only) — self-contained, no data
   model change, benefits regular user input immediately regardless of phase 2's
   timing.
2. **Answered-question-as-user-message** — needs the new `answerText` field, the
   `handleAnswer` change, and the new `ToolBlock` branch/component. Ships after (or
   alongside) phase 1 so the new rendering path picks up the inverted surface from
   day one instead of needing a follow-up restyle.

---

## Out of scope

- The *pending* `AgentQuestionPanel` (before an answer is given) keeps its own
  existing accent styling — this spec covers only the post-answer, in-history
  state, and regular typed user input.
- No change to the auto-timeout mechanics themselves
  (`SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md`,
  `SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md`) — only how the
  already-computed outcome is displayed once resolved.
- No change to `summary` or anything that reads it outside the pane (transcript
  export, muxbus surfaces, control-protocol logs) — `answerText` is a
  frontend-only, additive field.

---

## Related

- `docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md` — original `AskUserQuestion`
  panel.
- `docs/specs/SPEC_ASK_USER_QUESTION_AUTO_TIMEOUT_2026_08_06.md` — 30s countdown.
- `docs/specs/SPEC_ASK_USER_QUESTION_TIMEOUT_HOVER_PAUSE_2026_08_10.md` — hover-pause.
- `docs/specs/SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md` —
  introduced `--user-input-color` and the current `.agent-user-message` styling
  this spec revises.
