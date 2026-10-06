// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-ui-primitives.mjs.
// Spec: docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §7.

import { describe, expect, it } from "vitest";
import {
    badRadii,
    collect,
    compare,
    countButtons,
    declarations,
    parseCatFileBatch,
    pickBase,
    resolveSelector,
    solidFills,
    stripComments,
} from "./check-ui-primitives.mjs";

describe("SCSS walking", () => {
    it("resolves nesting with & and descendant selectors, across comma lists", () => {
        expect(resolveSelector("", ".a")).toBe(".a");
        expect(resolveSelector(".a", "&.is-active")).toBe(".a.is-active");
        expect(resolveSelector(".a", ".b")).toBe(".a .b");
        expect(resolveSelector(".a, .b", "&:hover")).toBe(".a:hover, .b:hover");
    });

    it("strips comments but keeps URLs and strings", () => {
        const out = stripComments('a { b: url(https://x/y); } // gone\n/* gone */ c { content: "//kept"; }');
        expect(out).toContain("https://x/y");
        expect(out).toContain('"//kept"');
        expect(out).not.toContain("gone");
    });

    it("attributes declarations to their resolved selector; at-rules add nothing, mixins are skipped", () => {
        const scss = `
            .btn {
                color: red;
                @container x (max-width: 10px) {
                    &.is-active { background: blue; }
                }
            }
            @mixin m { .inside { background: var(--accent-color); } }
            .w-#{$name} { width: 1px; }
        `;
        const decls = declarations(scss).map((d) => `${d.selector} | ${d.property}`);
        expect(decls).toEqual([".btn | color", ".btn.is-active | background", ".w-#{$name} | width"]);
    });
});

describe("rule 2: solid fills on controls", () => {
    it("flags a solid accent, error or success background on a control, with or without a fallback", () => {
        const scss = `
            .save-btn { background: var(--accent-color); }
            .rail-item.is-active { background-color: var(--error-color, #f00); }
            .toggle--on { background: var(--success-color) !important; }
        `;
        expect(solidFills(scss)).toEqual([".save-btn", ".rail-item.is-active", ".toggle--on"]);
    });

    it("ignores tints, non-controls and pseudo-element underlines", () => {
        const scss = `
            .save-btn { background: color-mix(in srgb, var(--accent-color) 12%, transparent); }
            .status-dot { background: var(--accent-color); }
            .tab.active .tab-inner::after { background: var(--accent-color); }
        `;
        expect(solidFills(scss)).toEqual([]);
    });
});

describe("rule 3: corner radii", () => {
    it("accepts 0, radius tokens, 50% and combinations of them", () => {
        const scss = `.a { border-radius: 0; } .b { border-radius: var(--radius-full); } .c { border-radius: 0 0 var(--radius-sm) 50%; }`;
        expect(badRadii(scss)).toEqual([]);
    });

    it("flags raw pixel radii", () => {
        const scss = `.a { border-radius: 4px; } .b { border-top-left-radius: 6px; }`;
        expect(badRadii(scss).map((d) => d.selector)).toEqual([".a", ".b"]);
    });
});

describe("collect", () => {
    const files = [
        { path: "frontend/app/view/x/x.tsx", source: '<button class="a" /><button class="b" />' },
        { path: "frontend/app/element/ui/Button.tsx", source: "<button />" },
        { path: "frontend/app/view/x/x.test.tsx", source: '<button />; el.style.setProperty("--set-in-test", "1")' },
        { path: "frontend/app/view/x/x.scss", source: ".x { color: var(--defined); background: var(--missing, red); }" },
        { path: "frontend/app/theme.scss", source: ":root { --defined: 1; }" },
        { path: "frontend/app/view/x/y.tsx", source: 'style={{ "--from-script": "1" }} class="z" />; css(`var(--from-script)`)' },
    ];

    it("counts buttons outside element/ui and outside tests", () => {
        expect(collect(files).buttons).toEqual({ "frontend/app/view/x/x.tsx": 2 });
    });

    it("finds variables defined nowhere, counting ones set from script as defined", () => {
        expect(collect(files).undefinedVars).toEqual(["--missing"]);
    });

    it("ignores variables mentioned only in script comments", () => {
        const commented = [
            ...files,
            { path: "frontend/app/view/x/c.ts", source: '/** falls back via `var(--doc, x)` */\n// var(--line)\nconst s = "var(--real)";' },
        ];
        expect(collect(commented).undefinedVars).toEqual(["--missing", "--real"]);
    });

    it("doesn't count a variable set only by a test as defined", () => {
        const withUse = [...files, { path: "frontend/app/view/x/z.scss", source: ".z { color: var(--set-in-test); }" }];
        expect(collect(withUse).undefinedVars).toEqual(["--missing", "--set-in-test"]);
    });

    it("counts each <button", () => {
        expect(countButtons("<button>\n<button\n  type='x'>\n<buttons>")).toBe(2);
    });
});

describe("compare", () => {
    const baseline = {
        buttons: { "a.tsx": 2 },
        radius: {},
        solidFills: ["x.scss :: .old-btn"],
        undefinedVars: ["--old"],
    };

    it("is clean at baseline", () => {
        expect(compare(structuredClone(baseline), baseline)).toEqual({ grew: [], shrank: [] });
    });

    it("reports growth, including in a file that was clean", () => {
        const current = { ...structuredClone(baseline), buttons: { "a.tsx": 3, "b.tsx": 1 } };
        expect(compare(current, baseline).grew).toEqual(["buttons: a.tsx has 3, base 2", "buttons: b.tsx has 1, base 0"]);
    });

    it("reports shrinkage separately from growth", () => {
        const current = { buttons: {}, radius: {}, solidFills: [], undefinedVars: [] };
        const { grew, shrank } = compare(current, baseline);
        expect(grew).toEqual([]);
        expect(shrank).toEqual(["buttons: a.tsx has 0, base 2", "solidFills: gone x.scss :: .old-btn", "undefinedVars: gone --old"]);
    });
});

describe("parseCatFileBatch", () => {
    it("splits blobs by their declared size, including newlines and multi-byte text", () => {
        const a = "line one\nline two\n";
        const b = "é ✓";
        const out = Buffer.concat([
            Buffer.from(`abc123 blob ${Buffer.byteLength(a)}\n${a}\n`),
            Buffer.from("frontend/missing.tsx missing\n"),
            Buffer.from(`def456 blob ${Buffer.byteLength(b)}\n${b}\n`),
        ]);
        expect(parseCatFileBatch(out)).toEqual([a, null, b]);
    });
});

describe("pickBase", () => {
    it("uses the merge-base on a pull request", () => {
        expect(pickBase("mb", "head", "parent")).toBe("mb");
    });

    it("uses the previous commit on a push to the base branch, where the merge-base is HEAD", () => {
        // `previous` is the push's `before` commit when CI provides it, else HEAD^.
        expect(pickBase("head", "head", "before")).toBe("before");
    });

    it("has nothing to compare against for a root commit", () => {
        expect(pickBase("head", "head", null)).toBeNull();
    });
});
