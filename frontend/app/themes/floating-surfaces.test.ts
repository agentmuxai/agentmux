// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Nothing that floats over a pane shows what is under it. `--main-bg-color`, `--panel-bg-color` and the other
 * pane-derived colours follow the window opacity, so a menu, popover, drop-down list, side panel or dialog that paints
 * one of them is see-through below 1. The menu family stands on the one opaque surface (`menu-surface` / `menu-fill`
 * in element/menu-frame.scss); a dialog on `--modal-bg-color`.
 *
 * The rule is found by scanning every stylesheet, not by a list of files: a rule that positions itself (absolute or
 * fixed) and is lifted (a z-index or a shadow) must not paint a pane-derived or translucent background. A new floating
 * stylesheet therefore fails here until it uses the surface. What is not a floating surface (a badge, a scrim, a
 * highlight, a sticky header) is listed in NOT_A_SURFACE with the reason; add to it only for such a thing.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import { bundledThemeIds, resolveTheme } from "@/app/element/ui/theme-test-utils";

const APP = new URL("../", import.meta.url);
const ROOTS = [APP.pathname.replace(/^\/([A-Za-z]:)/, "$1"), new URL("../../layout/", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1")];

function scssFiles(dir: string): string[] {
    const out: string[] = [];
    for (const name of readdirSync(dir)) {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) {
            if (name !== "node_modules") out.push(...scssFiles(path));
        } else if (name.endsWith(".scss")) out.push(path);
    }
    return out;
}

interface Rule {
    /** The nested selector path, "a > b > c". */
    path: string;
    /** Its own declarations (not those of nested rules), comments removed. */
    own: string;
}

/** Every rule of a stylesheet with its own declarations. Not a full Sass parser: enough for the rules we read. */
function rulesOf(scss: string): Rule[] {
    const src = scss.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "");
    const stack: { sel: string; own: string }[] = [];
    const rules: Rule[] = [];
    let buf = "";
    for (const ch of src) {
        if (ch === "{") {
            stack.push({ sel: buf.trim().replace(/\s+/g, " "), own: "" });
            buf = "";
        } else if (ch === "}") {
            if (buf.trim() && stack.length) stack[stack.length - 1].own += buf;
            buf = "";
            const top = stack.pop();
            if (top) rules.push({ path: [...stack.map((s) => s.sel), top.sel].join(" > "), own: top.own });
        } else if (ch === ";") {
            if (stack.length) stack[stack.length - 1].own += `${buf};`;
            buf = "";
        } else buf += ch;
    }
    return rules;
}

/** A background that follows the window opacity or is translucent by construction. */
const TRANSLUCENT =
    /main-bg-color|panel-bg-color|block-bg-color|attention-bg|tab-strip-bg|tint-|window-opacity|transparent|color-mix|rgba?\(|rgb\(|drop-target-bg/;

const backgroundOf = (own: string): string | undefined => own.match(/(?:^|[;\s])background(?:-color)?\s*:\s*([^;]+);/)?.[1].trim();
const isFloating = (own: string): boolean => /position\s*:\s*(absolute|fixed)/.test(own) && /box-shadow\s*:|z-index\s*:/.test(own);

/**
 * Things that are not a floating surface although they float and paint a translucent colour. [file, a piece of the
 * selector path, why]. A scrim or a tint over a pane is meant to show it through; a badge is a label, not a panel.
 */
const NOT_A_SURFACE: [file: string, selector: string, why: string][] = [
    ["app.scss", ".floating-redock-drop-placeholder", "a drop target's tint, meant to show the window under it"],
    ["drop-feedback.scss", "drop-edge-strip", "a gradient strip marking a drop target at a pane's edge, not a panel"],
    ["element/blockstats.scss", ".block-stats-badge", "a dim readout label in a pane's corner, on its own tint, not a panel"],
    ["element/markdown.scss", ".am-media-enlarge", "an icon button over an image, tinted so it reads on any picture"],
    ["view/agent/agent-view.scss", ".pane-tab-strip", "pane chrome: the tab strip along a pane's edge, translucent on purpose"],
    ["view/agent/styles/_document-nodes.scss", ".agent-tool-panel-actions", "quiet icon buttons shown at 55% until the pointer is on the preview, by design"],
    ["view/drone/drone-view.scss", ".drone-node", "a node drawn on the canvas and moved with it, not floating over content"],
    ["view/term/term.scss", ".agent-runtime-badge", "a label badge on the terminal, not a panel"],
];

describe("nothing that floats over a pane is see-through", () => {
    const found: { file: string; path: string; bg: string }[] = [];
    for (const root of ROOTS) {
        for (const file of scssFiles(root)) {
            for (const rule of rulesOf(readFileSync(file, "utf8"))) {
                const bg = backgroundOf(rule.own);
                if (bg && isFloating(rule.own) && TRANSLUCENT.test(bg)) found.push({ file: relative(root, file).replace(/\\/g, "/"), path: rule.path, bg });
            }
        }
    }

    const allowed = (f: { file: string; path: string }) => NOT_A_SURFACE.some(([file, sel]) => f.file.endsWith(file) && f.path.includes(sel));

    it("finds no floating rule that paints a pane-derived or translucent background", () => {
        const bad = found.filter((f) => !allowed(f)).map((f) => `${f.file}: ${f.path} -> ${f.bg}`);
        expect(bad).toEqual([]);
    });

    it("has a reason for every exemption, and no exemption that no longer matches", () => {
        for (const [file, sel, why] of NOT_A_SURFACE) {
            expect(why.length, `${file} ${sel} needs a reason`).toBeGreaterThan(10);
            expect(found.some((f) => f.file.endsWith(file) && f.path.includes(sel)), `${file} ${sel} is still floating and translucent`).toBe(true);
        }
    });

    it("finds a rule that floats on the pane's colour (sensitivity)", () => {
        const rules = rulesOf(".x { position: absolute; z-index: 5; background: var(--main-bg-color); } .y { position: absolute; z-index: 5; @include m.fill; }");
        expect(rules.filter((r) => isFloating(r.own) && TRANSLUCENT.test(backgroundOf(r.own) ?? "")).map((r) => r.path)).toEqual([".x"]);
    });
});

describe("the dialog colour is opaque on every theme", () => {
    it.each(["default", ...bundledThemeIds()])("--modal-bg-color on %s is a literal colour that ignores the window opacity", (id) => {
        const raw = resolveTheme(id).raw("modal-bg-color") ?? "";
        expect(raw).toMatch(/^\s*(#[0-9a-f]{6}|rgb\(\s*\d+\s*,\s*\d+\s*,\s*\d+\s*\))\s*$/i);
    });
});
