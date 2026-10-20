# REPORT: the Toolchain pane and the app's look

**Status:** active. Tier A (section 4) is merged (#5198). The shared `Pill` (section 4, "Shared pieces") is in the PR that edits this line, with the Toolchain pane as its first user. The Card/StatusDot rebuild (Tier B) and the colour-literal check are not started.
**Date:** 2026-10-19
**Author:** Agent1@narko
**Verified against:** `agentmuxai/agentmux` `main` @ `7269860e5` (v0.59.35), by reading code and git history. The findings were first made from the code alone. **Addendum 2026-10-20, with Tier A:** the pane's real stylesheet was then rendered statically (its markup, the real theme and kit stylesheets, headless Chrome) on `default`, `midnight`, `light` and `solarized-light`, before and after, which confirmed the surface mismatch (3.1) and the pill problems (3.2); the contrast risks in 3.3 were measured from the theme files (section 4, Tier A, item 2). The full app was not run.

## 1. The question, and the short answer

The operator noticed that the Toolchain pane does not follow the colour scheme well, and asked whether it was written before the colour scheme system was settled.

**Yes, mostly.** The pane's structure was written on 2026-06-15 (as a modal) and ported to a pane on 2026-06-25. The theme system and the shared component kit settled around it, later:

| Thing | Landed |
|---|---|
| `theme.scss` | 2026-03-03 |
| Named themes (`themes/*.scss`) and the `--space-*` scale | 2026-04-23/24 |
| `themes/light.scss` | 2026-07-12 |
| `element/ui` `Button`, `Field`, `SegmentedControl`, `Tabs`, inputs | 2026-10-06 (#4365) |
| `element/ui` `FilterInput` | 2026-10-08 |
| `element/ui` `Card`, `CardGrid`, `StatusDot` | 2026-10-14 (#4851) |
| `--tint-1..4` | 2026-10-15 |

The pane's rows, pills, section titles and env lines all come from June. Later work added content (external widgets, local models, version drift, disk-space prune, one-click installs) on the same classes. Two later sweeps touched the stylesheet but not its visual design:

- **#4383 (2026-10-06)** moved the pane's buttons onto `ui/Button` and deleted the old button CSS. It left the pills, rows, port input and path dump alone.
- **#4973 and #4975 (2026-10-15)** removed undefined variables and fallbacks and swapped in shared mixins (`label-caps`, `truncate`). #4973 explicitly kept the pane's `rgba(127,127,127,..)` greys "as a literal" and called them candidates for the tint tokens. They are still there.

So the pane is a June design with an October button swap, sitting in an app whose other panes were rebuilt on the October kit. That is why it looks like it belongs to a different app.

## 2. What "the app's look" is, in the rules that bind a pane

- **Tokens, not literals.** A component never contains a colour literal for a theme-dependent purpose (SPEC_LIGHT_THEME_RULES R6, status *proposed*; SPEC_COLOR_THEME_TOKEN_HARDENING, implemented for Tailwind classes).
- **No fading text with `opacity`** (light-theme R4). Use a secondary-text token.
- **Contrast** 4.5:1 for text and 3:1 for UI parts on light themes (R3).
- **A pane paints `var(--main-bg-color)`.** The frame behind a pane is `--block-bg-color`, darker than the theme surface in every dark theme. A pane that paints nothing shows that darker layer. `--main-bg-color` also carries the window's opacity, so a pane that paints it follows window transparency. This is the Tower finding (SPEC_TOWER_THEME_SURFACE_AND_COLOR §1.1), fixed there with one line.
- **Shared scales:** `--space-*`, `--text-3xs..3xl`, `--radius-*` (square, 0, except `--radius-full`), `--tint-*`, and `--success|warning|error|accent-bg|-line` washes built from the theme's own semantic colours.
- **Controls come from `element/ui`.** Line style: a control never fills; state is a 1px line, a tone colour and at most a faint tint.
- **Enforcement is thin.** The theme-colour ESLint rule only catches Tailwind palette classes in `.tsx`. Stylelint (`color-no-hex`, etc.) is not run in CI. `scripts/check-ui-primitives.mjs` counts hand-rolled buttons, solid fills, raw radii, undefined vars, raw px font sizes, raw z-index and custom focus rings, but not `rgba()` colour literals or rem font sizes. **Nothing catches `rgba(127,127,127,..)`.** A grep finds 9 of them in the repo's SCSS, and 5 are in this pane.

## 3. Findings, ranked by how much they affect the look

Line numbers are in `frontend/app/view/toolchain/toolchain-view.scss` unless noted.

### 3.1 The pane paints no surface (highest impact, one-line fix)
`.toolchain-view` (lines 8-14) sets layout only. It shows the frame's `--block-bg-color`, so the pane sits on a darker layer than Hangar, Tower, Swarm, Drone and Identity, which all paint `var(--main-bg-color)`. On a translucent window it also does not follow the window opacity the way those panes do. This is very likely the most visible "doesn't match" symptom.

### 3.2 Status pills ignore the theme
`.toolchain-pill` (lines 208-232) is a local invention, and the shared kit has no pill:
- Tints are fixed literals: `rgba(46,160,67,.18)` (ok) and `rgba(219,154,4,.18)` (warn and update) at 219, 223, 230. They do not follow the theme's own success and warning colours. The tokens exist: `--success-bg` and `--warning-bg` (`theme.scss:507-514`), at 12%.
- `--warn` and `--update` are identical rules.
- The base pill is `rgba(127,127,127,.15)` and the `--muted` variant fades the whole pill with `opacity: .7`, text and background together.
- `border-radius: 999px` is the only round-cornered pill-shaped thing in an app whose scale is square; its own input and `<code>` boxes use raw `4px`, while the path dump and rows use `--radius-md`, which resolves to 0. The pane mixes square and rounded corners.
- Status is communicated by text plus a colour, never by colour alone (good), but one grey pill carries at least six unrelated meanings ("Checking…", "managed", "up to date", "ahead of pin", "pin X · upstream Y", "Not installed (optional)"), and "Not found" and "update available" share one amber.

### 3.3 Greys and fades instead of text and surface tokens
- Hover, path dump, port input, install-command box and pill backgrounds are all `rgba(127,127,127,.07-.15)` (lines 86, 104, 170, 195, 216). A neutral 127 grey is a mid-grey on every theme: right-ish on one, wrong on another. The kit's answer is `--tint-1..4` (washes of the theme's own text colour) or `--hover-bg-color`.
- Secondary text is made with `opacity` on eight selectors (46, 65, 74, 109, 116, 141, 160, 227), values 0.5 to 0.85. The theme already has `--secondary-text-color`, `--grey-text-color` and `--tertiary-text-color`. The path line (`.55`) and port label (`.5`) are the weakest-contrast text in the pane; on the light themes this is the likeliest place for failing contrast. The missing-row *name* is also dimmed to .85, so the identifying text of a missing tool is quieter than an installed one's.

### 3.4 Type and spacing are off-scale
- Ten distinct rem font sizes between 0.72 and 0.9rem (lines 43, 55, 64, 87, 136, 140, 159, 166, 192, 212). The shared scale is `--text-3xs..3xl`. `check-ui-primitives` counts raw `px` sizes only, so rem sizes slip through.
- Eighteen spacing values are raw px (2, 3, 4, 5, 6, 8, 18, 20, 26, 60, 92, 180). Tokens exist for most; some (3px, 5px, 18px, 92px) are just arbitrary.

### 3.5 Hand-rolled parts of the kit
- **Rows** (`div.toolchain-row`) instead of `Card`/`CardGrid`.
- **Port field**: a native `<input type=number>` with its own style (lines 164-184), where `NumberInput` and `Field` exist. It applies only on blur, shows no error on bad input (it silently reverts), and its focus style removes the outline instead of using the shared focus ring.
- **Section titles**: `h3.toolchain-section-title` with `opacity: .6`, where the kit has `.ui-card-group-heading` (same label-caps look, a real token colour).
- **No filter**, no density class on the root (so the pane runs "comfortable" while Connectors and Memory run compact), no `paneZoom` capability (Connectors, Agent, Editor and Files declare one).
- `SystemToolInstallInline` and the install progress embed still use the legacy `@/element/button` with `className="green solid"`, a solid fill the line-style kit forbids, and a hand-styled native `<button>` for "copy details" in `InstallProgress`.
- The install steps and log declare `@container modal-mount` rules, but `modal-mount` is declared only on the modal layer, so inside a pane the compact variants never apply.
- Dead class names with no CSS: `toolchain-btn`, `toolchain-btn--ghost`, `toolchain-link-btn` (about 14 uses in the TSX), plus a "Buttons" comment block in the SCSS that describes rules deleted in #4383.

### 3.6 Behaviour that reads as unfinished
- Refresh is never disabled, so repeated clicks restart every probe; Check-latest swaps its own spinner instead of the `Button` `busy` prop.
- "Remove N · free X" (old CLI versions) is a quiet button with no confirmation and no danger tone. The kit has `tone="danger"` and `ConfirmModal destructive`.
- Icon-only buttons name themselves with `title`; the kit's `IconButton` provides an accessible name and a tooltip.
- A failed environment probe shows "Loading…" forever; a failed tool probe looks the same as "Not found".
- Copy is inconsistent for one condition: "Not found", "Not installed", "Not detected", "Not installed (optional)"; section titles mix sentence case ("Core tools") and title case ("External Widgets").
- Rows get a hover wash although they are not clickable.

### 3.7 Registration is fine, with one gap
`defaultHue: 210` and the `wrench` icon follow the pane-colour scheme (210 is used by no other view, so it is distinguishable). The manifest declares no `capabilities` (no `paneZoom`).

## 4. Recommendation

Two tiers. The first can go alone and fixes most of what the operator is seeing; the second is the real integration.

### Tier A: a tokens-only pass (small, low risk, one PR)
No markup change, no behaviour change; the stylesheet and a handful of lines of TSX:
1. `.toolchain-view { background: var(--main-bg-color); }` (3.1).
2. Pills: `--ok` / `--warn` onto the `--success-bg|-line` and `--warning-bg|-line` washes; merge `--update` into `--warn`; the plain pill is `--secondary-text-color` on `--tint-3`, with no `opacity`. **Measured, and the plan changed:** the first plan put the text in `--success-text-color` / `--warning-text-color`. Computed from the theme files, that text on a 12% wash of itself is about 2.6:1 on catppuccin-latte and solarized-light, 2.9 to 3.0 on gruvbox-light and nord (warning), 3.4 to 4.4 on several dark themes, and below the 4.5:1 AA bar on every theme but a few: the themes' semantic colours were never tuned for 11px text. So the pill text is the main text colour (at least 4.5:1 on every wash in all 12 themes), and the status is carried by the wash, a line in `--success-line` / `--warning-line` and the icon. `toolchain-pill-colors.test.ts` pins this. It also found that solarized-light's secondary text colour is about 2.6:1 on its own surface wherever it is used, so the plain pill is pinned there, as a theme-level issue for another day. Give the pill `--radius-full` and the input/code boxes the shared radius token so the pane stops mixing shapes.
3. Every `rgba(127,127,127,..)` to `--tint-1..4` or `--hover-bg-color`.
4. `opacity`-faded text to `--secondary-text-color` / `--grey-text-color` / `--tertiary-text-color`; stop dimming the missing-row name.
5. rem font sizes and px spacing onto `--text-*` and `--space-*` (snap the 3/5/18/92px outliers to the nearest token).
6. Delete the dead class names and the dead comments; add `capabilities: { paneZoom: {} }`.
7. Replace the legacy `Button` in `SystemToolInstallInline` and the native "copy details" button with `ui/Button`. Not done: `paneZoom`. The capability only enables the zoom handlers; a pane has to apply the zoom itself (Connectors' section pane does `style={{ zoom }}` from its model), and the Toolchain has no model, so it belongs with Tier B.

Acceptance: `scripts/check-ui-primitives.mjs` counts do not rise; no `rgba(127,127,127` left in the folder; a screenshot of the pane on at least `dark` (default), `midnight`, `light`, `solarized-light` and `catppuccin-latte` at 100% and 70% window opacity (see section 7).

### Tier B: rebuild on the Connectors/MCP pattern (medium PR, or two)
The closest model is the **MCP manager and Remotes** (`mcp-manager.tsx`, `remotes-view.tsx`): both are status-plus-actions lists on `Card` / `CardGrid` / `StatusDot` / `FilterInput`, with group headings, hover actions, inline details and a notice strip. Use Tower for the toolbar and surface, and the Settings widget catalog rows for the catalog detail. Memory's tiles and Warden's table rows are not models: Memory is a second card system and Warden hardcodes its status colours.

| Toolchain part | Becomes |
|---|---|
| Pane root | One `ui-card-section ui-density-compact` root that is the scroller, painting `--main-bg-color`. |
| Header strip | `ui-card-section-header`: title, a count ("N tools behind"), `FilterInput`, `IconButton` Refresh, and the Check-latest button. |
| Environment lines | A group heading plus label/value rows; the PATH dump behind a quiet disclosure (as Remotes' hidden group). |
| Core tools, Local models, Agent CLIs | `CardGrid` per group under an `h4.ui-card-group-heading`. Card: tool icon, name, version or path as subtitle, source in `meta`, and `status` from a `StatusDot` (installed = ok, missing or update = warn, optional-missing = idle, checking = busy, probe error = error). Required-but-missing gets `attention`; optional gets `muted`. |
| Install / update / docs / copy | Card detail row (`renderDetail`): accent `Button` for Install or Update, `IconButton`s for Docs and Copy command. The one-click installer expands inside this detail. |
| External widgets | A `CardGrid` with status Running / Installed / Missing; the detail holds the description, "Requires", the port as `Field` + `NumberInput` (apply on Enter and blur, with a visible error) and Reset as an `IconButton`. |
| Old CLI versions | A group with a `line.notice(...)` for the result, a quiet Check button, and a `tone="danger"` Remove behind `ConfirmModal destructive`. |
| Errors, loading, empty | `line.notice(danger)` in a `role="alert"` strip; `ui-card-empty` for loading and for "No tools match “x”." |

Tier B also removes the state-per-row inconsistencies in 3.2 and 3.6, because status becomes a kit concept instead of a local class.

### Shared pieces worth adding to `element/ui` (benefits more than this pane)
The survey found these re-implemented in several panes; Toolchain would be the first consumer of the first:
1. **`Pill` / `Badge`** (tone, icon, text). Today: `toolchain-pill`, `tower-badge`, `widget-row-state`, `warden-host-state` (with literal colours), `swarm-status-chip`, `memory-file-card-badge`, `ui-card-badge`, and more.
2. **`Notice`** (tone, dismiss) over the existing `line.notice` mixin: seven different error strips today.
3. **`EmptyState` / `Loading`**, **`SectionHeader`**, and a **`PaneBody`** wrapper that gives the pane surface, scroller and padding, so the omission in 3.1 cannot happen again.

## 5. Things to preserve
- The install flow (`SystemToolInstallInline` and the shared install kit, including auto-scroll of Details) and the tests that pin it (`SystemToolInstallInline.test.tsx`, 317 lines; `CliDiskSpace.test.tsx`; `toolchain-capabilities` and `toolchain-catalog` tests).
- The "managed / your install / ahead of pin / pin X · upstream Y" drift vocabulary and the Core/Local/Agent split, which are product content, not styling.
- The port-override semantics for external widgets and the widget `Open Pane` action.
- The hamburger-menu and More-dropdown entry points, `defaultHue: 210` and the wrench icon.

## 6. Suggested sequence and size
1. **Tier A**, one PR: the stylesheet, a few TSX lines and two small component swaps. A day of work; reviewable by eye on screenshots.
2. **`Pill`** (and optionally `Notice`) added to `element/ui` with tests, then Tier B in one or two PRs: first the tool lists and header, then widgets and prune. Acceptance adds: `check-ui-primitives` counts fall, a light-theme contrast test for the status colours (the proposed `theme-contrast.test.ts` from the light-theme spec does not exist yet; a small local one for the pill tokens would do), and no `title`-only icon buttons.
3. Optionally add a check for `rgba(127,127,127` and colour literals in SCSS to `check-ui-primitives`: the existing gates cannot see this class of problem, which is why a pane like this can drift for months.

## 7. Not verified, and what to look at
- **The full app was not run.** The static render (see the addendum at the top) used the pane's real stylesheets on four themes. Not covered: the other themes visually, a translucent window, narrow widths, and a screenshot from a running AgentMux. Contrast is computed for the pills only, not for the faded path and port-label text (alpha colours the test does not resolve).
- How a bare `<code>` is styled by the global stylesheet (the install command box sets no font family).
- Row counts in each catalog, so how long the lists get; whether a filter is needed depends on it.
- Whether `check-ui-primitives` currently flags this folder in CI (it compares with the merge base and keeps no baseline file).
- The June commits `1e4da3542` and `c175193b6` were dated from their titles and the file's own header, not diffed line by line.
