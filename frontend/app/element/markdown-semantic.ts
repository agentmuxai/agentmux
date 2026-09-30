// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The semantic colour classes an agent may put on a `<span>` in markdown
 * (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §2). A closed set tied to meaning,
 * never free-form colour: the theme picks the colour (`markdown.scss`), and on
 * GitHub or any other renderer the span is just its text.
 *
 * Three places must agree with this list, and `markdown-semantic.test.tsx`
 * checks all three: the sanitizer allowlist (`markdown.tsx`), the stylesheet
 * (`markdown.scss`), and the Operator Config entry that tells agents about it
 * (`crates/srv/operator-config-seed.json`, id `operator-config-rich-output`).
 */
export const AM_SPAN_CLASSES = [
    "am-ok",
    "am-warn",
    "am-error",
    "am-info",
    "am-muted",
    "am-added",
    "am-removed",
    "am-badge",
] as const;
