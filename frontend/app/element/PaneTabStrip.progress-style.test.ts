// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The progress line's stylesheet rules, read as text (like drop-feedback.test.ts):
 * what it animates, how long, and where its reduced treatment and the collapsed
 * pane's hidden border live. docs/specs/SPEC_PANE_TAB_PROGRESS_LINE_2026_10_24.md §3.5, §5.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(join(here, rel), "utf8").replace(/\r\n/g, "\n");
const strip = read("PaneTabStrip.scss");
const chrome = read("PaneChrome.scss");
const keyframes = read("../_keyframes.scss");

/** The text of the first rule whose selector line contains `selector`. */
function rule(css: string, selector: string): string {
    const at = css.indexOf(selector);
    expect(at, `${selector} is in the stylesheet`).toBeGreaterThanOrEqual(0);
    const open = css.indexOf("{", at);
    let depth = 1;
    let i = open + 1;
    while (depth > 0 && i < css.length) {
        if (css[i] === "{") depth++;
        else if (css[i] === "}") depth--;
        i++;
    }
    return css.slice(open + 1, i - 1);
}

describe("the progress line's stylesheet", () => {
    it("sweeps with transform only, one pass per 1.6 s", () => {
        const sweep = rule(keyframes, "@keyframes am-sweep");
        expect(sweep).toMatch(/transform: translateX\(-100%\)/);
        expect(sweep).toMatch(/transform: translateX\(300%\)/);
        expect(sweep).not.toMatch(/\b(left|right|top|width|height|margin)\b/);
        expect(strip).toMatch(/animation: am-sweep var\(--duration-sweep\) /);
        expect(read("../theme.scss")).toMatch(/--duration-sweep: +1600ms;/); // a loop: the Animations setting does not scale it
    });

    it("is 2px high, below the pane's ring, with a 25% track", () => {
        const base = rule(strip, ".pane-tab-progress {");
        expect(base).toMatch(/top: var\(--pane-ring-width, 0px\)/);
        expect(base).toMatch(/height: 2px/);
        expect(strip).toMatch(/\[data-state="working"\] \{\s*background: color-mix\(in srgb, var\(--progress-line-color\) 25%, transparent\)/);
    });

    it("gives every state its own shape", () => {
        expect(strip).toMatch(/\[data-state="waiting"\] \{\s*border-top: 2px solid var\(--accent-color\)/);
        expect(rule(strip, '&[data-state="signed_out"]')).toContain("repeating-linear-gradient"); // dashed
        expect(strip).not.toMatch(/data-state="limited"/); // out of quota has no line
    });

    it("stops and parks the bar under the motion policy's status category, not a raw media query", () => {
        const bar = rule(strip, ".pane-tab-progress--sweeping &");
        expect(bar).toMatch(/@include mixins\.when-motion-reduced\(status\) \{\s*animation: none;/);
        expect(strip).not.toMatch(new RegExp("prefers-" + "reduced-motion")); // spelled apart so the gate script does not read this line as a query
    });

    it("hides a collapsed pane's border animation", () => {
        expect(rule(chrome, ".pane-stack-collapsed .pane-progress-bar-slot")).toMatch(/display: none/);
    });

    it("leaves the compaction bar on the shared keyframes", () => {
        const control = read("../view/agent/styles/_control-bar.scss");
        expect(control).toContain("animation: am-sweep var(--duration-sweep)");
        expect(control).not.toContain("agent-compaction-indeterminate");
    });
});
