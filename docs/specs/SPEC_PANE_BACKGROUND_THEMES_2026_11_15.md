# Spec: Pane backdrops: a background that brings its own readable foreground

Status: Approved by the operator on 2026-11-15 (decisions D1 to D12 as recommended). Task #47695 (spec), build #47696. Author: Agent1@narko, 2026-11-15.
Read against: agentmuxai/agentmux `origin/main` at `09cb71400`. Engine: CEF crate `156.4.0+156.0.26` (Chromium 156), from `Cargo.lock`.

## 0. The operator's words

> "we also want a seperate pane background system. background colorss for panes allow the user to select a color for the bakground, but it would be more akin to a color theme, since a background change necessarily requires a robust change to the other colors (forground text). so if we have 16 background colors, we'd need the proper colors that set the proper contrast on the foreground. lets spec this, and think of good names for this feature. we also want it flexible, for example, being able to use an image as the background, and for the foreground elements to automatically invert over the background so users can see them. we also want to research latest best practices, lets put all ideas together, write spec to file"

> "the whole framework would need to be theme aware too"

My reading of the second line is in section 7.1 and in decision D2; the spec holds under either reading.

## 1. The idea in one page

A pane's background today is one colour variable, and everything drawn on it uses the app-wide text, border and status colours. Change the background and some of those become unreadable. So a background cannot be a colour. It has to be a small theme scoped to one pane.

**A backdrop** is that theme. It is one thing the user picks (a built-in colour, a custom colour, or an image), and picking it sets, for that pane only:

- the surface: the pane body, its raised surfaces (cards, inputs, menus the pane opens) and its header;
- the ink: every foreground token that sits on the surface: text, muted text, links, borders and dividers, icons, hover and selected washes, focus ring, selection, scrollbars;
- the signals: error, warning, success and info colours, tone-adjusted to read on this surface;
- `color-scheme` (light or dark) for native controls inside the pane.

Everything inside the pane already reads these tokens (section 7), so nothing inside has to know a backdrop exists. The pane's header identity colour (`frame:color`, the 16-slot palette) stays separate: it says whose pane this is; the backdrop says what is behind it.

Three rules hold the feature together:

1. **Contrast is a build output, never a hope.** Every backdrop's ink is computed from its surface by one function and checked by a test that fails CI below the targets in section 3.
2. **One ink per pane.** Text does not change colour as the user scrolls over a busy image. The surface is made safe for one ink (a scrim, section 5), not the ink made to chase the image.
3. **The app theme still rules the chrome.** Light and dark app themes keep working; a backdrop's polarity follows the app theme by default (section 8).

## 2. Names

The word "theme" is taken (the app theme, the terminal theme, a widget's theme). Candidates for the feature, and for one unit of it:

| Feature | One unit | What it implies | Risk |
|---|---|---|---|
| **Backdrops** | a backdrop | what is behind the pane's content; works for a colour or an image | none found; "background" and "backdrop" are close, but the feature name is distinct in menus |
| Surfaces | a surface | the material the content sits on | clashes with "surface" as a token name (`--elevated-surface`); used inside the spec already |
| Scenes | a scene | an image or mood | wrong for a plain colour; implies animation |
| Moods | a mood | feeling, not a measurable pair | vague; hides that contrast is the point |
| Washes | a wash | a tint over something | wrong for images |
| Palettes | a palette | colour sets | taken: the 16-slot pane palette |

**Recommendation: Backdrops.** The pane menu entry is "Backdrop…", the setting is `pane:backdrop`, the meta key is `backdrop`. The foreground half of a backdrop is called its **ink** in the spec and in code (`--ink-*`), which keeps "text colour" from meaning three things. It is never called a theme in the UI.

## 3. The built-in set and how its ink is made

### 3.1 Hand-tuned pairs or derived by rule

**Derived by rule, from a seed, with a small table of hand overrides.** A hand-tuned pair per backdrop cannot cover custom colours and images, and 16 backdrops times two app themes times 20 tokens is 640 values nobody can keep correct. One function, `deriveInk(surface, polarity, targets)`, serves built-ins, custom colours and image scrims; the override table exists only for a backdrop whose derived result a human dislikes (kept as data next to the seed, each with a reason).

Colour space: **OKLCH**. Lightness in OKLCH is perceptual, so "move the ink 0.04 lighter" means the same thing at every hue, and the repo already has OKLCH helpers (`frontend/app/theme/oklch.ts`). Contrast is measured with the WCAG 2.x ratio on sRGB relative luminance (section 12): that is the standard we are held to, and the derivation only searches in OKLCH for a colour that passes it. The existing `contrastRatio` in `frontend/app/block/pane-color-scheme.ts` is reused.

### 3.2 The 16

The 16 built-ins are the 16 hues of the existing pane palette (`frontend/schema/color-palette.json`, `frontend/app/theme/palette.ts`), so a user who coloured a pane's header sees the same family names in the backdrop picker. Each is a seed (hue, low chroma) plus a tone per polarity:

- **Deep tone** (for a dark app theme): surface OKLCH L about 0.20 to 0.26, chroma 0.03 to 0.07.
- **Pale tone** (for a light app theme): surface L about 0.94 to 0.97, chroma 0.015 to 0.04.

So the same 16 names work in both app themes, the way the palette's pane indicators already do. A backdrop that wants the opposite polarity is a per-pane choice (Dark / Light / Follow app, default Follow). The existing 16-slot palette's slot list and names must be confirmed in the build (the survey could not open the JSON's names); if it holds neutral slots, they become the first built-ins.

### 3.3 What `deriveInk` produces

For a surface colour S and a polarity, in OKLCH, keeping the surface's hue:

| Ink token | Target (contrast against S) | How |
|---|---|---|
| `--ink-text` (primary text) | **7:1** where reachable, never below 4.5:1 | the lightness nearest S's that passes, hue of S, chroma 0.01 to 0.02 |
| `--ink-muted` (secondary and placeholder text) | 4.5:1 | same, nearer to S than primary |
| `--ink-faint` (disabled, timestamps) | 3:1 (large or non-essential) | nearer still; never used for information alone |
| `--ink-border`, `--ink-divider` | 3:1 for borders of controls; 1.5:1 for decorative dividers | blend of ink toward S |
| `--ink-accent` and link | 4.5:1 as text, 3:1 as a UI mark | the app accent, tone-moved along its own hue until it passes |
| `--ink-error/-warning/-success/-info` | 4.5:1 as text, 3:1 as an icon or border | the app's status hues, tone-moved to pass; hue shift capped at 12 degrees so red stays red |
| `--ink-focus` | 3:1 against S and against the control's own surface | accent or a high-contrast fallback |
| `--ink-hover`, `--ink-selected` | washes of the ink at 8% and 16% over S | arithmetic, then the text on top is re-checked |
| scrollbar thumb and track | 3:1 / 1.5:1 | derived from border tokens |

Surfaces get the same treatment: `--surface` (the backdrop), `--surface-raised` (cards, inputs: ΔL +0.03 deep or -0.02 pale), `--surface-overlay` (menus and toasts the pane opens: ΔL +0.06 deep), each re-checked against all ink above it.

### 3.4 The automated proof

A vitest module, `backdrops/ink.test.ts` (build phase 1), walks **every built-in × both polarities × every ink token × every surface that token can sit on** and asserts the table above. A second test feeds the derivation a sweep: 4,096 colours on a grid of hue, chroma and lightness, and asserts the function either meets the targets or reports `nudged` (3.4 below), never silently fails. A third checks the status colours stay distinguishable from each other (CIE ΔE in OKLab at least 0.08 between error, warning and success) and that every status in the UI also has an icon or a word, so colour is never the only signal (audit item in section 7.2).

The existing tests this sits beside (`text-tokens.test.ts`, `status-text.test.ts`, `accent-text.test.ts`, `pane-color-scheme.test.ts`) check the app themes at 4.5:1 and 3:1; the new test reuses their helper and extends the same targets to backdrops.

### 3.5 Where WCAG's own arithmetic runs out

For white ink the contrast against relative luminance Y is 1.05 / (Y + 0.05); for black it is (Y + 0.05) / 0.05. So:

| Target | White ink works for Y up to | Black ink works for Y from | Dead zone (neither) |
|---|---|---|---|
| 4.5:1 | 0.183 (OKLCH L about 0.57) | 0.175 | none: one of them always reaches 4.5 |
| 7:1 | 0.100 (L about 0.46) | 0.300 (L about 0.67) | **Y 0.10 to 0.30, OKLCH L about 0.46 to 0.67** |
| 3:1 | 0.300 | 0.100 | none |

(L is the cube root of Y for a grey, an approximation good enough to place the band; the code uses exact conversions.)

So the mid-tone problem is real for the 7:1 goal and mild for 4.5:1 (a surface right at the crossover reaches only 4.58:1 with pure white or black, which is legal and ugly). The rule:

1. **Built-ins never sit in the dead zone**: their tones are chosen outside it (the tones in 3.2 are).
2. **A custom colour in the dead zone is nudged, not refused.** `deriveInk` moves the surface's lightness toward the nearer side by the smallest ΔL that reaches 7:1 for primary text, up to a cap of ΔL 0.08; the picker shows the nudged colour next to the chosen one ("adjusted so text stays readable") and the user can accept, pick Dark or Light, or step the nudge out.
3. If even the cap cannot reach 7:1, the build falls back to 4.5:1 with a flag, and the picker says "readable, not high-contrast".
4. A user may not turn the nudge off. Contrast is the feature.

## 4. Custom colour

The picker (section 10) takes any colour (hex, the OS-style picker, and an eyedropper where the engine has it). Flow: parse, convert to OKLCH, clamp chroma to what sRGB can show, run `deriveInk`, show the result live. The user's chosen hue and chroma are kept; only lightness may move (3.5). The chosen colour is stored as typed (`#RRGGBB`); the derived ink is not stored, it is recomputed at read time by the same function, so a later improvement to the derivation reaches old backdrops. (The cost is one pure function per pane at mount; it is cached per `(colour, polarity, app theme)`.)

## 5. Image backdrops

### 5.1 What the user picks and where it lives

- A file from disk (PNG, JPEG, WebP, AVIF; no SVG with scripts, no animated formats: an animated GIF or WebP imports its first frame), a built-in set shipped with the app (a small set of abstract gradients and textures, no photos with rights questions), or an image URL the user pastes (fetched once, stored, never hot-linked).
- Stored in the app's data folder under `backdrops/`, content-addressed by SHA-256, as a **derived copy**: the source is decoded, EXIF-rotated, downscaled to at most 2560 px on the long edge, re-encoded as WebP (quality 80, JPEG fallback if the encoder is missing), colour profile converted to sRGB. Source limit 20 MB, stored limit about 1.5 MB typical. The image is not copied into the pane's meta; the meta holds only the id. (The existing attachments store, `data/attachments`, already does content-addressed derived images; reuse it if its lifetime rules fit, otherwise a sibling folder with its own sweep: a build decision, not a user-visible one.)
- Controls: **fit** (cover, contain, tile), **position** (nine anchors plus a drag), **blur** 0 to 24 px, **dim** 0 to 80 percent (a tint towards the polarity's surface colour, not black), and **ink** (Auto, Light, Dark).

### 5.2 Keeping the foreground readable: the options

| Option | What it is | In Chromium 156 | Paint cost | Verdict |
|---|---|---|---|---|
| **Scrim**: a solid or gradient layer of the surface colour between the image and the content, alpha solved from the image | `linear-gradient` or a flat `rgba()` layer | everywhere | one extra composited layer, no per-frame work | **default** |
| Frosted card: a semi-opaque, blurred panel behind blocks of text | `backdrop-filter: blur()` over the image | supported; the app already uses it (`block/block.scss:113,136,205`) | a blur pass per card per paint, expensive on large or many cards | use only for the small raised surfaces (menus, the composer), not for the transcript |
| Per-region luminance sampling: sample the image on a grid, pick light or dark ink per region | canvas or `createImageBitmap` at import | n/a | one-off at import | **use at import to measure, not at runtime to switch ink** (text scrolls across regions; a flipping ink is worse than a dim) |
| Text shadow or outline | `text-shadow` | everywhere | per glyph, costly in a long transcript | only as a small last-resort on tiny labels over the image edge |
| Blend modes (`mix-blend-mode: difference`) | white text that inverts against what is under it | supported | forces a stacking context and an off-screen composite per blended element | **not for text**: at mid-tones the result is mid-grey on mid-grey, fails 4.5:1, is flattened in forced-colors mode, and fails automated audits on a mid-tone image (css-tricks, bgcolor, and the W3Tweaks write-up all say so) |
| `contrast-color()` | the browser picks black or white for a given colour | Chrome 147 and later (April 2026), so yes in 156 | free | **use for the solid-colour surfaces** as a cheap second opinion and fallback, not as the contrast authority (its algorithm is browser-defined, it can return white at the 4.5 crossover, and it cannot see an image) |
| Relative colour syntax, `color-mix()`, `light-dark()` | derive washes and polarity pairs in CSS | Chrome 119, 111 and 123 | free | **use** to write the washes and hover states (section 7.3) |

**Default: a solved scrim, plus one ink per pane.**

- At import, the app measures the stored image on a 16 by 9 grid: relative luminance of each cell (mean), and the 95th percentile of the cells' luminance. It picks the ink polarity from the post-dim average.
- It then **solves the scrim alpha**: the smallest alpha such that, composited over the worst cell the content can occupy (the brightest cell for dark ink, the darkest for light ink; for a pane the content covers the whole area, so the worst cell of the grid), the text target holds (7:1 where reachable, 4.5:1 floor). Alpha is capped at 0.88; past the cap the image would be invisible, so the system adds blur (up to the user's maximum) and reports "this image needs a strong dim".
- The result is stored with the backdrop (`scrim`, `ink`) so mounting needs no sampling.
- A **fallback surface colour** (the image's mean after the scrim) is painted under the image, so the pane is readable while the image decodes, after it fails to load, and in forced-colors mode.

### 5.3 What "automatically invert" means here

I read the operator's phrase as: the foreground must change to suit the background without the user choosing. So Auto ink flips light to dark **per backdrop**, chosen from the measurement, and the user sees the flip when they pick the image. It is not a CSS blend. If the operator wants the literal per-pixel inversion, it is available as an experimental option for decorative elements only (the header's pane icon, empty-state art), not for text; this is decision D5.

## 6. Per pane type

| Pane | Follows the backdrop | Keeps its own colours, and why |
|---|---|---|
| **Agent** (the first target) | surface, transcript text and muted text, composer, cards, status strip, dividers, scrollbars, the shell drawer's chrome | the provider's brand marks and the agent identity colour (`frame:color`): they identify, they are not backdrop |
| **Terminal** | the pane's frame and header; the terminal's default background becomes the backdrop's surface (today it paints `--main-bg-color`, `view/term/term.scss:15`) | the 16 ANSI colours and the user's terminal theme: programs depend on them. Option **Follow backdrop** (off by default): any ANSI colour below 4.5:1 on the surface is tone-moved, keeping its hue, so `ls` colours stay readable on a pale backdrop. Terminal images remain the terminal's own image feature; a backdrop image takes its place only if the terminal has none |
| **Editor** | frame, tabs, gutter, find bar, status line | the syntax theme. Option **Follow backdrop** (off by default) re-tones tokens below 4.5:1 |
| **Files, Kanban, Swarm, other app panes** | everything: they are built from kit components and tokens | nothing, except charts' categorical colours, which are checked for 3:1 and moved like status colours |
| **Browser pane** | the frame, header, loading and error overlays | the page: it is a native view the engine draws; the app cannot restyle a website (nor should it). It could send the page `prefers-color-scheme` from the backdrop's polarity (a build decision with a setting, off by default) |
| **Sandboxed widgets** | the widget SDK's theme variables (section 7.4) | a widget's own palette inside its frame; the SDK gives it the tokens, it may use them |
| **Pane header and tab pills** | the header's surface and its text come from the backdrop; the pill and the left indicator keep the `frame:color` identity, tone-checked against the strip | the identity colour itself |

## 7. The framework, theme aware throughout

### 7.1 Reading of the operator's line

"The whole framework would need to be theme aware" is read as: every component that can draw inside a pane takes its colours from the pane's scope, so no component has to be edited when a new backdrop exists, and a component that ignores the scope is a defect found by a check. That covers the UI kit, every pane's own styles, the menus, tooltips, toasts and popovers a pane opens, the widget SDK, and the status colours. An alternative reading is "the widget framework" only; the spec covers it as one of the parts (7.4). Decision D2.

### 7.2 The audit (survey of `09cb71400`)

Tokens are defined once on `:root` (`frontend/app/theme.scss`, light overrides near line 789; the 13 app themes in `frontend/app/themes/*.scss`). A pane's body paints `var(--main-bg-color)` in each view (`agent-view.scss:433`, `term.scss:15`, `editor-view.scss`, `files.scss:21`, `browser-view.scss`, `sandboxed-widget.scss:31`), and the frame fills `var(--block-bg-color)` (`block/block.scss:73-75`). No token is overridden per pane except the `--pane-*` indicators (`PaneChrome.scss:86`). So **the mechanism already works by inheritance: a pane that redefines the tokens on its own element re-colours everything under it that reads them.** What does not follow:

| Leak | Where it is | Size (matching lines, non-test) |
|---|---|---|
| Colour literals (`#hex`, `rgb()`, `hsl()`) in component styles | app-wide about 2,520 lines of which about 690 are the token and theme files themselves and are expected; the rest is components. Agent pane 584 (140 scss, 444 tsx), terminal 71, swarm 53, app chrome 216, UI kit (`app/element`) 103, other 490 | about 1,800 to triage, of which a large share is `rgba(0,0,0,x)` shadows and `transparent`, which pass |
| Overlays rendered through a portal to `document.body`, outside the pane's subtree, so a pane-scoped override does not reach them | `element/modal.tsx`, `element/anchored-popover.tsx`, `element/search.tsx`, `element/ui/open-menu-at.tsx`, the tooltips in `PaneTabStrip.tsx` | 5 mechanisms, many callers |
| Code that reads colours from the document, not from the pane | the widget bridge (`block/widget-bridge.ts:121-137`, `resolveVar` on `document.documentElement`), xterm theme construction, any `getComputedStyle(document.body)` | to be found by grep, about a dozen |
| Tailwind palette classes | `tailwindsetup.css` (47 literals) and any `bg-gray-*` use; `eslint.theme-colors.config.js` already bans the class names | already enforced |
| A colour that is the only carrier of meaning | status dots, diff add and remove, chart series | audited against "has an icon, shape or word too" |

The first build phase turns this table into a list per file (a script prints it) and a baseline (7.5).

### 7.3 The scope a pane sets

The pane frame element carries `data-backdrop="<id>"` and an inline style of custom properties:

```
--surface, --surface-raised, --surface-overlay
--ink-text, --ink-muted, --ink-faint, --ink-border, --ink-divider, --ink-accent,
--ink-error, --ink-warning, --ink-success, --ink-info, --ink-focus, --ink-hover, --ink-selected
color-scheme
```

and maps the existing token names onto them **inside the scope**: `--main-bg-color: var(--surface)`, `--block-bg-color: var(--surface)`, `--elevated-bg-color: var(--surface-raised)`, `--main-text-color: var(--ink-text)`, `--secondary-text-color: var(--ink-muted)`, `--border-color: var(--ink-border)`, `--accent-color: var(--ink-accent)`, `--error-color`, `--warning-color`, `--success-color` likewise, and the `-text` variants. Because every component reads the existing names today, the mapping is the whole integration; **no component is edited for the feature itself**, only the leaks above. Registered with `@property` (inherits: true, typed `<color>`) so the browser validates them and transitions are possible. A pane with no backdrop sets nothing and inherits the app's tokens exactly as now.

`--main-bg-color` also carries window opacity (`--window-opacity`, `app.tsx:103-131`): a backdrop surface multiplies its own alpha by `--window-opacity` the same way, so the transparent-window feature keeps working; images are drawn at the backdrop's opacity times the window's.

### 7.4 Overlays, menus, toasts, tooltips

A menu opened from a themed pane should look like the pane. Mechanism: **a scope registry and one portal helper.** The pane registers its resolved variables under its block id when its backdrop mounts or changes (`registerPaneScope(blockId, vars)`). Every portal entry point (modal, anchored popover, context-menu open, tooltip, toast) takes the owning `blockId` it already has or can be given, and the portal root it renders into gets those variables set on it (`PaneScope`, a 20-line component). A toast about a pane uses that pane's scope; an app-level toast (updates, MuxBus) uses the app's. Overlays opened with no block id keep the app tokens, so nothing changes for them. The rule for the surface of an overlay: it uses `--surface-overlay`, so it is a step above the pane and readable over it, never an identical surface (no "menu vanishes into the pane").

### 7.5 The widget SDK

`sdk/widget-sdk/am-widget.css` and the bridge hand a widget `--am-bg`, `--am-fg`, `--am-muted`, `--am-accent`, `--am-border`, `--am-error`, `--am-warning`, `--am-success` (and font, radius, pane hue), resolved from the **document**. The change: resolve from the **widget's own pane element**, so the values are the pane's scope; add `--am-surface-raised`, `--am-fg-faint`, `--am-info` and a `mode` that is the backdrop's polarity; re-send them when the backdrop changes (the bridge already re-sends on theme change). The widget never learns the image; it gets a computed fallback colour. Existing widgets keep working (the names are unchanged; only the values differ inside a backdrop).

### 7.6 The rule for new code, and the CI check

**Rule:** a component's colour comes from a token, never from a literal and never from the document. A colour that has no token yet adds a token to `theme.scss` and to the scope map in 7.3, not a literal in the component. Overlays go through `PaneScope`.

**Checks** (all fail CI):

1. **`check-color-literals`** (a script beside `check-motion-gates.mjs`, with a baseline file in the same style as `check-file-sizes.baseline`): fails on a *new* colour literal in `*.scss`, `*.css`, `*.tsx` and `*.ts` style strings outside an allowlist of the token and theme files; the baseline holds today's count per file and may only fall. This is the ratchet: it does not demand the 1,800 lines be fixed before the feature ships, only that none are added.
2. **`ink.test.ts`** (3.4): the contrast proof for every built-in, both polarities, and the sweep.
3. **`scope-coverage.test.ts`**: for each component kind that can draw in a pane (kit components, each pane's root), mount it inside a scope whose every `--ink-*` is a distinct sentinel colour and assert that **its computed colours resolve to sentinels**. A component with a literal shows up as a computed colour that is not a sentinel. This runs in the browser test environment on the compiled CSS (headless Edge or the CEF test harness the repo already has for layout checks); jsdom cannot resolve `var()` through compiled SCSS, so the check is in the layout harness, with the unit-level part (the scope map itself) in vitest.
4. **A portal check**: a test opens each portal entry point from a pane with a backdrop and asserts the overlay root carries the pane's scope.

## 8. The app theme, the per-pane colour, and what exists today

- **App theme (light and dark, and switching between them).** The backdrop stores its seed, not its colours. On an app-theme switch the scope is recomputed (the same function, a different polarity under Follow): the same 16 names, a deep tone becoming a pale one. A pane set to a fixed Dark or Light keeps its polarity under both app themes, and its chrome (header, tab pill) stays legible in either. The `high-contrast` app theme and Windows forced-colors suppress backdrops (section 12).
- **Per-pane colour (`frame:color`).** Unchanged. It is identity, not surface. Headers get their surface from the backdrop and their indicator from `frame:color`.
- **The existing `bg` meta (migration).** The survey found `bg`, `bg:opacity` and `bg:blendmode` still honoured (`util/muxutil.ts:131-149`, applied to the pane's inner element at `blockframe.tsx:1255-1256`), with no writer in the frontend (the colour menu writes `frame:color`; the `bg:*` tier was removed, `focus-ring-color.test.ts:1-5`), and a reducer in `store/agent-document`. They can still come from the terminal's image, from MuxAPI and from old workspaces. **Migration: nothing a user set is lost or unreadable.** On read, a pane with `bg` and no `backdrop` gets an in-memory backdrop of kind `legacy`: the value is drawn as before, and its ink is derived by the same measurement as an image (a gradient or URL) or the same function (a colour). It is written back as a real `backdrop` only when the user opens the picker. `bg:blendmode` is honoured for the legacy pane only. The terminal's own image remains the terminal's feature.

## 9. Where it is stored, and how far it reaches

- **Per pane**, in block meta: `backdrop: { kind: "builtin" | "color" | "image", id | hex | imageId, polarity: "follow" | "dark" | "light", fit?, position?, blur?, dim?, scrim?, ink?, version }`. Survives restart, tab tear-off and window move because block meta does.
- **A default for new panes**: setting `pane:backdrop` (none by default), applied when a pane is created, not retroactively.
- **Per agent**: a field on the agent's definition (`backdrop`), applied to the panes it opens, so an agent keeps its look wherever it is opened. A pane's own choice wins over the agent's; "Reset" removes the pane's, which reveals the agent's.
- **Per tab and per workspace**: phase 4 (a tab-level backdrop is the same record applied to panes with none of their own). Not in the first builds.
- **MuxAPI**: `Pane` gets an action `set_backdrop` (target pane, a `backdrop` record, or `reset`), for native and container agents alike (nothing in it writes or runs on the host). An agent may set built-ins, a custom colour, and an image by attachment id (uploaded through the same attachment route container agents already use). It is recorded under the agent's name, and the user sees it in the pane menu as "set by <agent>" with Reset. An agent setting its own pane's backdrop is allowed by default (workspace rule: agents can do anything except forge the operator's voice).

## 10. The UI

- **Where:** the pane's menu entry "Backdrop…" (next to the existing pane colour), the command palette ("Backdrop: …"), and Settings > Appearance > Pane backdrop for the default.
- **Layout (an inline panel in the pane, no window modal):** a 4 by 4 grid of the 16 swatches, each drawn as a mini-pane showing a sample of text in its derived ink; a Custom tile (colour picker plus the nudge notice); an Image tile (choose, built-in set); below, the image controls (fit, position, blur, dim, ink) when an image is chosen; Polarity (Follow, Dark, Light); a one-line contrast readout ("text 9.4:1, muted 5.1:1") that turns amber when a target is missed; **Reset**.
- **Live preview:** hovering a swatch previews it in this pane (applied through the scope, reverted on mouse-out); choosing keeps it. For image controls the pane updates as sliders move, debounced to one frame.
- **Reachable with the keyboard** (arrow keys across the grid, Enter to pick, Esc to revert), labelled for screen readers by name and by its contrast readout.

## 11. Performance

- A **colour backdrop costs nothing at paint time** beyond the variables it sets (the tokens already exist and the browser repaints on a custom-property change in one pass).
- An **image backdrop** is one decoded bitmap per pane drawn as a CSS background. Memory is about width times height times 4 bytes at the decoded size (a 1080p pane about 8 MB; a 2560 by 1440 stored image about 15 MB if shown at full size). Mitigations: the stored copy is capped at 2560 on the long edge; the image is drawn at the pane's size (`background-size` with a decode size hint via `image-set` or a canvas downscale for panes under half the stored size); **a pane in a hidden tab, or hibernated, drops the layer** (the existing hibernation and visibility hooks remove `background-image`, and re-adding it re-decodes from the cached file); one image used by several panes is one decoded resource (same URL).
- A scrim is one extra layer; a blur on the image is applied once at import for the stored copy where the user's blur is fixed, so no `filter: blur` runs per frame (live preview uses the filter, the committed value uses the baked file). `backdrop-filter` is restricted to small raised surfaces.
- **What to measure before it ships** (a lab, which needs the operator's yes through AgentY; **proposed, not run**): renderer memory and GPU memory with 1, 4 and 12 visible image panes and with the same 12 in hidden tabs; scroll frame time in a long agent transcript with and without a scrim and with `backdrop-filter` on the composer; time to first paint after a tab switch with an image pane. Question it answers: whether the 12-pane case needs a cap (my default if untested: at most 4 decoded images alive at once, the rest fall back to their colour). Where: the Windows host that has the reproducible bench, on current main plus the phase 3 build, about 30 minutes, no real mouse or keyboard.

## 12. Accessibility

- **Standard we hold to: WCAG 2.2 level AA contrast (4.5:1 text, 3:1 large text and UI components), with 7:1 as the target for primary text** (AAA's level), because it is the one law and procurement reference and it is what the repo's existing tests assert. APCA (Lc values) is a promising perceptual method and the research below uses it as a second opinion for hard cases (target Lc 60 or more for body text in the preferred range, 75 preferred), but **it is not a W3C standard**: the WCAG 3 draft is still a Working Draft and, as reported by secondary sources in September 2026, does not name APCA in its text; confirm at w3.org before quoting it as final. So: AA is the gate; APCA is a diagnostic in the derivation tests that never overrides AA.
- **High-contrast app theme and `forced-colors: active`:** backdrops are suppressed: the pane uses system colours, an image is not drawn, the fallback surface is the system `Canvas`. The picker says so.
- **Reduced transparency (`prefers-reduced-transparency`) and the OS "reduce transparency" setting:** scrims become opaque at the solved alpha's colour, blur and `backdrop-filter` are off, window-opacity transparency already has its own setting.
- **Colour-blind safety:** status colours keep hue within 12 degrees, are separated by at least 0.08 in OKLab (3.4), and every status already shown by colour gets a second carrier (icon, shape, word); the audit (7.2) lists the ones that do not.
- **Motion:** a backdrop is **static**. No animated image, no animated gradient, no parallax. Switching between backdrops cross-fades surface colours over the app's decorative-fast duration, gated by the existing motion policy (`check-motion-gates`) and disabled under `prefers-reduced-motion`.
- **Focus and selection:** the focus ring and text selection colours are derived (3:1 against the surface and against the control), never the browser default over a coloured surface.

## 13. Research

Dated and linked; read 2026-11-15. Facts below are from those pages; where only secondary sources were reachable, it says so.

**Contrast standards.** WCAG 3 is still a W3C Working Draft; reporting from September 2026 (qualibooth, colourwise, vervali guides) puts the latest draft at 10 September 2026, says its contrast text marks the algorithm as to be determined, and says APCA's visual-contrast section was moved out of the draft in July 2023 for further evaluation. WCAG 2.x AA ratios (4.5:1 and 3:1) remain what laws and procurement point to. APCA gives an Lc value (about 0 to 106) that depends on text size, weight and polarity; commonly cited anchors are Lc 90 preferred for body text, 75 minimum for body text, 60 for headlines, 45 large display, 15 invisible. Sources: https://www.qualibooth.com/resources/wcag-22-vs-30-whats-coming/ , https://colourwise.app/accessibility/apca-and-wcag-3-draft , https://cssdna.com/blog/wcag-vs-apca-contrast/ . Decision: WCAG 2.x is the gate (section 12).

**CSS features and our engine (Chromium 156).**
- `contrast-color()`: shipped in Safari 26 (Sept 2025), Firefox 146 (Dec 2025), Chrome and Edge 147 (April 2026), Baseline newly available April 2026. It returns white or black, leaves the algorithm to the browser, advises against plain WCAG 2.1 ratio, and is weak at mid-tones; guard with `@supports`. https://una.im/contrast-color , https://www.smashingmagazine.com/2026/05/building-self-correcting-color-systems-contrast-color/ .
- Relative colour syntax: Chrome and Edge 119; `color-mix()` and `oklch()`: Chrome 111; `light-dark()`: Chrome 123 (needs `color-scheme`). https://developer.mozilla.org/en-US/docs/Web/CSS/Guides/Colors/Using_relative_colors , https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Values/color_value .
- `@property` typed custom properties (Chrome 85), cascade layers (Chrome 99), `@scope` (Chrome 118): all well below 156. Verify each on MDN in the build; I did not fetch those pages today.
- `backdrop-filter`, `mix-blend-mode`, `image-set`: supported; costs in section 5.2.

**Text over images.** Sources agree a scrim is the dependable approach, sized to the worst pixel (composite the scrim over the worst pixel, measure, take the lowest alpha that clears the target, and keep a fallback `background-color`); luminance sampling is the sensible second tool, used best as a check; `mix-blend-mode: difference` is the weakest for accessibility (mid-tone failures, forced-colors flattening). https://bgcolor.com/blog/text-over-images/ , https://css-tricks.com/methods-contrasting-text-backgrounds/ , https://www.w3tweaks.com/css/css-mix-blend-mode-explained/ , https://www.ppimage.com/blog/text-over-image-readability-guide . This is the basis for section 5.2.

**Terminal emulators.** None found that compute readable text automatically; they give the user manual levers. WezTerm composes layers, each with opacity and an HSB transform, and its documentation example dims the image to 10 percent brightness. iTerm2 has a Blending slider and an option to keep coloured cells opaque. https://wezterm.org/config/lua/config/background.html , https://iterm2.com/documentation-preferences-profiles-window.html . (Windows Terminal's image settings were not retrieved; add before the build if wanted.) Lesson taken: the dim and blend are user levers, and our addition is that the system solves the lever's value from the image.

**Material You and tonal palettes.** One seed expands through the HCT space (hue, chroma, tone) into tonal palettes assigned to semantic roles; a `contrastLevel` from -1 to 1 maps to target ratios by a contrast curve with tone-delta constraints between paired roles. The same idea as 3.3: a seed plus a rule, with the roles' tones constrained against their pairs. https://github.com/material-foundation/material-color-utilities/blob/main/dev_guide/creating_color_scheme.md , https://docs.materialkolor.com/material-color-utilities/com.materialkolor.scheme/-dynamic-scheme/index.html . Adoption note: HCT is Google's space; OKLCH (already in the repo) gives the same property (tone-stable lightness) with CSS support, so we do not take a new dependency.

**Scoped themes in token systems.** Angular Material 3 scopes a theme by emitting its custom properties on the element where the mixin is applied, so a subtree is themed by inheritance: the same mechanism as 7.3. https://www.resumelens.org/blog/angular/angular-material-3-deep (secondary).

**Browsers and editors.** Arc themes each Space with up to three colours as a gradient plus intensity and grain; Arc's own text-colour logic is not documented, and third-party clones derive text from the theme's light or dark appearance. VS Code themes a window per workspace through `workbench.colorCustomizations` (workspace values override user values, keys such as `titleBar.activeBackground`), which is the closest precedent for per-window colours and shows the risk: users must pick the foreground themselves, and mistakes make unreadable bars. Our difference is exactly the operator's point: pick one thing, derive the rest. https://thesweetsetup.com/first-look-arc-browser/ , https://marketplace.visualstudio.com/items?itemName=RajeshRenato.workspace-colors (secondary). Slack and Discord were not researched in depth; both give a fixed set of sidebar themes with a custom option, and none I found scopes themes per pane. **Gap to fill in the build if wanted.**

## 14. Phases

Each phase is a mergeable PR with its tests and macOS and Linux look checks where it is visible.

| Phase | Scope | Visible? | Tests and checks |
|---|---|---|---|
| **0. Foundations** | the scope map (7.3) and `PaneScope`; the audit script and `check-color-literals` with its baseline; `@property` registrations; `deriveInk` and its tests; the pure parts only, no picker | no | `ink.test.ts`, the sweep, the baseline ratchet |
| **1. Colour backdrops, agent pane** | the 16 built-ins, custom colour with nudge, the inline picker, live preview, reset, per-pane meta, migration of `bg`; the agent pane and its kit components fixed for the leaks found in phase 0 | yes | scope-coverage for the agent pane; look checks dark and light at 100 and 150 percent zoom |
| **2. All pane types and overlays** | files, editor chrome, Kanban, Swarm, headers and pills; the portal helper through modal, popover, menus, tooltips and toasts; the widget SDK scope | yes | portal check; scope-coverage for every pane root; widget bridge test |
| **3. Image backdrops** | import and derived store, the solved scrim, the controls, forced-colors and reduced-transparency handling; the lab runs here (with the operator's yes) and decides the live-image cap | yes | measurement script; look checks with a bright, a dark and a busy mid-tone image |
| **4. Reach** | `pane:backdrop` default, per-agent backdrop, MuxAPI `set_backdrop`, tab and workspace defaults; terminal and editor "Follow backdrop" | yes | MuxAPI tests; container-agent test |

**Left out, on purpose:** animated backgrounds; per-region or per-line adaptive ink; backdrops for the browser pane's page; sharing or syncing backdrops between machines; the operator's literal per-pixel inversion for text (decision D5); user-authored derivation rules.

## 15. Decisions for the operator

Each with my recommendation.

| # | Decision | Recommendation |
|---|---|---|
| D1 | The feature's name | **Backdrops**; the foreground half is "ink" (section 2) |
| D2 | How far "the whole framework" reaches | Everything that draws in a pane: kit, every pane, overlays, the widget SDK, status colours, enforced by a ratchet check (7.1, 7.6). Slower start, no regressions later |
| D3 | The 16 | The existing 16-slot palette's hues, a deep tone for dark app themes and a pale one for light, so the same names work in both (3.2) |
| D4 | Contrast target | WCAG 2.2 AA is the gate; 7:1 for primary text where reachable; APCA as a diagnostic only (section 12) |
| D5 | What "invert" means | Auto ink flips light or dark per backdrop from a measurement, with a solved scrim; the CSS difference blend is not used for text, only offered experimentally for decorative marks (5.3) |
| D6 | A mid-tone custom colour | Nudge its lightness the smallest amount that gives readable text, show the adjustment, never refuse (3.5) |
| D7 | Terminal ANSI colours and editor syntax colours | Keep their own colours; "Follow backdrop" is an option, off by default (section 6) |
| D8 | Where backdrops reach first | Per pane in phase 1; per agent, tab, workspace and MuxAPI in phase 4 (section 9) |
| D9 | Image limits | 20 MB source, 2560 px stored, a static first frame for animated formats, at most 4 decoded image panes alive until the lab says otherwise (sections 5.1 and 11) |
| D10 | The performance lab | Approve the proposed lab (section 11) when phase 3 is ready, or decide the cap of 4 without it |
| D11 | Phase order | 0, 1, 2, 3, 4 as in section 14; or swap 3 and 2 if images matter more than overlays |
| D12 | Whether a backdrop's polarity may differ from the app theme | Yes, per pane (Follow, Dark, Light), default Follow |

## 16. What this reuses and what it replaces

**Reuses:** the `:root` token system and its names (`theme.scss`), the 16-slot palette and `oklch.ts`, `contrastRatio` in `pane-color-scheme.ts`, the 4.5:1, 3:1 targets and test helpers of the contrast specs (SPEC_LIGHT_THEME_RULES_AND_CONTRAST_ENFORCEMENT_2026_10_01, SPEC_COLOR_THEME_TOKEN_HARDENING_2026_09_21), the `--window-opacity` mechanism, block meta storage, the widget bridge's re-send on theme change, the existing tooltip and portal primitives (given a scope), `check-motion-gates` for transitions.

**Replaces:** the legacy `bg`, `bg:opacity`, `bg:blendmode` meta route as the way to colour a pane's body (kept readable as `legacy`, section 8); the idea of one `--block-bg-color` per app as the only surface; `resolveVar` on `document.documentElement` in the widget bridge (now per pane). It does **not** replace `frame:color` or the app themes. SPEC_COLOR_PALETTE_EXPANSION_REUSE_2026_06_30's proposal to remove the pane colour submenu is unrelated and stays its own decision.

## 17. Open questions the survey could not settle

- The names and neutral slots of the 16-slot palette JSON; `SPEC_LIGHT_THEME_RULES_...` is cited for its targets, but its R3 and its proposed `theme-contrast.test.ts` and literal lint were not found in the code (the present enforcement is the tests named in 3.4 and the Tailwind class ban).
- Whether the native browser pane exposes any frame-colour hook beyond its header (section 6 assumes not).
- The Rust-side reducer for `bg` in `crates/srv/src/reducer/workspace.rs` was not read; the migration in section 8 must read it before the build.
- Slack, Discord and Windows Terminal details (section 13).
