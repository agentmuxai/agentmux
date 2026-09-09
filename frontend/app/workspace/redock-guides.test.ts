// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { DropDirection } from "@/layout/lib/types";
import { guideRectsForLeaf, hitTestGuides } from "./redock-guides";

// A comfortably large leaf, so nothing is clamped.
const LEAF = { top: 100, left: 200, width: 800, height: 600 };

function hit(x: number, y: number, leaf = LEAF): DropDirection | null {
    return hitTestGuides(guideRectsForLeaf(leaf), x, y);
}

const centerX = LEAF.left + LEAF.width / 2;
const centerY = LEAF.top + LEAF.height / 2;

describe("redock guides — the parking zone", () => {
    it("leaves the bulk of the leaf neutral so a floater can be parked", () => {
        // Halfway between the compass cluster and the edge band, on each axis.
        expect(hit(LEAF.left + 150, centerY)).toBe(null);
        expect(hit(LEAF.left + LEAF.width - 150, centerY)).toBe(null);
        expect(hit(centerX, LEAF.top + 120)).toBe(null);
        expect(hit(centerX, LEAF.top + LEAF.height - 120)).toBe(null);
        // Diagonally out from the compass, well clear of every band.
        expect(hit(LEAF.left + 200, LEAF.top + 160)).toBe(null);
    });

    it("measures the neutral area as the clear majority of the leaf", () => {
        // Guard against a future tweak quietly eating the parking area: sample
        // the leaf on a grid and require most of it to hit nothing.
        const guides = guideRectsForLeaf(LEAF);
        let neutral = 0;
        let total = 0;
        for (let x = LEAF.left; x < LEAF.left + LEAF.width; x += 10) {
            for (let y = LEAF.top; y < LEAF.top + LEAF.height; y += 10) {
                total++;
                if (hitTestGuides(guides, x, y) === null) neutral++;
            }
        }
        expect(neutral / total).toBeGreaterThan(0.6);
    });

    it("reports nothing for a point outside the leaf entirely", () => {
        expect(hit(LEAF.left - 5, centerY)).toBe(null);
        expect(hit(centerX, LEAF.top + LEAF.height + 5)).toBe(null);
    });
});

describe("redock guides — the compass", () => {
    it("maps the centre cell to Center", () => {
        expect(hit(centerX, centerY)).toBe(DropDirection.Center);
    });

    it("maps the four cells around the centre to the half-splits", () => {
        const guides = guideRectsForLeaf(LEAF);
        const cell = guides.find((g) => g.dir === DropDirection.Center)!;
        const step = cell.height + 4; // one cell plus the gap
        expect(hitTestGuides(guides, centerX, centerY - step)).toBe(DropDirection.Top);
        expect(hitTestGuides(guides, centerX, centerY + step)).toBe(DropDirection.Bottom);
        expect(hitTestGuides(guides, centerX - step, centerY)).toBe(DropDirection.Left);
        expect(hitTestGuides(guides, centerX + step, centerY)).toBe(DropDirection.Right);
    });

    it("leaves the diagonal corners of the cluster neutral", () => {
        // The compass is a plus, not a 3x3 block — corners are parking area.
        const guides = guideRectsForLeaf(LEAF);
        const cell = guides.find((g) => g.dir === DropDirection.Center)!;
        const step = cell.height + 4;
        expect(hitTestGuides(guides, centerX - step, centerY - step)).toBe(null);
        expect(hitTestGuides(guides, centerX + step, centerY + step)).toBe(null);
    });
});

describe("redock guides — the edge bands", () => {
    it("maps each edge strip to its Outer direction", () => {
        expect(hit(centerX, LEAF.top + 2)).toBe(DropDirection.OuterTop);
        expect(hit(centerX, LEAF.top + LEAF.height - 2)).toBe(DropDirection.OuterBottom);
        expect(hit(LEAF.left + 2, centerY)).toBe(DropDirection.OuterLeft);
        expect(hit(LEAF.left + LEAF.width - 2, centerY)).toBe(DropDirection.OuterRight);
    });

    it("keeps every direction reachable, so nothing is lost versus the old mapping", () => {
        const reachable = new Set(guideRectsForLeaf(LEAF).map((g) => g.dir));
        expect(reachable).toEqual(
            new Set([
                DropDirection.Top,
                DropDirection.Right,
                DropDirection.Bottom,
                DropDirection.Left,
                DropDirection.OuterTop,
                DropDirection.OuterRight,
                DropDirection.OuterBottom,
                DropDirection.OuterLeft,
                DropDirection.Center,
            ]),
        );
    });
});

describe("redock guides — small leaves", () => {
    it("still fits a usable compass in a small pane without overlapping guides", () => {
        const small = { top: 0, left: 0, width: 200, height: 160 };
        const guides = guideRectsForLeaf(small);
        // Every guide must lie inside the leaf.
        for (const g of guides) {
            expect(g.left).toBeGreaterThanOrEqual(small.left);
            expect(g.top).toBeGreaterThanOrEqual(small.top);
            expect(g.left + g.width).toBeLessThanOrEqual(small.left + small.width);
            expect(g.top + g.height).toBeLessThanOrEqual(small.top + small.height);
        }
        // And the centre must still resolve to Center rather than a band.
        expect(hitTestGuides(guides, 100, 80)).toBe(DropDirection.Center);
    });

    it("prefers the compass over an edge band where a tiny leaf makes them meet", () => {
        // Ordering guarantee: the compass is hit-tested first, so a clamped
        // layout can never make the centre of a pane mean OuterLeft.
        const tiny = { top: 0, left: 0, width: 90, height: 90 };
        const guides = guideRectsForLeaf(tiny);
        expect(hitTestGuides(guides, 45, 45)).toBe(DropDirection.Center);
    });
});
