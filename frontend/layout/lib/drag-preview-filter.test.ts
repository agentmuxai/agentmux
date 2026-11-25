// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { insideGhost } from "./drag-preview-filter";

type Box = { top: number; left: number; width: number; height: number };
const el = (b: Box) =>
    ({ getBoundingClientRect: () => ({ ...b, bottom: b.top + b.height, right: b.left + b.width }) }) as unknown as HTMLElement;

const root = el({ top: 100, left: 50, width: 300, height: 200 });

describe("insideGhost", () => {
    it("keeps the root and what lies inside or across the preview's box", () => {
        const keep = insideGhost(root);
        expect(keep(root)).toBe(true);
        expect(keep(el({ top: 110, left: 60, width: 100, height: 20 }))).toBe(true);
        expect(keep(el({ top: 250, left: 60, width: 100, height: 400 }))).toBe(true); // starts inside, runs out
        expect(keep(el({ top: 300, left: 60, width: 100, height: 20 }))).toBe(true); // touches the edge
    });

    it("leaves out what is wholly below, above, left of or right of the box", () => {
        const keep = insideGhost(root);
        expect(keep(el({ top: 302, left: 60, width: 100, height: 20 }))).toBe(false);
        expect(keep(el({ top: 40, left: 60, width: 100, height: 20 }))).toBe(false);
        expect(keep(el({ top: 110, left: 352, width: 100, height: 20 }))).toBe(false);
        expect(keep(el({ top: 110, left: -80, width: 100, height: 20 }))).toBe(false);
    });

    it("keeps an element with no box of its own, which may hold ones that have", () => {
        expect(insideGhost(root)(el({ top: 900, left: 900, width: 0, height: 0 }))).toBe(true);
    });

    it("keeps everything when the preview has no size to compare with", () => {
        const keep = insideGhost(el({ top: 0, left: 0, width: 0, height: 0 }));
        expect(keep(el({ top: 5000, left: 5000, width: 10, height: 10 }))).toBe(true);
    });
});
