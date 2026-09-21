// @ts-check
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A standalone, narrowly-scoped ESLint pass — deliberately separate from
// eslint.config.js's general `recommended` ruleset, which has 1200+
// pre-existing violations across the frontend (mostly @typescript-eslint/
// no-explicit-any/no-unused-vars) that are a much larger, separate cleanup
// effort. Wiring THIS rule into that config would make a CI gate meant to
// catch one specific bug class immediately fail for 1200 unrelated reasons.
// This file exists so `npx eslint --config eslint.theme-colors.config.js
// frontend/app` can run clean and mean something on its own.
//
// See docs/specs/SPEC_COLOR_THEME_TOKEN_HARDENING_2026_09_21.md for the full
// audit this rule follows from.
import tseslint from "typescript-eslint";

// Matches a raw Tailwind palette utility (its own fixed literal color,
// completely blind to this app's `[data-theme="..."]` system) applied to
// text/background/border/etc. -- `text-white`, `bg-black`, `bg-gray-800`,
// `hover:text-white`, `border-white/10`, and their `slate`/`neutral`/`zinc`/
// `stone` siblings, with an optional interaction-variant prefix and/or
// opacity modifier.
//
// This is NOT a blanket ban on the words "white"/"black"/"gray" -- a fixed
// white-on-a-fixed-dark-backdrop pane (dragoverlay.tsx) is genuinely
// theme-independent by design and gets an inline
// eslint-disable-next-line with a one-line reason instead. The point is
// that using one of these utilities is now a DELIBERATE, visible choice
// (a disable comment someone has to write and justify), not something
// that silently slips in because it's the first color-sounding class name
// that came to mind.
// The trailing opacity modifier (`/20`, `/[0.03]`) contains a literal `/`,
// which has to be escaped as `\/` here specifically because this whole
// pattern gets embedded inside esquery's own `/regex/` selector-string
// delimiters below (an UNESCAPED `/` would prematurely close esquery's
// selector, not the JS RegExp itself) -- confirmed the hard way: esquery
// threw "Unterminated group" pointing exactly at that `/` before this fix.
const FORBIDDEN_UTILITY =
    "\\b(hover:|focus:|active:|group-hover:)?(text|bg|border|ring|divide|placeholder|from|via|to|fill|stroke)-" +
    "(white|black|gray-\\d+|slate-\\d+|neutral-\\d+|zinc-\\d+|stone-\\d+)(\\/[\\d.\\[\\]]+)?\\b";

const MESSAGE =
    "Hardcoded Tailwind palette color (white/black/gray-N/etc.) in a class list. " +
    "This ignores the app's [data-theme] system entirely -- it renders identically " +
    "on every theme, which is exactly how the widget-bar text-disappears-on-light-theme " +
    "bug happened (SPEC_COLOR_THEME_TOKEN_HARDENING_2026_09_21.md). Use one of this " +
    "app's own theme-synced tokens instead (text-foreground/text-secondary/bg-hover/" +
    "bg-hoverbg/bg-highlightbg/bg-panel/bg-modalbg/border-border, etc. -- see any " +
    "theme's own \"Tailwind @theme token sync\" block in frontend/app/themes/*.scss " +
    "for the full list). If this really is meant to be the same color on every theme " +
    "(e.g. white text painted on this component's own fixed-color background), add " +
    "`// eslint-disable-next-line no-restricted-syntax -- <why this one is theme-independent>` " +
    "immediately above it.";

export default tseslint.config(
    {
        // Pre-existing, unrelated to color theming: these three files carry
        // `eslint-disable-next-line jsx-a11y/...` / `solid/no-innerhtml`
        // comments for plugins that were apparently planned but never
        // actually added as dependencies anywhere in this repo (not even in
        // the main eslint.config.js) -- referencing an unknown rule in a
        // disable comment is itself an ESLint error ("Definition for rule
        // ... was not found"), independent of and unrelated to this file's
        // own rule. Excluded here rather than fixed: adopting those plugins
        // repo-wide is a separate, unrelated effort.
        ignores: [
            "frontend/app/view/agent/components/AgentLaunchModal.tsx",
            "frontend/app/view/agent/components/AgentQuestionPanel.tsx",
            "frontend/app/view/editor/editor-view.tsx",
        ],
    },
    {
        files: ["frontend/app/**/*.tsx"],
        languageOptions: {
            parser: tseslint.parser,
            parserOptions: { ecmaFeatures: { jsx: true } },
        },
        rules: {
            "no-restricted-syntax": [
                "error",
                { selector: `Literal[value=/${FORBIDDEN_UTILITY}/]`, message: MESSAGE },
                { selector: `TemplateElement[value.raw=/${FORBIDDEN_UTILITY}/]`, message: MESSAGE },
            ],
        },
    },
);
