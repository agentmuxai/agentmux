// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-motion-gates.mjs.
// Spec: docs/specs/SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md §4.6, §9.

import { describe, expect, it } from "vitest";
import { checkFile, collect } from "./check-motion-gates.mjs";

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

describe("the repository", () => {
    it("is clean", () => {
        expect(collect()).toEqual([]);
    });
});
