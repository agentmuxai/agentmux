// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { uniquifySvgIds } from "./svg-ids";

describe("uniquifySvgIds", () => {
    const svg = '<svg><linearGradient id="g"/><path fill="url(#g)"/><use href="#g"/><rect fill="#ff0000"/></svg>';

    it("gives every copy its own ids and rewrites references to them", () => {
        const a = uniquifySvgIds(svg);
        const b = uniquifySvgIds(svg);
        const idOf = (s: string) => /id="([^"]+)"/.exec(s)![1];
        expect(idOf(a)).not.toBe(idOf(b));
        expect(a).toContain(`url(#${idOf(a)})`);
        expect(a).toContain(`href="#${idOf(a)}"`);
    });

    it("leaves colours and id-less markup alone", () => {
        expect(uniquifySvgIds(svg)).toContain('fill="#ff0000"');
        expect(uniquifySvgIds("<svg><path/></svg>")).toBe("<svg><path/></svg>");
    });
});
