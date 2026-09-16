# Retro: repeated punctuation renders as blanks under CEF 152

**Date:** 2026-09-15
**Status:** implemented — root-caused and fixed by removing the bundled
JetBrains Mono, the only font we shipped with the offending `calt` rules;
follow-up font-system cleanup tracked separately.
**Severity:** Medium — highly visible in every text surface, but **not data
loss**: the underlying value always kept every character, which is why
messages reached agents complete.
**Affects:** CEF 152 (shipped in v0.56.0, 2026-09-15). CEF 148 is clean.
**Platforms:** Windows and Ubuntu both.

---

## TL;DR

Holding down `.` (or `;` `:` `?` `!` `#` `&` `+` `_` `=` `-` `|`) rendered a
run of blank cells with a single glyph at the end — `"            ."` —
while the actual text kept every character.

JetBrains Mono implements programming ligatures the way monospace fonts must:
to ligate `..` without breaking the character grid, it substitutes the first
period with **`glyph00388`, a glyph with no contours** (invisible, still one
cell wide) and the second with a composed glyph whose outline extends
*leftward* to draw both dots. Net width unchanged, one symbol drawn.

The font explicitly guards this so it only applies to runs of 2–3 (see
"Why CEF 148 was fine"). **CEF 152 stopped honoring those guards**, so the
substitution ran across an entire run: many blanks, one composed glyph at the
end that can only draw 2–3 dots.

**Fix:** `font-variant-ligatures: none` + `font-feature-settings: "calt" 0`,
declared *outside* `@layer base` in `frontend/app/reset.scss`.

---

## Root cause, in detail

Verified directly against `public/fonts/jetbrains-mono-v13-latin-regular.woff2`
with fontTools + HarfBuzz — no DevTools required:

```
period       advance=600  bounds=(217, -10, 383, 156)   <- a real dot
glyph00388   advance=600  bounds=None                   <- NO CONTOURS (blank)
glyph00194   advance=600  bounds=(-301, -10, 303, 156)  <- composed 2-dot, draws leftward
glyph00195   advance=600  bounds=(-881, -10, 283, 156)  <- composed 3-dot
```

`calt` GSUB lookups map `period -> glyph00388` when a period is followed by
another period. `calt` is **on by default** in CSS, so it must be disabled
explicitly.

### Why CEF 148 was fine

The font deliberately prevents this on long runs. OpenType contextual rules
are tried in order and the **first match wins**, so a font blocks a
substitution by putting a no-op rule ahead of it. JetBrains Mono does exactly
that:

```
lookup 59  (the ".." ligature)
  rule 0: back=.  input=.  ahead=.    -> NO-OP GUARD   "mid-run: do nothing"
  rule 1:         input=.  ahead=..   -> NO-OP GUARD   "longer run ahead: do nothing"
  rule 2:         input=.  ahead=.<   -> NO-OP GUARD
  rule 3: back=blank input=.          -> SUBSTITUTES (composed form)
  rule 4:         input=.  ahead=.    -> SUBSTITUTES (-> blank)
```

Rules 0–1 exist precisely so `..` and `...` ligate while `....` and longer
stay literal. CEF 148 honored them; CEF 152 does not. **The font is correct
and unchanged — the shaper regressed.** This machine's own HarfBuzz also
honors the guards, which is why the bug could not be reproduced offline.

This is therefore an **upstream Chromium/HarfBuzz regression**, not an
AgentMux bug, and it is not specific to JetBrains Mono: the guard-rule
technique is standard, so Fira Code, Cascadia Code and Iosevka are affected
the same way. Worth an upstream report; minimal repro is a ligature font and
a held-down `.` key.

### The evidence that settled it

Which characters break was **predicted from the font tables before most were
tested** — "breaks" = has a `calt` rule mapping to `glyph00388`:

- **Predicted broken, all confirmed:** `.` `;` `:` `?` `!` `#` `&` `+` `_`
  `=` `-` `|`
- **Predicted working, all confirmed:** `,` `@` `$` `%` `^` `(` `)` `\` `{`
  `}`

**22 of 23 correct.** The comma is decisive: it is the only sentence
punctuation with no blank-glyph rule, and the only one that renders. The one
miss, `*`, ligates at *exactly* 3 while runs of 2/4/6 pass through — a
shallower chain, so holding the key never triggers it.

The pattern in one line: **every broken character is half of a
programming-operator ligature** (`..` `::` `;;` `??` `!!` `##` `&&` `++` `__`
`==` `--` `||`). Nothing outside that set breaks.

---

## What was ruled out (and why it took so long)

Four hypotheses died before the right one. Each was eliminated by real
evidence, but all four were reached by reasoning ahead of observation:

| Hypothesis | Killed by |
|---|---|
| Terminal predictive local echo × PTY output coalescing (#1223 × #3206) | Disabling `term:predictiveecho` changed nothing — and the bug wasn't in a terminal at all |
| GPU/ANGLE compositing regression (this upgrade needed two ANGLE fixes, #3172/#3229) | `--disable-gpu` changed nothing |
| `field-sizing: content` on the composer | Bug also hit the editor, filter bar and swarm field, none of which use it |
| Bundled fonts failing to load under 152 | Host log shows all 8 `FontFace` loads succeeding, zero failures |

**The real cost was symptom ambiguity, not any single wrong theory.** The
first report read as a *terminal* bug; it was actually the agent composer,
then the editor, filter bar, swarm field and browser address bar. Several
cycles were spent investigating terminal-only machinery (predictive echo, PTY
batching) that shares no code with the affected surfaces.

**Lessons:**

1. **Confirm which UI surface reproduces a bug before investigating its
   subsystem.** A terminal pane and a chat textarea both look like "a text
   box" to a user and share essentially no code.
2. **Ask what else differs.** "Google's search box is fine but the address bar
   above it isn't" — one sentence from the repo owner — eliminated Chromium's
   general text layout and pointed straight at the font. That question should
   have been asked on day one.
3. **Check what the app already logs.** `fontutil.ts` logs every font load;
   nobody looked until four hypotheses in.
4. **Prefer offline analysis over UI fiddling.** The whole root cause was
   settled with fontTools/HarfBuzz against the font file — no DevTools, no
   rebuild, and it produced a *falsifiable character-level prediction* rather
   than another plausible story.
5. **A failed fix is not a failed theory.** The first `calt` fix appeared to
   disprove the hypothesis; it had actually been neutralized by the cascade
   (see below). Verify the fix *applied* before concluding it didn't work.

### Cascade trap worth remembering

`reset.scss` is wrapped in `@layer base`, and it contains:

```scss
input, button, textarea, select { font: inherit; }
```

The `font` **shorthand resets the font-variant/font-feature longhands**, and
layered rules lose to unlayered ones regardless of order. Any future
font-feature rule must be declared **outside the layer** (as the fix is) or it
will silently do nothing on exactly the form controls that matter.

---

## Follow-ups

1. **Font-system cleanup (PR 2).** This investigation surfaced a much larger
   mess: **53 of ~108 font-variable usages reference variables that are never
   defined** — `--termfontfamily` (26 uses), `--monospace-font` (10),
   `--mono-font` (7), `--mono-font-family` (3), `--monofontfamily` (2),
   `--termfontsize` (2), `--main-font-family`, `--font-family`,
   `--default-font-feature-settings`. Seven different names for "the mono
   font"; every use silently falls back, and the fallbacks disagree (14 fall
   back to bare `monospace`, 11 to JetBrains Mono). `--font-mono`/`--font-sans`
   are each defined **twice** with different values (`theme.scss` vs
   `tailwindsetup.css`). Consequence: `term:fontfamily` works in xterm
   (`term.tsx:193` reads it in JS) but does nothing for the 26 CSS rules that
   expect it. Planned: consolidate to one `--font-mono`/`--font-sans`,
   VS Code–style platform stack, wire the settings for real, and **drop the
   bundled JetBrains Mono** (nothing deliberately selects it — it only appears
   as a fallback inside dangling vars), which removes this bug at the source.
2. **Ligature opt-in setting**, mirroring VS Code's `editor.fontLigatures`
   (default `false`, accepts a raw `font-feature-settings` string).
3. **Upstream report** of the guard-rule regression to CEF/Chromium.
4. **DevTools exiting on its own** was observed during this session on CEF
   152, unexplained and unrelated. Tracked separately.

## Fix: remove the font, not the feature

**Shipped fix — delete the bundled JetBrains Mono** (3 woff2 files + its
loader in `frontend/util/fontutil.ts`). It was the only font we shipped
carrying these `calt` rules; Hack, our actual mono default, has no `calt`
table at all. With it unloaded, the CSS rules that merely *name* it in a
fallback list fall through to system `monospace` (Consolas / DejaVu Sans
Mono), neither of which has programming ligatures.

Nothing ever selected JetBrains Mono deliberately — it appeared **only**
inside the fallback of `var(--termfontfamily, …)`, a variable that is never
defined (see Follow-ups). So removing it costs nothing that was intended.

### Why not the CSS fix (`font-variant-ligatures: none` / `"calt" 0`)

That was tried first and **abandoned as structurally unsound.** The `font`
shorthand resets both longhands, and the app has **19 `font:` shorthand
declarations**; each one silently re-enables `calt` on its element at a
specificity the override cannot beat:

- The first attempt (`html, body, input, …`) was defeated on form controls by
  `@layer base`'s own `input,button,textarea,select { font: inherit }`.
- The second (`*, *::before, *::after` + a `body` re-declaration) was defeated
  by `.pane-tab-rename-input { font: inherit }`, `.block-frame-view-type--input`,
  and `.agent-user-message-summary` — all `.class` (0,1,0), all beating `*`
  (0,0,0). Found by review (codex P1, reagent P1/P2×2), not by testing.

Winning that fight needs `!important` on a universal selector, or edits to all
19 sites plus vigilance forever. Deleting one font file is smaller, total, and
leaves nothing to regress. **Prefer removing the input to a bug over
suppressing the bug's output.**

A CSS-level ligature control still belongs in the product — as the opt-in
setting in Follow-ups, built once the `font:` shorthands are gone.
