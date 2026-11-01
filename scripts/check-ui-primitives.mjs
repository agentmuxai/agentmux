// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-ui-primitives.mjs — CI ratchet for the line-style component set.
//
// docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §7
//
// THE RULES. Controls come from frontend/app/element/ui/ and follow its line
// style. The counts below may not go up compared with the base branch (the
// merge-base with origin/main, or $GITHUB_BASE_REF):
//
//   1. buttons      hand-rolled `<button` elements in .tsx outside element/ui/,
//                   per file. A hard zero everywhere, the agent pane included
//                   (#13044): a file passes only through BUTTON_ALLOWLIST below, with a
//                   reason.
//   2. solidFills   SCSS rules on a control (a button, tab, toggle or an
//                   active/primary/confirm/danger state) whose background is a
//                   solid `var(--accent-color|--error-color|--success-color)`
//   3. radius       `border-radius` values other than 0, `var(--radius-*)`,
//                   50%, inherit or none, per file (SPEC_HARD_CORNERS_2026_05_26).
//                   A hard zero everywhere, like rules 1 and 8.
//   4. undefinedVars `var(--x)` where `--x` is declared nowhere in frontend/
//                   (not in SCSS/CSS, not set from TS). Every use silently
//                   takes its fallback: settings.scss's `--panel-bg-alt` was
//                   a fixed grey in every theme.
//
// Five more hold the shared scales (SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_
// 2026_10_15.md §6.6, item 20, and §6.3, item 10), all per file, in SCSS/CSS:
//
//   5. durations    raw times (`120ms`, `0.5s`) in transition and animation
//                   declarations, where a `--duration-*` token belongs. A hard zero
//                   everywhere: DURATIONS_ALLOWLIST names the loops,
//                   one-shot keyframes and in-between values that have no token.
//   6. fontSizes    `font-size` given as a raw px or rem value, not a
//                   `--text-*` token
//                   A hard zero everywhere: FONT_SIZES_ALLOWLIST names the
//                   glyph and display sizes, and the sizes between two steps.
//   7. fallbacks    `var(--x, …)` where `--x` is set on `:root` itself, outside
//                   any at-rule: the fallback can never apply, and a wrong one
//                   misleads (`var(--radius-sm, 4px)` renders 0)
//   8. zIndex       `z-index` given as a raw number, not a `--z-*` token. A hard zero
//                   everywhere: Z_INDEX_ALLOWLIST names the files whose
//                   numbers only order a component's own children, or are xterm's.
//   9. focusRings   `:focus-visible` rules that draw their own ring (an
//                   outline or box-shadow) instead of `@include
//                   mixins.focus-ring`. `outline: none` alone doesn't count,
//                   nor does the shared ring: mixin bodies are skipped.
//                   A hard zero everywhere, with no allowlist.
//
// And two for the colour scheme (docs/reports/REPORT_TOOLCHAIN_PANE_THEME_
// INTEGRATION_2026_10_19.md §6), per file; the first in SCSS/CSS outside the theme files:
//
//  10. colours      colour literals (`#ef4444`, `rgba(245, 158, 11, 0.1)`,
//                   `hsl(…)`) where a theme token belongs: a literal is one
//                   colour in all twelve themes, light ones included. Pure
//                   black and white (shadows, scrims) and colours built from
//                   a token (`rgba(var(--x-rgb), 0.2)`, `rgb(from var(…) …)`),
//                   a palette token set on `:root`, and lines under a
//                   `stylelint-disable color-no-hex` don't count.
//
//  11. tsColours    colour literals in .ts/.tsx (an inline `style`, a canvas or SVG
//                   fill, a generated palette), per file, other than pure black and
//                   white. Comments don't count; a line marked `// colour: fixed`
//                   (or the line after a `// colour: fixed` comment) doesn't either:
//                   a brand mark, a user-pickable palette, a fallback for a page
//                   that has no stylesheet. A chrome colour is a `var(--…)` token.
//                   A hard zero everywhere: TS_COLOURS_ALLOWLIST names the
//                   files that hold a palette, a brand mark or a fallback, with a count.
//
// One for the tag and badge family (docs/specs/SPEC_UI_PANEL_PRIMITIVE_2026_10_29.md §9.5), per
// file, in SCSS/CSS outside element/ui/:
//
//  13. tags         a rule whose class name ends in badge, chip, tag, pill or ribbon and that
//                   gives itself a box (a padding, a font-size, and a border-radius or a
//                   background): a tag drawn by hand, where `Tag` belongs. A rule marked
//                   `// tag: fixed` (on its selector line, above it or inside it) doesn't count:
//                   a control that is not a tag but is named like one, a third party's markup.
//
//  14. nativeTitles `title=` on a lowercase JSX element (a button, span, div ...) in .tsx
//                   outside element/ui/, per file. The browser's own tooltip waits
//                   0.5 to 1 s; a hover label is `<Tooltip>` or an IconButton's tooltip.
//                   title-tooltip.ts shows a leftover `title` through the shared tooltip,
//                   so existing ones still work, but a new one is a regression in intent.
//                   `option`, `optgroup`, `iframe` and an element marked
//                   `data-native-title` don't count.
//                   A title on a control (button, a, input, select, textarea, summary) is
//                   also counted as controlTitles, a hard zero everywhere
//                   (CONTROL_TITLES_ALLOWLIST); a title on text (a cell, a truncated
//                   name) only ratchets, as title-tooltip.ts shows it through the shared
//                   tooltip and a Tooltip wrapper would break table and inline layout.
//
//  15. textFields   a hand-written text `<input>` or `<textarea>` in .tsx outside element/ui/, per
//                   file. A text field is `TextInput`, `TextArea`, `NumberInput` or `FilterInput`
//                   from element/ui/. A checkbox, radio, range, file or colour input does not count,
//                   nor does an element marked `data-raw-text-field` (a field the kit cannot be: the
//                   terminal's hidden input, a third-party widget's). Selecting text by dragging
//                   past a field's edge is guarded for every field (element/ui/text-drag.ts), so
//                   this rule is about one look and one set of behaviours, not about that guard.
//
//  16. menus        a menu drawn or opened outside the kit's `Menu` (element/ui/Menu.tsx), per file in
//                   .tsx outside element/ui/: `role="menu"`, `<FlyoutMenu`, `<PopoverMenu`, `<MenuButton` and
//                   `showContextMenu(`. The app grew four menu systems; the kit's Menu is the one (anchored,
//                   upward, keyboard-driven), so the count only goes down as each is converted.
//
// A count above the base's fails: use the element/ui/ components instead. A
// count below it is reported and passes. There is no checked-in baseline:
// one used to live in scripts/ui-primitives-baseline.json, and since every
// migration PR edited it, each merge put the others into conflict. Comparing
// with the base branch gives the same ratchet with nothing to keep in sync.
//
// Why a script and not stylelint: stylelint isn't run in CI (see
// scripts/check-no-transition-all.sh), and rule 2 needs SCSS nesting resolved.
//
// Usage:
//   node scripts/check-ui-primitives.mjs   # working tree vs the base branch

import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const FRONTEND = join(REPO_ROOT, "frontend");
const UI_DIR = "frontend/app/element/ui/";

// ── File walking ────────────────────────────────────────────────────────────

function walk(dir, out = []) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.name === "node_modules" || entry.name === "dist") continue;
        const full = join(dir, entry.name);
        if (entry.isDirectory()) walk(full, out);
        else out.push(full);
    }
    return out;
}

const rel = (p) => relative(REPO_ROOT, p).split(sep).join("/");
const isTest = (p) => /\.test\.(tsx?|mjs)$/.test(p);

// ── Rule 1: hand-rolled buttons ─────────────────────────────────────────────

/** Menu surfaces that are not the kit's `Menu`: a hand-made role="menu", the older menu components, the JS context menu. */
export function countMenus(source) {
    return (stripScriptComments(source).match(/role="menu"|<(?:FlyoutMenu|PopoverMenu|MenuButton)\b|\bshowContextMenu\(/g) ?? []).length;
}

export function countButtons(source) {
    return (stripScriptComments(source).match(/<button\b/g) ?? []).length;
}

/**
 * The only hand-rolled `<button>`s allowed: the file,
 * how many, and why the kit's Button / IconButton can't be it. Adding a line
 * here is a design decision for the kit's owner, not a way past the check.
 */
export const BUTTON_ALLOWLIST = {
    "frontend/app/window/window-controls.win32.tsx": {
        max: 3,
        reason: "Windows caption buttons (minimise, maximise, close) draw the OS's own glyphs and hover fills",
    },
    "frontend/app/window/window-controls.darwin.tsx": {
        max: 3,
        reason: "macOS traffic lights are round coloured dots, not a line control",
    },
    "frontend/app/window/hamburger-menu.tsx": {
        max: 1,
        reason: "the window-chrome menu trigger: a hand-drawn three-bar glyph that must stay pixel-exact, inside the drag region",
    },
    "frontend/app/element/iconbutton.tsx": {
        max: 2,
        reason: "the older widget IconButton (wave-iconbutton, with long-click) that the kit's IconButton replaces",
    },
    "frontend/app/element/PaneTabStrip.tsx": {
        max: 2,
        reason: "the tab strip's own add (+) and close (x) controls: drawn glyph, hover-revealed, sized to the tab",
    },
    "frontend/app/statusbar/StatusBar.tsx": {
        max: 1,
        reason: "the version readout uses the status bar's stat-button recipe (full-height, inset focus ring)",
    },
    "frontend/app/statusbar/SystemStats.tsx": {
        max: 2,
        reason: "the CPU and Disk readouts use the status bar's stat-button recipe (full-height, inset focus ring)",
    },
    "frontend/app/statusbar/TokenUsageIndicator.tsx": {
        max: 1,
        reason: "the token readout uses the status bar's stat-button recipe (full-height, inset focus ring)",
    },
    "frontend/app/element/markdown-media.tsx": {
        max: 1,
        reason: "the round play overlay on a video poster is a solid accent disc over the picture; the kit never fills",
    },
};

/** The z-index values that only order a component's own children, below the shared `--z-*` layers. */
export const Z_INDEX_ALLOWLIST = {
    "frontend/app/view/term/xterm.css": {
        max: 8,
        reason: "xterm.js's own stylesheet: its layers run from -5 to 10 inside the terminal and are not ours to re-token",
    },
    "frontend/app/view/term/term.scss": {
        max: 2,
        reason: "lifts xterm's scrollbar above xterm's own link layer (2) and helpers (5), so it names xterm's numbers",
    },
    "frontend/app/element/ui/ui.scss": {
        max: 1,
        reason: "raises a joined segment that is hovered, selected or focused over its neighbour, so its whole line shows",
    },
    "frontend/app/element/PaneLoadingCover.scss": {
        max: 1,
        reason: "orders the cover against the block's own z-index 0, inside the block's stacking context",
    },
    "frontend/app/element/EmptyPaneLogo.scss": {
        max: 1,
        reason: "z-index 0 keeps the faint logo behind the empty pane's own content",
    },
    "frontend/app/view/settings/settings-search-bar.scss": {
        max: 1,
        reason: "the results dropdown over the settings rows, inside the settings pane's own stacking context",
    },
    "frontend/app/view/tower/tower-view.scss": {
        max: 1,
        reason: "keeps the sticky table header over the rows that scroll under it",
    },
    "frontend/app/view/drone/drone-view.scss": {
        max: 4,
        reason: "orders the Drone canvas's own error banner, ports, run panel and zoom controls against each other",
    },
    "frontend/app/view/browser/browser-view.scss": {
        max: 1,
        reason: "the address-suggestion dropdown over the page below the bar, inside the browser pane",
    },
    "frontend/app/view/files/files.scss": {
        max: 2,
        reason: "keeps the sticky header and the rename problem note over the rows inside the Files pane",
    },
    "frontend/app/view/sysinfo/sysinfo-plot.scss": {
        max: 2,
        reason: "orders a chart's title and tooltip over its canvas",
    },
    "frontend/app/view/editor/editor-view.scss": {
        max: 3,
        reason: "orders the empty editor's shortcuts, actions and hint over its backdrop",
    },
    "frontend/app/view/agent/agent-view.scss": {
        max: 1,
        reason: "z-index 0 makes the pane's root a stacking context of its own",
    },
    "frontend/app/view/agent/shutdown/ShutdownOverlay.scss": {
        max: 1,
        reason: "the shutdown overlay over the pane's content, inside the pane's own stacking context",
    },
    "frontend/app/view/agent/styles/_btw.scss": {
        max: 1,
        reason: "the side-question overlay over the transcript, inside the pane's stacking context",
    },
    "frontend/app/view/agent/styles/_composer-strip.scss": {
        max: 1,
        reason: "the agent-lock badge over the composer strip",
    },
    "frontend/app/view/agent/styles/_document-nodes.scss": {
        max: 2,
        reason: "a tool block's actions over its panel, and the node peek overlay over the transcript",
    },
    "frontend/app/view/agent/styles/_document.scss": {
        max: 3,
        reason: "orders the scroll region, the history-loading note and a raised expanded startup message against the transcript rows",
    },
    "frontend/app/view/agent/styles/_pending-footer.scss": {
        max: 1,
        reason: "the mic button over the input it sits in",
    },
    "frontend/app/view/agent/styles/_picker.scss": {
        max: 1,
        reason: "the new-session button over its card",
    },
    "frontend/app/view/agent/styles/_recent-sessions.scss": {
        max: 4,
        reason: "orders the recent-sessions body, its active badge, menu toggle and the exiting row against each other",
    },
    "frontend/app/view/agent/styles/_search.scss": {
        max: 1,
        reason: "the in-pane search bar over the transcript",
    },
    "frontend/app/view/agent/styles/_shell-node.scss": {
        max: 1,
        reason: "the activity dock over the shell node",
    },
    "frontend/app/view/agent/styles/_slash.scss": {
        max: 1,
        reason: "the slash-command list over the composer and the transcript",
    },
};

/** Raw times in `transition` / `animation` that are not a state transition on the shared duration scale. */
export const DURATIONS_ALLOWLIST = {
    "frontend/app/app.scss": {
        max: 6,
        reason: "the reduced-motion neutraliser (1ms) and Font Awesome's own spin and delay defaults, which are its API",
    },
    "frontend/app/element/modal.scss": {
        max: 1,
        reason: "the reduced dismiss nudge's outline: a real 240 ms so it shows and animationend fires even when Off scales motion to 0",
    },
    "frontend/app/statusbar/StatusBar.scss": {
        max: 1,
        reason: "a 1.2 s spinner loop, a looping keyframe with its own length",
    },
    "frontend/app/view/browser/browser-view.scss": {
        max: 2,
        reason: "two 200 ms overlay fades (no 200 ms token; 160 or 240 would change them)",
    },
    "frontend/app/view/identity/styles/_status-dot.scss": {
        max: 1,
        reason: "the checking dot's 1 s pulse loop, a looping keyframe with its own length",
    },
    "frontend/app/view/settings/settings-search-bar.scss": {
        max: 1,
        reason: "the 1.5 s highlight fade on a found setting, a one-shot keyframe with its own length",
    },
    "frontend/app/view/swarm/swarm-view.scss": {
        max: 1,
        reason: "a 0.5 s accent flash on a row, a one-shot keyframe with its own length",
    },
    "frontend/app/view/agent/agent-view.scss": {
        max: 2,
        reason: "a 200 ms fade (no 200 ms token; 160 or 240 would change it) and the 0.5 s ant-march loop",
    },
    "frontend/app/view/agent/components/AgentQuestionPanel.scss": {
        max: 1,
        reason: "the question panel's 1 s pulse loop, a looping keyframe with its own length",
    },
    "frontend/app/view/agent/components/PaneRow.scss": {
        max: 1,
        reason: "a 0.2 s border-colour fade (no 200 ms token; 160 or 240 would change it)",
    },
    "frontend/app/view/agent/styles/_attachments.scss": {
        max: 2,
        reason: "a 900 ms spinner loop and a 900 ms attachment flash, each a keyframe with its own length",
    },
    "frontend/app/view/agent/styles/_btw.scss": {
        max: 1,
        reason: "the caret's 1 s blink, a looping keyframe with its own length",
    },
    "frontend/app/view/agent/styles/_composer-strip.scss": {
        max: 2,
        reason: "a 2.4 s looping animation and a 200 ms fade (no 200 ms token; 160 or 240 would change it)",
    },
    "frontend/app/view/agent/styles/_control-bar.scss": {
        max: 2,
        reason: "the progress bar's 1 s linear width and its 1.6 s sweep loop",
    },
    "frontend/app/view/agent/styles/_decision-panel.scss": {
        max: 1,
        reason: "the 0.5 s flash when the decision panel arms, a one-shot keyframe with its own length",
    },
    "frontend/app/view/agent/styles/_document.scss": {
        max: 4,
        reason: "three 180 ms row fades and a 90 ms fade (no 180 or 90 ms token; the nearest would change them)",
    },
    "frontend/app/view/agent/styles/_picker.scss": {
        max: 1,
        reason: "a 0.8 s spinner loop, a looping keyframe with its own length",
    },
    "frontend/app/view/agent/styles/_recent-sessions.scss": {
        max: 2,
        reason: "the reduced-motion neutraliser (1 ms)",
    },
    "frontend/app/view/agent/styles/_search.scss": {
        max: 1,
        reason: "the 0.25 s flash on a search match, a one-shot keyframe with its own length",
    },
    "frontend/app/view/agent/styles/_shell-node.scss": {
        max: 4,
        reason: "two 0.2 s border fades (no 200 ms token), the 2 s running pulse and the 1 s caret blink",
    },
};

/** `font-size` values that are glyph or display sizes, or sit between two `--text-*` steps. */
export const FONT_SIZES_ALLOWLIST = {
    "frontend/app/block/block.scss": {
        max: 2,
        reason: "the frame icon button's 45px glyph and the 60px block number in the mask: display sizes of one-off artwork",
    },
    "frontend/app/block/missing-widget.scss": {
        max: 1,
        reason: "the 24px icon glyph on the missing-widget card, sized as artwork",
    },
    "frontend/app/block/sandboxed-widget.scss": {
        max: 1,
        reason: "the 24px icon glyph on the widget failure card, sized as artwork",
    },
    "frontend/app/element/PaneTabStrip.scss": {
        max: 1,
        reason: "the add-tab button's 15px glyph, sized to its 28px-high button (no 15px token)",
    },
    "frontend/app/element/zoomindicator.scss": {
        max: 1,
        reason: "the 48px zoom readout, a display size above the scale",
    },
    "frontend/app/mixins.scss": {
        max: 2,
        reason: "the frame close icon's 25px glyph, and the extra-small avatar's 7px initials, scaled with the avatar (18px * 20/50)",
    },
    "frontend/app/view/browser/browser-view.scss": {
        max: 1,
        reason: "the 48px empty-state icon glyph",
    },
    "frontend/app/view/bundle/bundle-view.scss": {
        max: 1,
        reason: "a 20px bundle name, between --text-xl (18) and --text-2xl (22)",
    },
    "frontend/app/view/bundle-summary.scss": {
        max: 1,
        reason: "a 15px bound-agent name, between --text-md (14) and --text-lg (16)",
    },
    "frontend/app/view/drone/drone-view.scss": {
        max: 1,
        reason: "the 15px zoom-control glyphs, sized to the 26px square buttons",
    },
    "frontend/app/view/editor/editor-view.scss": {
        max: 1,
        reason: "the 32px error panel icon glyph",
    },
    "frontend/app/view/files/files.scss": {
        max: 4,
        reason: "8px chevrons and link marks beside text, and the 40px preview and tile icons: glyph sizes, not text",
    },
    "frontend/app/view/identity/identity-pane-view.scss": {
        max: 1,
        reason: "a 20px agent name, between --text-xl (18) and --text-2xl (22)",
    },
    "frontend/app/view/swarm/swarm-view.scss": {
        max: 1,
        reason: "the current-tool icon's 8px glyph",
    },
    "frontend/app/window/action-widgets.scss": {
        max: 1,
        reason: "the 8px chevron on a parent widget",
    },
    "frontend/app/view/agent/styles/_attachments.scss": {
        max: 4,
        reason: "attachment tile glyphs and the lightbox icon (24 and 64 px), the +N more count (15 px), and a 5 px miniature of the file's text in its tile: display sizes, not running text",
    },
    "frontend/app/view/agent/styles/_document-nodes.scss": {
        max: 1,
        reason: "the day divider label's 10.5px, between --text-2xs (10) and --text-xs (11)",
    },
    "frontend/app/view/agent/styles/_recent-sessions.scss": {
        max: 1,
        reason: "a 15px session name, between --text-md (14) and --text-lg (16)",
    },
};

/** A native `title=` on a control (button, a, input, select, textarea, summary) that has no kit component to take it. */
export const CONTROL_TITLES_ALLOWLIST = {
    "frontend/app/element/PaneTabStrip.tsx": {
        max: 1,
        reason: "the tab close control's title carries its state (Closing, Close with unsaved changes) and sits on a control already on the button allowlist",
    },
    "frontend/app/element/iconbutton.tsx": {
        max: 2,
        reason: "the older widget IconButton, on the button allowlist, which the kit's IconButton replaces",
    },
    "frontend/app/statusbar/InstancePanel.tsx": {
        max: 1,
        reason: "the opacity range slider: a bare input with a title and an aria-label; a Tooltip wrapper would break its row",
    },
    "frontend/app/view/editor/editor-save-as-input.tsx": {
        max: 1,
        reason: "the Save As path input in a tab: its title is the only hint for Enter and Esc, and a wrapper would break the tab's layout",
    },
    "frontend/app/view/editor/viewers/pdf/pdf-viewer.tsx": {
        max: 2,
        reason: "the page and find inputs in the PDF toolbar: their titles carry the shortcut, and a wrapper would break the toolbar's flex row",
    },
    "frontend/app/view/swarm/swarm-view.tsx": {
        max: 2,
        reason: "the row and section select checkboxes: a bare checkbox whose title says why it can't be selected",
    },
    "frontend/app/window/hamburger-menu.tsx": {
        max: 1,
        reason: "the window-chrome menu trigger, on the button allowlist",
    },
    "frontend/app/window/window-controls.darwin.tsx": {
        max: 3,
        reason: "the macOS traffic lights, on the button allowlist",
    },
    "frontend/app/window/window-controls.win32.tsx": {
        max: 3,
        reason: "the Windows caption buttons, on the button allowlist",
    },
};

/** Colour literals in .ts/.tsx that are a palette a user picks from, a brand mark, a fixed dev-tool or terminal scheme, or a fallback with no stylesheet. */
export const TS_COLOURS_ALLOWLIST = {
    "frontend/app/view/term/termutil.ts": {
        max: 24,
        reason: "the terminal's own colour schemes: the user's terminal theme, not the app's chrome",
    },
    "frontend/app/devtools/agent-pane-perf-section.tsx": {
        max: 19,
        reason: "a dev-only panel with its own fixed dark palette, legible over any theme",
    },
    "frontend/app/devtools/diag-panel.tsx": {
        max: 17,
        reason: "a dev-only panel with its own fixed dark palette, legible over any theme",
    },
    "frontend/perf/hud.tsx": {
        max: 10,
        reason: "the dev performance HUD, with its own fixed dark palette, legible over any theme",
    },
    "frontend/app/element/ProviderLogo.tsx": {
        max: 12,
        reason: "brand marks (Slack and others) drawn in their own colours",
    },
    "frontend/app/block/widget-bridge.ts": {
        max: 9,
        reason: "fallbacks for the widget CSS variables when the theme sets none (a widget's frame has no stylesheet)",
    },
    "frontend/app/init/error-display.ts": {
        max: 7,
        reason: "the fatal error page, painted before any stylesheet or theme has loaded",
    },
    "frontend/app/view/remotes/remotes-sections.ts": {
        max: 6,
        reason: "the palette a user picks a remote's colour from",
    },
    "frontend/app/view/drone/block-registry.ts": {
        max: 5,
        reason: "each Drone block kind's identity colour",
    },
    "frontend/app/view/accounts/muxbus-sign-in-choices.tsx": {
        max: 4,
        reason: "Google's own four-colour mark, drawn in its brand colours",
    },
    "frontend/app/store/command-registry.ts": {
        max: 3,
        reason: "command-palette icon colours: the Anthropic brand colour and two fixed ambers",
    },
    "frontend/app/view/term/termsticker.tsx": {
        max: 2,
        reason: "the paw sticker's own colours on one pane (an easter egg)",
    },
    "frontend/app/view/term/term.tsx": {
        max: 2,
        reason: "xterm's search decoration colours, drawn over the terminal's own scheme",
    },
    "frontend/app/view/settings/sections/appearance-section.tsx": {
        max: 1,
        reason: "a placeholder example string (#1a1a1a), not a colour the app draws",
    },
    "frontend/app/block/blockframe.tsx": {
        max: 1,
        reason: "the default header wash for a non-agent pane, held at the lightness of the tinted header (see its comment)",
    },
};

/** Per rule: the hard-zero allowlist (file → { max, reason }). Anything not allowlisted must be 0. */
export const HARD_ZERO = { buttons: BUTTON_ALLOWLIST, radius: {}, zIndex: Z_INDEX_ALLOWLIST, durations: DURATIONS_ALLOWLIST, fontSizes: FONT_SIZES_ALLOWLIST, focusRings: {}, controlTitles: CONTROL_TITLES_ALLOWLIST, tsColours: TS_COLOURS_ALLOWLIST };

/**
 * Lines for every count the hard zero refuses in `counts` (file → n) for rule `key`: any file
 * that isn't allowlisted, or has more than it is allowed, and any
 * allowlist entry whose file has none left (remove it).
 */
export function zeroViolations(key, counts, allowlist) {
    const out = [];
    for (const [file, n] of Object.entries(counts).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
        const allowed = allowlist[file]?.max ?? 0;
        if (n > allowed) out.push(`${key}: ${file} has ${n}, allowed ${allowed}`);
    }
    for (const file of Object.keys(allowlist).sort()) {
        if (!counts[file]) out.push(`${key}: ${file} has none; remove it from the ${key} allowlist`);
    }
    return out;
}

/** Rule 1's hard zero: hand-rolled buttons. */
export const buttonViolations = (buttons, allowlist = BUTTON_ALLOWLIST) => zeroViolations("buttons", buttons, allowlist);

/** Every hard-zero rule's violations for `current` (the collected counts). */
export function hardZeroViolations(current, hard = HARD_ZERO) {
    return Object.entries(hard).flatMap(([key, allowlist]) => zeroViolations(key, current[key] ?? {}, allowlist));
}

// ── SCSS walking (rules 2 and 3) ────────────────────────────────────────────

/** Remove comments, keeping line breaks and string contents intact. */
export function stripComments(scss) {
    let out = "";
    let i = 0;
    let quote = null;
    while (i < scss.length) {
        const c = scss[i];
        const next = scss[i + 1];
        if (quote) {
            out += c;
            if (c === "\\") {
                out += next ?? "";
                i += 2;
                continue;
            }
            if (c === quote) quote = null;
            i++;
        } else if (c === '"' || c === "'") {
            quote = c;
            out += c;
            i++;
        } else if (c === "/" && next === "*") {
            const end = scss.indexOf("*/", i + 2);
            const stop = end === -1 ? scss.length : end + 2;
            out += scss.slice(i, stop).replace(/[^\n]/g, "");
            i = stop;
        } else if (c === "/" && next === "/" && scss[i - 1] !== ":") {
            // `//` comment; `:` guard keeps `url(https://…)`.
            while (i < scss.length && scss[i] !== "\n") i++;
        } else {
            out += c;
            i++;
        }
    }
    return out;
}

/** Resolve a nested selector against its parent, SCSS-style (`&` or descendant). */
export function resolveSelector(parent, child) {
    const children = child.split(",").map((s) => s.trim()).filter(Boolean);
    if (!parent) return children.join(", ");
    const parents = parent.split(",").map((s) => s.trim());
    const out = [];
    for (const p of parents) {
        for (const c of children) out.push(c.includes("&") ? c.replace(/&/g, p) : `${p} ${c}`);
    }
    return out.join(", ");
}

/**
 * Every declaration in a stylesheet, with the resolved selector it sits
 * under. At-rules (@media, @container, @include with a block) don't add to
 * the selector; @mixin and @function bodies are skipped, since their
 * selectors only mean something where they are included. Checks that don't
 * look at the selector (the scale ratchets) pass `{ includeMixins: true }`,
 * so a raw value written inside a mixin counts too.
 */
export function declarations(scss, { includeMixins = false } = {}) {
    const text = stripComments(scss);
    const stack = []; // { selector, skip, conditional }
    const out = [];
    let buf = "";
    let line = 1;
    let bufLine = 1;
    let interpolation = 0; // depth inside `#{…}`, which is not a block
    for (const c of text) {
        if (c === "\n") line++;
        if (c === "{" && buf.endsWith("#")) {
            interpolation++;
            buf += c;
        } else if (c === "}" && interpolation > 0) {
            interpolation--;
            buf += c;
        } else if (c === "{") {
            const head = buf.trim();
            const parent = stack.length ? stack[stack.length - 1] : { selector: "", skip: false, conditional: false };
            const skip = parent.skip || (!includeMixins && /^@(mixin|function)\b/.test(head));
            const selector = head.startsWith("@") ? parent.selector : resolveSelector(parent.selector, head);
            // Inside @media, @include, @if and the like; @layer always applies.
            const conditional = parent.conditional || (head.startsWith("@") && !/^@layer\b/.test(head));
            stack.push({ selector, skip, conditional });
            buf = "";
            bufLine = line;
        } else if (c === "}") {
            flush();
            stack.pop();
            buf = "";
            bufLine = line;
        } else if (c === ";") {
            flush();
            buf = "";
            bufLine = line;
        } else {
            if (!buf.trim()) bufLine = line;
            buf += c;
        }
    }
    return out;

    function flush() {
        const decl = buf.trim();
        const top = stack[stack.length - 1];
        if (!top || top.skip) return;
        const m = /^([a-z0-9-]+)\s*:\s*([\s\S]+)$/i.exec(decl);
        if (!m) return;
        out.push({
            selector: top.selector,
            property: m[1].toLowerCase(),
            value: m[2].trim(),
            line: bufLine,
            conditional: top.conditional,
        });
    }
}

// The token alone, or with a fallback: `var(--error-color, #f38ba8)`.
const SOLID_FILL = /^var\(--(accent|error|success)-color(\s*,[^)]*)?\)\s*(!important)?$/;
const CONTROL_SELECTOR =
    /(btn|button|tab|toggle)\b|is-active|is-selected|--active\b|--on\b|--primary\b|--confirm\b|--destructive\b|--danger\b|\.solid\b|\.primary\b/i;

/** True when every selector in the list targets a pseudo-element (an underline, a marker), not the control. */
const onlyPseudoElements = (selector) => selector.split(",").every((s) => /::?(after|before)\s*$/.test(s.trim()));

export function solidFills(scss) {
    return declarations(scss)
        .filter((d) => (d.property === "background" || d.property === "background-color") && SOLID_FILL.test(d.value))
        .filter((d) => CONTROL_SELECTOR.test(d.selector) && !onlyPseudoElements(d.selector))
        .map((d) => d.selector.replace(/\s+/g, " "));
}

const RADIUS_OK = /^(0|0px|50%|inherit|none|initial|unset|var\(--radius-[a-z]+\))$/;

export function badRadii(scss) {
    return declarations(scss)
        .filter((d) => /^border(-(top|bottom)-(left|right))?-radius$/.test(d.property))
        .filter((d) => {
            const value = d.value.replace(/\s*!important$/, "");
            return !value.split(/\s+/).every((part) => RADIUS_OK.test(part));
        });
}

// ── Rule 4: undefined custom properties ─────────────────────────────────────

export function definedVars(source, kind) {
    const names = new Set();
    if (kind === "style") {
        for (const m of source.matchAll(/(--[\w-]+)\s*:/g)) names.add(m[1]);
    } else {
        // Set from script: style keys, setProperty calls, string constants.
        for (const m of source.matchAll(/["'`](--[\w-]+)["'`]/g)) names.add(m[1]);
    }
    return names;
}

/**
 * Drop block comments and whole-line `//` comments from TS/TSX. Not
 * quote-aware on purpose: an apostrophe in JSX text would throw a
 * quote-tracking stripper off, and a doc comment that mentions
 * `var(--x, …)` must not count as a use.
 */
export function stripScriptComments(source) {
    return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^[ \t]*\/\/.*$/gm, "");
}

export function usedVars(source) {
    const names = new Set();
    // Not a name built in a template literal, `var(--conn-icon-color-${n})`.
    for (const m of source.matchAll(/var\(\s*(--[\w-]+)(?![\w-]|\$\{)/g)) names.add(m[1]);
    return names;
}

// ── Rules 5-8: the shared scales ────────────────────────────────────────────

// An optional sign, for a negative delay (`-120ms`); the look-behind still
// rejects a time inside a name such as `tab-1s`.
const RAW_TIME = /(?<![\w.-])-?\d*\.?\d+m?s\b/g;
const unimportant = (value) => value.replace(/\s*!important$/, "");

export function rawDurations(scss) {
    return declarations(scss, { includeMixins: true })
        .filter((d) => /^(transition|animation)(-duration|-delay)?$/.test(d.property))
        .reduce((n, d) => n + (d.value.match(RAW_TIME) ?? []).length, 0);
}

export function rawFontSizes(scss) {
    return declarations(scss, { includeMixins: true }).filter((d) => d.property === "font-size" && /^\d*\.?\d+(px|rem)$/.test(unimportant(d.value)));
}

// Functions are the ones that take literal channels; `rgb(from var(--x) …)` and
// `rgba(var(--x-rgb), 0.2)` are built from a token and are skipped.
const COLOUR_LITERAL =
    /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\((?!\s*(?:var\(|from\b))[^)]*\)/gi;

const NAMED_COLOURS = (
    "aliceblue antiquewhite aqua aquamarine azure beige bisque blanchedalmond blue blueviolet brown burlywood cadetblue " +
    "chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen " +
    "darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue " +
    "darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue firebrick " +
    "floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink " +
    "indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan " +
    "lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray " +
    "lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue " +
    "mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred " +
    "midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid " +
    "palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple " +
    "rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue " +
    "slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat " +
    "whitesmoke yellow yellowgreen"
).split(" ");
const NAMED_COLOUR = new RegExp(`(?<![\\w-])(?:${NAMED_COLOURS.join("|")})(?![\\w-])`, "gi");
// Properties whose value can be a colour. Others (`font-family: tan`, a `grid-area` name) are not looked at.
const COLOUR_PROPERTY =
    /^(?:--.+|color|fill|stroke|caret-color|accent-color|scrollbar-color|background(?:-color|-image)?|border(?:-(?:top|right|bottom|left))?(?:-color)?|outline(?:-color)?|text-decoration(?:-color)?|column-rule(?:-color)?|box-shadow|text-shadow)$/;

/** The named colours in a value, leaving out names inside `var(…)`, `url(…)` and strings. */
function namedColours(property, value) {
    if (!COLOUR_PROPERTY.test(property)) return [];
    const bare = value.replace(/var\([^)]*\)|url\([^)]*\)|"[^"]*"|'[^']*'/g, " ");
    return bare.match(NAMED_COLOUR) ?? [];
}

/** Pure black or white, with or without alpha: a shadow or a scrim, the same on every theme. */
function isNeutral(literal) {
    if (literal.startsWith("#")) return /^#(?:0{3,8}|f{3}|f{6}|f{8})$/i.test(literal) || /^#(?:000|fff|000000|ffffff)[0-9a-f]{1,2}$/i.test(literal);
    const [r, g, b] = (literal.match(/\d*\.?\d+%?/g) ?? []).slice(0, 3);
    return (r === "0" && g === "0" && b === "0") || (r === "255" && g === "255" && b === "255");
}

/**
 * Lines a stylesheet has marked as meant to be a fixed colour, the way stylelint
 * is already told: `stylelint-disable color-no-hex` … `stylelint-enable` around
 * a block (vendor swatches, the macOS traffic lights), or `-disable-line` /
 * `-disable-next-line` on one.
 */
function allowedColourLines(scss) {
    const allowed = new Set();
    let open = false;
    scss.split("\n").forEach((text, i) => {
        const n = i + 1;
        if (/stylelint-enable.*color-no-hex/.test(text)) open = false;
        if (open) allowed.add(n);
        if (/stylelint-disable-line.*color-no-hex/.test(text)) allowed.add(n);
        if (/stylelint-disable-next-line.*color-no-hex/.test(text)) allowed.add(n + 1);
        if (/stylelint-disable(?!-).*color-no-hex/.test(text)) {
            open = true;
            allowed.add(n);
        }
    });
    return allowed;
}

/**
 * Colour literals in a stylesheet's declarations, other than pure black and
 * white, a palette token defined on `:root` (that is where a colour is set),
 * and the lines marked fixed above.
 */
export function colourLiterals(scss) {
    const allowed = allowedColourLines(scss);
    return declarations(scss, { includeMixins: true })
        .filter((d) => !allowed.has(d.line) && !(d.property.startsWith("--") && d.selector.includes(":root")))
        .flatMap((d) =>
            [...(d.value.match(COLOUR_LITERAL) ?? []).filter((literal) => !isNeutral(literal)), ...namedColours(d.property, d.value)].map(
                (literal) => ({ selector: d.selector, property: d.property, literal, line: d.line })
            )
        );
}

// A hex colour (3, 6 or 8 digits: not an issue number like `#1057`, which has 4) or a colour
// function whose channels are literal numbers (`rgba(${r}, ${g}, 0)` and `rgb(from var(--x) r g b)`
// are built from something else), in a script's code.
const TS_COLOUR = /(?<![\w&/#-])#(?:[0-9a-f]{8}|[0-9a-f]{6}|[0-9a-f]{3})\b(?![\w-])|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch)\((?=\s*[\d.])[^)$]*\)/gi;

const NATIVE_TITLE_OK = new Set(["option", "optgroup", "iframe"]);

/** Elements a user operates: a title here is a hover label on a control, where `IconButton`'s tooltip belongs. */
const INTERACTIVE = new Set(["button", "a", "input", "select", "textarea", "summary"]);

/** How many native titles are on a control (see INTERACTIVE) rather than on text. */
export function controlTitleCount(source) {
    return nativeTitleElements(source).filter((t) => INTERACTIVE.has(t.name)).length;
}

/** Skip a quoted string or template literal starting at `i` (the opening quote); returns the index after it. */
function skipString(code, i) {
    const quote = code[i];
    for (let j = i + 1; j < code.length; j++) {
        if (code[j] === "\\") j++;
        else if (code[j] === quote) return j + 1;
    }
    return code.length;
}

/**
 * `title=` attributes on lowercase JSX elements in a .tsx file: the line of each. A small scanner,
 * not a parser: it finds a tag opening (`<div`, not a generic `Array<string>`), walks its attributes
 * with `{}` nesting and strings, and notes a top-level `title` followed by `=`. Elements named in
 * NATIVE_TITLE_OK and those carrying `data-native-title` are left out.
 */
export function nativeTitleAttributes(source) {
    return nativeTitleElements(source).map((t) => t.line);
}

/** The elements that carry a native title, in order: `{ line, name }` (see `nativeTitleAttributes`). */
export function nativeTitleElements(source) {
    const code = stripScriptComments(source);
    const out = [];
    for (let i = 0; i < code.length; i++) {
        if (code[i] !== "<" || !/[a-z]/.test(code[i + 1] ?? "") || /[\w)\]]/.test(code[i - 1] ?? " ")) continue;
        const name = /^[a-z][\w.-]*/.exec(code.slice(i + 1))[0];
        let j = i + 1 + name.length;
        let depth = 0;
        const found = [];
        let native = false;
        for (; j < code.length; j++) {
            const c = code[j];
            if (c === '"' || c === "'" || c === "`") j = skipString(code, j) - 1;
            else if (c === "{") depth++;
            else if (c === "}") depth--;
            else if (c === ">" && depth === 0) break;
            else if (depth === 0 && /\s/.test(code[j - 1] ?? "") && /[a-zA-Z]/.test(c)) {
                const attr = /^[\w:-]+/.exec(code.slice(j))[0];
                if (attr === "title" && /^\s*=/.test(code.slice(j + attr.length))) found.push(j);
                if (attr === "data-native-title") native = true;
                j += attr.length - 1;
            }
        }
        if (!native && !NATIVE_TITLE_OK.has(name)) for (const at of found) out.push({ line: code.slice(0, at).split("\n").length, name });
        i = j;
    }
    return out;
}

const TEXT_INPUT_TYPES = new Set(["", "text", "search", "email", "url", "tel", "password", "number"]);

/**
 * Hand-written text fields in a .tsx file: the line of each `<textarea` and each `<input` whose type is
 * text-like (none, or a literal in TEXT_INPUT_TYPES; a type given by an expression counts, as it can be
 * text). An element carrying `data-raw-text-field` is left out. The same small scanner as
 * nativeTitleAttributes.
 */
export function rawTextFields(source) {
    const code = stripScriptComments(source);
    const out = [];
    for (let i = 0; i < code.length; i++) {
        if (code[i] !== "<" || !/[a-z]/.test(code[i + 1] ?? "") || /[\w)\]]/.test(code[i - 1] ?? " ")) continue;
        const name = /^[a-z][\w.-]*/.exec(code.slice(i + 1))[0];
        let j = i + 1 + name.length;
        if (name !== "input" && name !== "textarea") {
            i = j - 1;
            continue;
        }
        let depth = 0;
        let type = "";
        let marked = false;
        for (; j < code.length; j++) {
            const c = code[j];
            if (c === '"' || c === "'" || c === "`") j = skipString(code, j) - 1;
            else if (c === "{") depth++;
            else if (c === "}") depth--;
            else if (c === ">" && depth === 0) break;
            else if (depth === 0 && /\s/.test(code[j - 1] ?? "") && /[a-zA-Z]/.test(c)) {
                const attr = /^[\w:-]+/.exec(code.slice(j))[0];
                if (attr === "data-raw-text-field") marked = true;
                if (attr === "type") {
                    const literal = /^type\s*=\s*(?:"([^"]*)"|'([^']*)'|\{\s*(?:"([^"]*)"|'([^']*)')\s*\})/.exec(code.slice(j));
                    type = literal ? (literal[1] ?? literal[2] ?? literal[3] ?? literal[4] ?? "") : "expression";
                }
                j += attr.length - 1;
            }
        }
        const text = name === "textarea" || type === "expression" || TEXT_INPUT_TYPES.has(type.toLowerCase());
        if (text && !marked) out.push(code.slice(0, i).split("\n").length);
        i = j;
    }
    return out;
}

/**
 * Colour literals in a script's code, other than pure black and white, comments, and lines the
 * script marks `colour: fixed` (on the line, or on a comment line just above it).
 */
export function tsColourLiterals(source) {
    // Comments blanked, line numbers kept, so the markers below still point at the right line.
    const code = source.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, "")).replace(/^[ \t]*\/\/.*$/gm, "");
    const marked = new Set();
    source.split("\n").forEach((text, i) => {
        if (!/colour:\s*fixed/.test(text)) return;
        marked.add(i + 1);
        if (/^\s*(\/\/|\*|\/\*)/.test(text)) marked.add(i + 2);
    });
    const out = [];
    code.split("\n").forEach((text, i) => {
        if (marked.has(i + 1)) return;
        for (const literal of text.replace(/\s\/\/\s.*$/, "").match(TS_COLOUR) ?? []) if (!isNeutral(literal)) out.push({ line: i + 1, literal });
    });
    return out;
}

export function rawZIndexes(scss) {
    return declarations(scss, { includeMixins: true }).filter((d) => d.property === "z-index" && /^-?\d+$/.test(unimportant(d.value)));
}

/** `:focus-visible` rules (by resolved selector) that set an outline or box-shadow of their own. */
export function handFocusRings(scss) {
    const ring = (d) =>
        (d.property.startsWith("outline") || d.property === "box-shadow") && !/^(none|0)$/.test(unimportant(d.value));
    const rules = declarations(scss).filter((d) => d.selector.includes(":focus-visible") && ring(d));
    return [...new Set(rules.map((d) => d.selector))];
}

const TAG_CLASS = /\.[\w-]*(?:badge|chip|tag|pill|ribbon)$/i;

/**
 * Rules (by resolved selector) that draw a tag by hand: the class ends in badge, chip, tag, pill
 * or ribbon, and the rule declares a padding, a font-size and a border-radius or a background.
 * A declaration anywhere in the file for the same selector counts toward it. A `// tag: fixed`
 * comment from three lines above a rule's first declaration to its last excuses it.
 */
export function handRolledTags(scss) {
    const byRule = new Map();
    for (const d of declarations(scss)) {
        if (!d.selector.split(",").some((sel) => TAG_CLASS.test(sel.trim()))) continue;
        const rule = byRule.get(d.selector) ?? { props: new Set(), first: d.line, last: d.line };
        rule.props.add(d.property.replace(/-(top|right|bottom|left|inline|block)(-(start|end))?$/, "").replace(/^background-.*/, "background"));
        rule.first = Math.min(rule.first, d.line);
        rule.last = Math.max(rule.last, d.line);
        byRule.set(d.selector, rule);
    }
    const marks = [];
    scss.split("\n").forEach((text, i) => {
        if (/tag:\s*fixed/.test(text)) marks.push(i + 1);
    });
    return [...byRule]
        .filter(([, r]) => r.props.has("padding") && r.props.has("font-size") && (r.props.has("border-radius") || r.props.has("background")))
        .filter(([, r]) => !marks.some((m) => m >= r.first - 3 && m <= r.last))
        .map(([selector]) => selector);
}

/** Custom properties set on `:root` itself (alone or in a list), outside any at-rule: defined in every element. */
export function rootTokens(scss) {
    const onRoot = (selector) => selector.split(",").some((s) => s.trim() === ":root");
    return declarations(scss)
        .filter((d) => onRoot(d.selector) && !d.conditional && d.property.startsWith("--"))
        .map((d) => d.property);
}

/** `var(--x, …)` uses whose `--x` is one of `tokens`, so the fallback never applies. */
export function deadFallbacks(scss, tokens) {
    let n = 0;
    for (const m of stripComments(scss).matchAll(/var\(\s*(--[\w-]+)\s*,/g)) if (tokens.has(m[1])) n++;
    return n;
}

// ── Collect ─────────────────────────────────────────────────────────────────

const THEME_FILE = /^frontend\/(tailwindsetup\.css|app\/(theme\.scss|themes\/))/;

export function collect(files) {
    const buttons = {};
    const fills = new Set();
    const radius = {};
    const defined = new Set();
    const used = new Set();
    const durations = {};
    const fontSizes = {};
    const zIndex = {};
    const focusRings = {};
    const colours = {};
    const tsColours = {};
    const tags = {};
    const nativeTitles = {};
    const textFields = {};
    const controlTitles = {};
    const menus = {};
    const styles = [];

    for (const { path, source } of files) {
        const isStyle = /\.(s?css)$/.test(path);
        const isScript = /\.(tsx?)$/.test(path);
        if (!isStyle && !isScript) continue;
        if (isTest(path)) continue;

        for (const name of definedVars(source, isStyle ? "style" : "script")) defined.add(name);
        for (const name of usedVars(isStyle ? stripComments(source) : stripScriptComments(source))) used.add(name);

        if (isScript && !path.endsWith(".d.ts")) countInto(tsColours, path, tsColourLiterals(source).length);
        if (path.endsWith(".tsx") && !path.startsWith(UI_DIR)) {
            const n = countButtons(source);
            if (n > 0) buttons[path] = n;
            countInto(nativeTitles, path, nativeTitleAttributes(source).length);
            countInto(textFields, path, rawTextFields(source).length);
            countInto(controlTitles, path, controlTitleCount(source));
            countInto(menus, path, countMenus(source));
        }
        if (isStyle) {
            for (const selector of solidFills(source)) fills.add(`${path} :: ${selector}`);
            const n = badRadii(source).length;
            if (n > 0) radius[path] = n;
            styles.push({ path, source });
            countInto(durations, path, rawDurations(source));
            countInto(fontSizes, path, rawFontSizes(source).length);
            countInto(zIndex, path, rawZIndexes(source).length);
            countInto(focusRings, path, handFocusRings(source).length);
            // The theme files are where colours are defined.
            if (!THEME_FILE.test(path)) countInto(colours, path, colourLiterals(source).length);
            if (!path.startsWith(UI_DIR)) countInto(tags, path, handRolledTags(source).length);
        }
    }

    const tokens = new Set(styles.flatMap(({ source }) => rootTokens(source)));
    const fallbacks = {};
    for (const { path, source } of styles) countInto(fallbacks, path, deadFallbacks(source, tokens));

    // `--tw-*` are Tailwind's; `--fa-*` are Font Awesome's optional hooks
    // (public/fontawesome/), always read with FA's own fallback; `--shiki-*`
    // are set inline on each highlighted run by Shiki, the `-font-style` and
    // `-font-weight` ones only when the theme styles that token.
    const undefinedVars = [...used].filter((name) => !defined.has(name) && !/^--(tw|fa|shiki)-/.test(name));
    return {
        buttons: sortObject(buttons),
        solidFills: [...fills].sort(),
        radius: sortObject(radius),
        undefinedVars: undefinedVars.sort(),
        durations: sortObject(durations),
        fontSizes: sortObject(fontSizes),
        fallbacks: sortObject(fallbacks),
        zIndex: sortObject(zIndex),
        focusRings: sortObject(focusRings),
        colours: sortObject(colours),
        tsColours: sortObject(tsColours),
        tags: sortObject(tags),
        nativeTitles: sortObject(nativeTitles),
        textFields: sortObject(textFields),
        controlTitles: sortObject(controlTitles),
        menus: sortObject(menus),
    };
}

function countInto(counts, path, n) {
    if (n > 0) counts[path] = n;
}

function sortObject(obj) {
    return Object.fromEntries(Object.entries(obj).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
}

// ── Compare ─────────────────────────────────────────────────────────────────

const PER_FILE = ["buttons", "radius", "durations", "fontSizes", "fallbacks", "zIndex", "focusRings", "colours", "tsColours", "tags", "nativeTitles", "textFields", "controlTitles", "menus"];

/** How `current` differs from `baseline` (the base branch), as human-readable lines. */
export function compare(current, baseline) {
    const grew = [];
    const shrank = [];

    for (const key of PER_FILE) {
        const files = new Set([...Object.keys(current[key] ?? {}), ...Object.keys(baseline[key] ?? {})]);
        for (const file of [...files].sort()) {
            const now = current[key]?.[file] ?? 0;
            const was = baseline[key]?.[file] ?? 0;
            if (now > was) grew.push(`${key}: ${file} has ${now}, base ${was}`);
            else if (now < was) shrank.push(`${key}: ${file} has ${now}, base ${was}`);
        }
    }
    for (const key of ["solidFills", "undefinedVars"]) {
        const was = new Set(baseline[key] ?? []);
        const now = new Set(current[key]);
        for (const item of now) if (!was.has(item)) grew.push(`${key}: new ${item}`);
        for (const item of was) if (!now.has(item)) shrank.push(`${key}: gone ${item}`);
    }
    return { grew, shrank };
}

// ── The base branch ─────────────────────────────────────────────────────────

function git(args, input) {
    return spawnSync("git", args, { cwd: REPO_ROOT, input, maxBuffer: 1 << 30 });
}

/** The commit to compare against, or null when it can't be resolved (shallow clone, no remote). */
function baseCommit() {
    const branch = process.env.GITHUB_BASE_REF || "main";
    const rev = (ref) => {
        const r = git(["rev-parse", "--verify", "--quiet", ref]);
        return r.status === 0 ? r.stdout.toString().trim() : null;
    };
    // Set by ci-pr.yml on a push: the commit the branch pointed to before it.
    const before = process.env.UI_RATCHET_BEFORE?.trim();
    const pushedFrom = before && !/^0+$/.test(before) ? rev(`${before}^{commit}`) : null;
    for (const ref of [`origin/${branch}`, branch]) {
        const mb = git(["merge-base", "HEAD", ref]);
        if (mb.status === 0) return pickBase(mb.stdout.toString().trim(), rev("HEAD"), pushedFrom ?? rev("HEAD^"));
    }
    return null;
}

/**
 * The commit to compare against, given the merge-base with the base branch.
 * On a pull request that is where the branch left main. On a push to main
 * itself the merge-base is HEAD, which would compare the tree with itself and
 * never fail, so `previous` is used instead: the push event's `before` commit,
 * which covers every commit in the push, or failing that HEAD's first parent.
 */
export function pickBase(mergeBase, head, previous) {
    if (mergeBase !== head) return mergeBase;
    return previous;
}

/** Parse `git cat-file --batch` output into the contents of each blob, in order. */
export function parseCatFileBatch(buf) {
    const out = [];
    let i = 0;
    while (i < buf.length) {
        const nl = buf.indexOf(10, i);
        const header = buf.subarray(i, nl).toString();
        i = nl + 1;
        const m = /^\S+ (\S+) (\d+)$/.exec(header);
        if (!m) {
            out.push(null); // "<name> missing"
            continue;
        }
        const size = Number(m[2]);
        out.push(buf.subarray(i, i + size).toString("utf8"));
        i += size + 1; // contents, then a newline
    }
    return out;
}

/** The frontend's style and script files as they are at `ref`. */
function filesAtRef(ref) {
    const list = git(["ls-tree", "-r", "--name-only", ref, "--", "frontend"]);
    const paths = list.stdout
        .toString()
        .split("\n")
        .filter((p) => /\.(s?css|tsx?)$/.test(p));
    const batch = git(["cat-file", "--batch"], paths.map((p) => `${ref}:${p}`).join("\n") + "\n");
    const contents = parseCatFileBatch(batch.stdout);
    return paths.map((path, i) => ({ path, source: contents[i] ?? "" }));
}

// ── Main ────────────────────────────────────────────────────────────────────

if (isMain(import.meta.url)) {
    const base = baseCommit();
    if (!base) {
        console.log("check-ui-primitives: cannot resolve the base branch (shallow clone?) — skipping.");
        process.exit(0);
    }
    const current = collect(walk(FRONTEND).map((p) => ({ path: rel(p), source: readFileSync(p, "utf8") })));
    const before = collect(filesAtRef(base));
    const { grew, shrank } = compare(current, before);
    grew.unshift(...hardZeroViolations(current));

    if (grew.length > 0) {
        console.log("ERROR: new UI that bypasses the line-style component set");
        console.log("(docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md):");
        console.log();
        for (const line of grew) console.log(`  ${line}`);
        console.log();
        console.log("Use Button / IconButton / Tabs / SegmentedControl / Field / inputs from");
        console.log("frontend/app/element/ui/, a line instead of a solid fill, a --radius-* token,");
        console.log("only custom properties that are defined, --duration-*, --text-* and --z-* tokens");
        console.log("instead of raw values, no fallback on a token :root always sets, and");
        console.log("`@include mixins.focus-ring` for a focus ring, and theme tokens (--error-color,");
        console.log("--success-bg, --tint-2, …) instead of a colour literal, and Tag / TagList /");
        console.log("FilterChip instead of a hand-drawn badge, chip, pill or ribbon, <Menu> instead of a new menu, and <Tooltip> (or an");
        console.log("IconButton tooltip) instead of a native title= hover label, and TextInput / TextArea /");
        console.log("NumberInput / FilterInput instead of a hand-written text <input> or <textarea>.");
        process.exit(1);
    }
    const total = (o) => Object.values(o).reduce((a, b) => a + b, 0);
    console.log(
        `OK: no new hand-rolled controls vs ${base.slice(0, 9)}. Now ${total(current.buttons)} hand-rolled buttons, ` +
            `${current.solidFills.length} solid fills, ${total(current.radius)} off-token radii, ` +
            `${current.undefinedVars.length} undefined variables, ${total(current.durations)} raw durations, ` +
            `${total(current.fontSizes)} raw px font sizes, ${total(current.fallbacks)} fallbacks on always-defined ` +
            `tokens, ${total(current.zIndex)} raw z-indexes, ${total(current.focusRings)} hand-written focus rings, ` +
            `${total(current.colours)} colour literals in styles, ${total(current.tsColours)} in scripts, ` +
            `${total(current.tags)} hand-drawn tags, ` +
            `${total(current.nativeTitles)} native title= labels, ${total(current.textFields)} hand-written text fields, ` +
            `${total(current.menus)} menus outside the kit's Menu.`
    );
    if (shrank.length > 0) {
        console.log(`Down from the base branch (${shrank.length}):`);
        for (const line of shrank) console.log(`  ${line}`);
    }
}
