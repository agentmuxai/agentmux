# SPEC: `AgentWorkingRow` typography refresh — drop the accent-color text, match the thinking-text font, go bold

**Date:** 2026-09-03
**Status:** Proposed. No code changed by this document.
**Component:** `AgentWorkingRow` (`frontend/app/view/agent/components/AgentFooter.tsx:146-347`)
**Styles:** `.agent-working-row-anchor .agent-working-row` (`frontend/app/view/agent/styles/_control-bar.scss:104-276`)
**Verified against:** `main` @ `ee327f631` (2026-09-03 pull)

---

## 0. Request, as given

> in agent pane lets refine the "Working..." bar .. we dont want it blue, we
> want it bold and in the same font as the normal thinking text of the same
> font as the normal agent thinking text.

Three asks, deduplicating the repeated clause:

1. Stop the row's text reading as blue.
2. Make it bold.
3. Use the same font as the normal agent "thinking" text.

---

## 1. What "the Working... bar" is, precisely

The component is `AgentWorkingRow` (`AgentFooter.tsx:146`), rendered as a
normal-flow sibling directly above the composer
(`SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md` moved it there from an
absolutely-positioned overlay). It has two mutually exclusive states, gated by
`props.loading`:

- **`.agent-working-row--loading`** — spinner dot + cycling left-zone text
  (`"Working…"`, a tool name, `"Compacting…"`, etc. — see `loadingLeftText()`,
  `AgentFooter.tsx:127-144`) + elapsed time/tokens on the right. **This is the
  one the request is about** — it's the only state that reads blue.
- **`.agent-working-row--worked`** — the post-completion `"✓ Worked · Ns"`
  summary. Already styled with `color: var(--secondary-text-color)`
  (`_control-bar.scss:220`) — not blue, not in scope for this change, and the
  spec below does not touch it.

---

## 2. Current state — three separate rules produce the reported look

All confirmed by direct read of `_control-bar.scss` on `main`, not inferred.

**2.1 — Text color is `--accent-color`.**

```scss
// _control-bar.scss:119-120
&--loading {
    color: var(--accent-color);
```

`--accent-color` is theme-adaptive by design (`theme.scss:63`,
`--accent-color: rgb(65, 159, 224)` under the default theme — a blue) — it is
not a hardcoded hex the way the request's phrasing might suggest. But because
color is inherited and nothing downstream overrides it, this one line is what
makes the spinner-adjacent text, AND the right-zone elapsed/token readout,
read as the theme's accent hue end to end. Confirmed this is the sole color
source: no descendant selector between here and the rendered text
re-declares `color`.

**2.2 — The shimmer highlight is ALSO accent-colored.**

```scss
// _control-bar.scss:159-172
&.agent-working-shimmer {
    background-image: linear-gradient(
        90deg,
        var(--secondary-text-color) 0%,
        var(--secondary-text-color) 40%,
        var(--accent-color) 50%,        // ← the sweeping highlight band
        var(--secondary-text-color) 60%,
        var(--secondary-text-color) 100%
    );
    ...
    animation: agent-working-shimmer-sweep 2.4s ease-in-out infinite alternate;
}
```

The left-zone text (`"Working…"`, tool name, etc.) is ALWAYS rendered with
this shimmer class while loading (`AgentFooter.tsx:330`) — it's not an
occasional accent, it's the base rendering of the left zone. A fix that only
touches §2.1 leaves the sweeping highlight still visibly blue.

**2.3 — Font is the fixed/monospace font, not the thinking-text font.**

```scss
// _control-bar.scss:108-110
.agent-working-row-anchor .agent-working-row {
    font-size: 10px;
    font-family: var(--fixed-font, monospace);
```

`--fixed-font` (`theme.scss:62`) is `normal 12px / normal "Hack", monospace`
— the terminal/code font, used elsewhere in this same file for genuinely
monospace content (tool summaries, live-tail output — e.g.
`_document-nodes.scss:125,295,377,505...`). The working row uses it too, but
nothing about a status sentence like "Working…" is code-shaped.

**2.4 — What "the normal thinking text" font actually is.**

Traced through the render path: a `thinking` node renders as
`.agent-markdown-block.thinking-block` (`_document-nodes.scss:92-100`), which
sets `opacity`, `font-style: italic`, `color`, and border/background — but
**does not** set `font-family` or `font-size`. Those are inherited from the
`Markdown` component it wraps (`frontend/app/element/markdown.tsx`, styled by
`frontend/app/element/markdown.scss:19-34`):

```scss
// markdown.scss:26-34
.content {
    line-height: 1.5;
    color: var(--main-text-color);
    font-family: var(--markdown-font-family);
    font-size: var(--markdown-font-size);
```

```scss
// theme.scss:85-88
--markdown-font-family: -apple-system, BlinkMacSystemFont, "Segoe UI",
    "Noto Sans", Helvetica, Arial, sans-serif, "Apple Color Emoji",
    "Segoe UI Emoji";
--markdown-font-size: 14px;
```

So "the normal agent thinking text" is a **sans-serif UI font stack**
(`--markdown-font-family`), not `--fixed-font`'s monospace. No font-weight is
set anywhere in that chain — thinking text renders at normal/400 weight,
which is consistent with asking for *this* row specifically to go bold as a
distinguishing change, not a match.

---

## 3. Proposed change

Three edits, all confined to `.agent-working-row-anchor .agent-working-row`
(the base rule, `_control-bar.scss:108`) and its `&--loading .agent-working-shimmer`
child (`:159-172`). No component/TSX change — this is styling only.

```scss
.agent-working-row-anchor .agent-working-row {
    font-size: 10px;                         // unchanged — see §4.1
    font-family: var(--markdown-font-family); // was: var(--fixed-font, monospace)
    font-weight: 700;                         // new
    ...

    &--loading {
        color: var(--main-text-color);        // was: var(--accent-color)
        ...
        .agent-working-row-left.agent-working-shimmer {
            background-image: linear-gradient(
                90deg,
                var(--secondary-text-color) 0%,
                var(--secondary-text-color) 40%,
                var(--main-text-color) 50%,    // was: var(--accent-color)
                var(--secondary-text-color) 60%,
                var(--secondary-text-color) 100%
            );
        }
    }
}
```

Rationale for the two color targets landing on `--main-text-color` rather
than, say, `--secondary-text-color`: the shimmer gradient's OTHER four stops
are already `--secondary-text-color` (§2.2) — the highlight band needs to
read as visibly *brighter* than its own resting stops, or the sweep animation
has nothing left to sweep. `--main-text-color` is the brightest general-purpose
text token available and is what `--secondary-text-color` is already defined
relative to elsewhere in this file, so it preserves the existing
dim-to-bright sweep shape while removing the accent hue.

---

## 4. Open questions — need a decision before implementation, not guessed here

**4.1 — Font SIZE: match too, or keep the row's existing 10px?**

The request says "same font," which is most literally about the typeface
(§2.4's `--markdown-font-family`), not necessarily `--markdown-font-size`
(14px). This spec's proposal (§3) keeps `font-size: 10px` and changes only
`font-family` — a 14px status line stacked directly above the composer would
be noticeably larger than every other control-bar element around it
(`.agent-control-bar` itself is `font-size: 11px`, `_control-bar.scss:287`).
**Recommend keeping 10px.** Flagging explicitly in case "same font" was meant
to include size.

**4.2 — Does the spinner dot (`.agent-spinner-dot`) also lose its accent color?**

```scss
// _control-bar.scss:301-311
.agent-spinner-dot {
    background: var(--accent-color);
    box-shadow: 0 0 6px color-mix(in srgb, var(--accent-color) 60%, transparent);
```

This is a `background`, not `color` — untouched by §3's `color:` edit, and
not literally "the bar['s]" text. A small colored pulse dot next to
otherwise-neutral text is also a fairly standard "still alive" affordance
(distinct signal color for the one genuinely animated glyph). **Recommend
leaving it accent-colored** unless the intent is "nothing in this row should
carry the theme accent at all," in which case it should move to
`--main-text-color` or `--secondary-text-color` alongside §3's other two
edits, and its `box-shadow` halo would need the same substitution.

**4.3 — `--fixed-font` includes a font-weight-relevant shorthand; confirm no regression.**

`--fixed-font` and `--base-font` are both CSS shorthand `font:` properties
(`normal 12px / normal "Hack", monospace` — the leading `normal normal` is
`font-style font-variant`, not weight, per the shorthand's own ordering) —
but `_control-bar.scss:108-110` only ever consumed the `font-family` /
`font-size` longhand pieces of `--fixed-font`, never the shorthand itself, so
switching to `var(--markdown-font-family)` (family-only, not a shorthand) is
a like-for-like swap. Confirmed no other longhand (`font-style`,
`line-height`) was implicitly riding along that needs restating.

---

## 5. Non-goals

- No change to `.agent-working-row--worked` (§1) — already non-accent.
- No change to layout, the type-out reveal mechanic, or the shimmer's timing
  — only its color stop.
- No change to `--accent-color`, `--fixed-font`, or any other token's
  *definition* — this scopes to `AgentWorkingRow`'s own rule, not a
  site-wide accent-color audit.
- Does not touch `AgentComposerStrip.tsx`'s compacting/reconnecting-adjacent
  code — those states were relocated INTO `AgentWorkingRow` already
  (`AgentFooter.tsx:97-109`'s prop doc comments); nothing left to change
  there for this request.

---

## 6. Verification plan (for the implementation PR, not this doc)

- Visual: trigger a live turn, confirm the left-zone phrase/tool text and the
  right-zone elapsed/token text are no longer accent-hued, confirm bold, and
  confirm the shimmer still visibly sweeps (brighter band moving across
  dimmer text) without regressing the reduced-motion fallback
  (`_control-bar.scss:172-182`'s comment on why it can't just be disabled).
- Existing tests to keep green: `AgentFooter.test.tsx` doesn't assert on
  inline color/font styles today (component-behavior tests, not visual
  regression) — expect no test changes needed, but run
  `npx vitest run frontend/app/view/agent/components/AgentFooter.test.tsx`
  to confirm.
- No `tsc`/build-affecting change — SCSS-only.
