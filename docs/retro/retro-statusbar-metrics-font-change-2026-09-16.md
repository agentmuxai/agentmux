# Retro: status-bar performance metrics silently rendered in the wrong font for a long time — the recent "font upgrade" just fixed it

**Date:** 2026-09-16
**Status:** implemented — not a bug in the font-system PRs themselves (they're
correct); root-caused here, and the pre-existing look was then restored on
purpose in `StatusBar.scss` per the Outcome section below.
**Severity:** Cosmetic. No data or functionality was ever affected.
**Affects:** `.stat-mono` in `frontend/app/statusbar/StatusBar.scss` — the CPU,
GPU, memory, commit/pagefile, disk, network, and uptime readouts in the status
bar (`SystemStats.tsx`, `BackendStatus.tsx`, `GpuStatus.tsx`).

---

## TL;DR

Pulling latest `main` brought in two font-system PRs from 2026-09-16:
[#3247](https://github.com/agentmuxai/agentmux) (drop bundled JetBrains Mono —
its `calt` ligatures blanked repeated punctuation on CEF 152) and
[#3254](https://github.com/agentmuxai/agentmux) (consolidate the font
variable system onto two canonical tokens, `--font-sans` / `--font-mono`).

The status bar's performance metrics visibly changed font as a result — but
**not because a font was swapped out**. `.stat-mono`'s `font-family`
declaration was **invalid CSS and had never applied** until #3254 fixed it.
The "old font" was never a monospace font at all: it was `Inter`, the same
proportional UI font as every other label in the app, quietly inherited
because the intended declaration was silently dropped by the browser. The
"new font" is the first time this readout has ever actually rendered in a
monospace face — `Hack Nerd Font Mono`, the app's bundled webfont, with
system `Consolas` only as its fallback if Hack fails to load (it doesn't;
verified below). Hack's plain, humanist letterforms read as "Consolas-like"
next to nothing-in-particular before, which is what prompted this
investigation.

---

## What actually changed, line by line

`frontend/app/statusbar/StatusBar.scss`, PR #3254:

```diff
     .stat-mono {
-        font-family: var(--fixed-font);
+        font-family: var(--font-mono);
         font-size: 11px;
         display: inline-block;
     }
```

`--fixed-font` (`frontend/app/theme.scss`, pre-#3254) was never a valid value
for `font-family`:

```scss
--fixed-font: normal 12px / normal "Hack", monospace;
```

That's a `font` **shorthand** (style / size / line-height / family all in one
string) — valid for the `font` property, not for `font-family`. Per the CSS
spec, assigning a shorthand's serialized form to a longhand property makes
the whole declaration invalid, and an invalid declaration is dropped from the
cascade entirely — it does not fall back to some default, it just isn't
there. `font-family` is inherited, so the computed value fell through to the
nearest ancestor that actually set one.

That ancestor is `body`:

```scss
// frontend/app/app.scss
body {
    ...
    font: var(--base-font);   // --base-font: normal 14px / normal "Inter", sans-serif;
    ...
}
```

Nothing between `body` and `.status-bar-item .stat-mono` sets its own
`font-family` (confirmed — no rule in `StatusBar.scss` above line 101 touches
it). So every CPU/GPU/RAM/disk/network number in the status bar has been
rendering in **Inter**, the ordinary proportional UI sans-serif, since
whenever this rule was written — not a monospace font, despite the class
being named `stat-mono` and clearly intending one.

This wasn't a one-off. PR #3254's own commit message independently found the
same shape 44 times across the codebase (`font-family: var(--fixed-font)`)
and 53 more using variables that are never defined anywhere
(`--termfontfamily`, `--monospace-font`, `--mono-font`, etc.) — a systemic
issue, not specific to the status bar. `.stat-mono` was simply one instance.

## Why it now looks like Consolas

`--font-mono` (defined once, `frontend/tailwindsetup.css`) is:

```css
--font-mono: "Hack", Consolas, Menlo, monospace;
```

`"Hack"` here refers to the bundled Hack Nerd Font Mono webfont
(`public/fonts/hacknerdmono-{regular,bold,italic,bolditalic}.woff2`),
registered globally via the JS `FontFace` API in
`frontend/util/fontutil.ts`'s `loadHackNerdFont()`, called once at startup
from `app-init.ts`. `Consolas`/`Menlo`/`monospace` are only reached if that
webfont fails to load.

Checked directly against the just-built portable
(`agentmux-0.56.1+g3cb775fc3.20260916T144709.798-x64-portable`) — the Hack
woff2 files are present under `runtime/frontend/fonts/`, so the asset
pipeline shipped them correctly and there's no missing-file reason for
Consolas to be what's actually on screen. The font genuinely rendering is
Hack Nerd Font Mono; it's just a plain, humanist-style monospace face (Hack
is a DejaVu Sans Mono derivative) with none of JetBrains Mono's distinctive
slashed zero or ligature glyphs, so at 11px in a status bar it reads as "some
generic system monospace, like Consolas" to the eye — which is exactly the
report that kicked off this investigation.

## Why JetBrains Mono isn't the answer either

It might seem like "the old font" should have been JetBrains Mono, since
that's what got removed in #3247 and is the more recognizable code font.
It wasn't, for `.stat-mono` specifically: JetBrains Mono only ever appeared
inside the fallback list of variables that were never defined anywhere in
the codebase (`--termfontfamily` and friends) — not `--fixed-font`, and not
anything `.stat-mono` referenced. `.stat-mono` was broken in a different way
(invalid property value, not an undefined variable) that happened to land on
the *same* visual result — the browser's own default UI font stack — as the
undefined-variable cases did, just by a different mechanism.

## Verification

- `git log -p -- frontend/app/statusbar/StatusBar.scss` — confirms the exact
  `--fixed-font` → `--font-mono` diff in PR #3254 (commit `4cd20749d`).
- `git show 257d7ee41^:frontend/app/theme.scss` — confirms `--fixed-font`'s
  pre-fix value was a `font` shorthand, and `--base-font` (what `body`
  actually used) was `"Inter", sans-serif`.
- No `@font-face` rule for `"Hack"` exists anywhere in the SCSS/CSS — it's
  loaded exclusively via `fontutil.ts`'s `FontFace` API, confirmed present
  and wired to `app-init.ts:630`.
- `public/fonts/hacknerdmono-*.woff2` present in source and confirmed copied
  into the shipped portable build's `runtime/frontend/fonts/`.

## Outcome

No fix needed for #3254 itself — it's correct, and made `.stat-mono` apply a
real, valid `font-family` for the first time. Once that was understood, the
preference was to keep the *pre-existing look* (Inter, the ordinary UI sans)
rather than switch to the newly-working monospace rendering: `.stat-mono` in
`StatusBar.scss` now reads `font-family: var(--font-sans)` explicitly, so the
status bar's CPU/GPU/RAM/disk/net/uptime numbers keep rendering in Inter —
same as before this pull — but on purpose this time, not by way of an
invalid-CSS accident. The comment above the rule points back here so a future
reader doesn't mistake it for another instance of the `--fixed-font` bug and
"fix" it back to `--font-mono`.
