// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-colour-surfaces.mjs, and the gate itself: the repo's stylesheets must be clean.
// Spec: docs/specs/SPEC_PICKED_COLOUR_IS_THE_COLOUR_SEEN_2026_11_18.md §7.

import { describe, expect, it } from "vitest";
import { ALLOW, checkFile, collect, declarations } from "./check-colour-surfaces.mjs";

const found = (css) => checkFile("x.scss", css).map((v) => `${v.property}:${v.selector}`);

describe("declarations", () => {
    it("flattens nested selectors, & and comma lists", () => {
        const css = ".tab { &.active { .inner { color: red; } } &:hover, &.dragging { opacity: 1; } }";
        const d = declarations(css);
        expect(d.find((x) => x.property === "color").selectors).toEqual([".tab.active .inner"]);
        expect(d.find((x) => x.property === "opacity").selectors).toEqual([".tab:hover", ".tab.dragging"]);
    });

    it("passes at-rules through and skips comments, keeping line numbers", () => {
        const css = "// a // b\n@media (min-width: 1px) {\n  .a { /* filter: blur(1px); */\n    width: 1px;\n  }\n}\n";
        const d = declarations(css);
        expect(d).toHaveLength(1);
        expect(d[0]).toMatchObject({ property: "width", selectors: [".a"], line: 4 });
    });
});

describe("checkFile", () => {
    it("flags a filter, backdrop-filter, blend mode or opacity on a coloured tab, nested or not", () => {
        expect(found(".tab.tab-colored:hover .tab-inner { filter: brightness(1.1); }")).toEqual(["filter:.tab.tab-colored:hover .tab-inner"]);
        expect(found(".tab { &.tab-colored { mix-blend-mode: multiply; opacity: .5; backdrop-filter: blur(2px); } }")).toHaveLength(3);
        expect(found(".pane-tab--colored { opacity: 0.5; }")).toEqual(["opacity:.pane-tab--colored"]);
        expect(found(".palette-swatch { filter: none; }")).toEqual(["filter:.palette-swatch"]);
    });

    it("flags !important on a coloured tab's fill, but not on another property or an uncoloured element", () => {
        expect(found(".tab.tab-colored .tab-inner { background: red !important; }")).toHaveLength(1);
        expect(found(".tab.tab-colored .tab-inner { color: red !important; }")).toEqual([]);
        expect(found(".tab:not(.tab-colored) .tab-inner { background: red !important; filter: none; opacity: 1; }")).toEqual([]);
    });

    it("leaves an element that is not user-coloured alone", () => {
        expect(found(".block-frame { filter: blur(1px); opacity: .5; backdrop-filter: blur(4px); }")).toEqual([]);
        expect(found(".palette-swatches { opacity: .5; }")).toEqual([]);
    });
});

describe("the repository's stylesheets", () => {
    it("have no filter, opacity or blend over a user-coloured element, and ALLOW has no stale entry", () => {
        const { violations, stale } = collect();
        expect(violations.map((v) => `${v.file}:${v.line} ${v.why} (${v.selector})`)).toEqual([]);
        expect(stale.map((a) => `${a.file} ${a.selector} ${a.property}`)).toEqual([]);
    });

    it("keeps every ALLOW entry with a reason", () => {
        for (const a of ALLOW) expect(a.reason.length).toBeGreaterThan(10);
    });
});
