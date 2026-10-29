// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-motion-gates.mjs.
// Spec: docs/specs/SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md §4.6, §9.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
    checkFile,
    collect,
    collectCounts,
    compareBaseline,
    countMotion,
    formatBaseline,
    lowered,
    parseBaseline,
} from "./check-motion-gates.mjs";

const rules = (path, text) => checkFile(path, text).map((v) => `${v.rule}:${v.line}`);

describe("rule 1: no raw prefers-reduced-motion", () => {
    it("flags a media query in a stylesheet, with its line", () => {
        const scss = ".a {\n    animation: x 1s;\n    @media (prefers-reduced-motion: reduce) {\n        animation: none;\n    }\n}\n";
        expect(rules("frontend/app/x.scss", scss)).toEqual(["media-query:3"]);
    });

    it("flags it in TS and in comments too", () => {
        expect(rules("frontend/app/x.ts", "// honours prefers-reduced-motion\n")).toEqual(["media-query:1"]);
        expect(rules("frontend/app/x.css", "@media (prefers-reduced-motion: reduce) {}")).toEqual(["media-query:1"]);
    });

    it("allows the mixin, motion.ts and its test", () => {
        const text = 'matchMedia("(prefers-reduced-motion: reduce)")';
        for (const p of ["frontend/app/mixins.scss", "frontend/app/motion/motion.ts", "frontend/app/motion/motion.test.ts"]) {
            expect(rules(p, text), p).toEqual([]);
        }
    });

    it("passes the policy's own selectors", () => {
        const scss = ':root[data-motion-tier="reduced"]:not([data-motion-reduce~="status"]) .fa-spin { animation-duration: 2s; }';
        expect(rules("frontend/app/app.scss", scss)).toEqual([]);
    });
});

describe("rule 2: no ad-hoc matchMedia for reduced motion", () => {
    it("flags a matchMedia call in a file that mentions reduced-motion", () => {
        const ts = 'const Q = "(reduced-motion)";\nconst x = 1;\nconst m = window.matchMedia(Q);\n';
        expect(rules("frontend/app/x.ts", ts)).toEqual(["match-media:3"]);
        expect(rules("frontend/app/y.ts", "// REDUCED-MOTION\nwindow.matchMedia?.(q)\n")).toEqual(["match-media:2"]);
    });

    it("leaves other media queries alone", () => {
        expect(rules("frontend/app/init/dpr.ts", 'matchMedia(`(resolution: ${dpr}dppx)`)')).toEqual([]);
    });
});

describe("rule 3: a known category", () => {
    it("accepts each category, namespaced or not, quoted or not", () => {
        const scss = [
            "@include mixins.when-motion-reduced(status) {}",
            "@include when-motion-reduced(feedback) {}",
            '@include mixins.when-motion-reduced("transition") {}',
            "@include m.when-motion-reduced( decorative ) {}",
        ].join("\n");
        expect(rules("frontend/app/x.scss", scss)).toEqual([]);
    });

    it("flags an unknown one, with its line", () => {
        expect(rules("frontend/app/x.scss", ".a {}\n@include mixins.when-motion-reduced(statuses) {}")).toEqual(["category:2"]);
    });

    it("ignores the mixin's own definition", () => {
        expect(rules("frontend/app/mixins.scss", "@mixin when-motion-reduced($category) {}")).toEqual([]);
    });
});

const found = (path, text) => countMotion(path, text).map((c) => `${c.rule}:${c.lines.join(",")}`);

describe("rule 4: no hand-rolled animate", () => {
    const call = "const a = el.animate([], { duration: 1 });\n";

    it("counts an animate call in a script, with its line", () => {
        expect(found("frontend/app/x.ts", `import x from "y";\n${call}`)).toEqual(["animate:2"]);
        expect(found("frontend/app/x.tsx", `${call}${call}`)).toEqual(["animate:1,2"]);
    });

    it("counts startViewTransition too", () => {
        expect(found("frontend/app/x.ts", "document.startViewTransition(() => {});")).toEqual(["animate:1"]);
    });

    it("allows the primitives, the view-transition entry point and the audio-clock exception", () => {
        for (const p of [
            "frontend/app/motion/primitives.ts",
            "frontend/app/util/view-transition.ts",
            "frontend/app/notification/activity-flash.ts",
        ]) {
            expect(found(p, call), p).toEqual([]);
        }
    });

    it("ignores comments and tests", () => {
        expect(found("frontend/app/x.ts", "// el.animate(x)\n/* document.startViewTransition() */\n")).toEqual([]);
        expect(found("frontend/app/x.test.ts", call)).toEqual([]);
        expect(found("frontend/app/__tests__/x.ts", call)).toEqual([]);
    });
});

describe("rule 5: no new keyframes", () => {
    it("counts a @keyframes outside _keyframes.scss, with its line", () => {
        expect(found("frontend/app/x.scss", ".a {}\n@keyframes spin { to { opacity: 1; } }\n")).toEqual(["keyframes:2"]);
    });

    it("allows _keyframes.scss and ignores comments", () => {
        expect(found("frontend/app/_keyframes.scss", "@keyframes am-fade-in {}")).toEqual([]);
        expect(found("frontend/app/x.scss", "// @keyframes old\n/* @keyframes older */")).toEqual([]);
    });
});

describe("rule 6: no literal milliseconds", () => {
    it("counts a literal duration in animation and transition, with its line", () => {
        const scss = ".a {\n    transition: opacity 140ms ease;\n}\n.b { animation: x 0.3s; animation-duration: 90ms; }\n";
        expect(found("frontend/app/x.scss", scss)).toEqual(["literal-ms:2,4"]);
    });

    it("counts a declaration that spans lines once", () => {
        expect(found("frontend/app/x.scss", ".a {\n    transition:\n        opacity 100ms ease,\n        color 200ms ease;\n}")).toEqual(["literal-ms:2"]);
    });

    it("passes tokens, and a custom property that defines one", () => {
        const scss = ".a { transition: opacity var(--duration-fast) var(--ease-out); }\n:root { --duration-x: calc(80ms * var(--motion-scale)); }\n";
        expect(found("frontend/app/x.scss", scss)).toEqual([]);
    });
});

describe("the baseline", () => {
    const base = new Map([
        ["animate frontend/a.ts", 2],
        ["keyframes frontend/b.scss", 1],
    ]);

    it("lets a listed file stay as it is", () => {
        expect(compareBaseline(new Map(base), base)).toEqual({ grew: [], added: [], shrank: [] });
    });

    it("fails a file that is not listed, and a count that grew", () => {
        const counts = new Map([
            ["animate frontend/a.ts", 3],
            ["keyframes frontend/b.scss", 1],
            ["animate frontend/new.ts", 1],
        ]);
        const { grew, added } = compareBaseline(counts, base);
        expect(grew.map((v) => v.key)).toEqual(["animate frontend/a.ts"]);
        expect(added.map((v) => v.key)).toEqual(["animate frontend/new.ts"]);
    });

    it("fails a listed file that no longer needs its entry, until the baseline is lowered", () => {
        const counts = new Map([["animate frontend/a.ts", 1]]);
        expect(compareBaseline(counts, base).shrank.map((v) => v.key)).toEqual(["animate frontend/a.ts", "keyframes frontend/b.scss"]);
        const next = lowered(counts, base);
        expect([...next]).toEqual([["animate frontend/a.ts", 1]]);
        expect(compareBaseline(counts, next)).toEqual({ grew: [], added: [], shrank: [] });
    });

    it("never raises a number or adds a file when lowering", () => {
        const counts = new Map([
            ["animate frontend/a.ts", 5],
            ["animate frontend/new.ts", 1],
        ]);
        expect([...lowered(counts, base)]).toEqual([["animate frontend/a.ts", 2]]);
    });

    it("round-trips through its file format and rejects a bad line", () => {
        expect(parseBaseline(formatBaseline(base))).toEqual(base);
        expect(() => parseBaseline("animate frontend/a.ts")).toThrow(/expected/);
    });
});

describe("the repository", () => {
    it("is clean", () => {
        expect(collect()).toEqual([]);
    });

    it("has no more hand-rolled motion than the baseline allows, and none the baseline no longer needs", () => {
        const baseline = parseBaseline(readFileSync(new URL("./check-motion-gates.baseline", import.meta.url), "utf8"));
        expect(compareBaseline(collectCounts(), baseline)).toEqual({ grew: [], added: [], shrank: [] });
    });
});
