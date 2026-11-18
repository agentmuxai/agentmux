// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A magnified pane never opens past the layout area, whatever the config holds. */

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ stored: undefined as unknown }));
vi.mock("@/app/store/block-atom-cache", () => ({ getSettingsKeyAtom: () => () => h.stored }));

import { clampMagnifiedSize, getMagnifiedSizeAtom, MAGNIFIED_SIZE_MAX, MAGNIFIED_SIZE_MIN } from "./magnified-size";

beforeEach(() => {
    h.stored = undefined;
});

describe("clampMagnifiedSize", () => {
    it("holds a size to the layout area: 3 is read as the largest, 0 and negatives as the smallest", () => {
        expect(clampMagnifiedSize(3)).toBe(MAGNIFIED_SIZE_MAX);
        expect(clampMagnifiedSize(1.5)).toBe(MAGNIFIED_SIZE_MAX);
        expect(clampMagnifiedSize(0)).toBe(MAGNIFIED_SIZE_MIN);
        expect(clampMagnifiedSize(-2)).toBe(MAGNIFIED_SIZE_MIN);
    });

    it("leaves a size in range as it is, the ends included", () => {
        for (const size of [MAGNIFIED_SIZE_MIN, 0.8, 0.9, MAGNIFIED_SIZE_MAX]) expect(clampMagnifiedSize(size)).toBe(size);
    });
});

describe("getMagnifiedSizeAtom, what the layout reads", () => {
    it("holds a stored size from an old config to the range", () => {
        h.stored = 3;
        expect(getMagnifiedSizeAtom()()).toBe(MAGNIFIED_SIZE_MAX);
        h.stored = 0.1;
        expect(getMagnifiedSizeAtom()()).toBe(MAGNIFIED_SIZE_MIN);
    });

    it("passes a size in range through", () => {
        h.stored = 0.85;
        expect(getMagnifiedSizeAtom()()).toBe(0.85);
    });

    it("stays unset while the setting is unset or not a number, so each reader's own default applies", () => {
        expect(getMagnifiedSizeAtom()()).toBeUndefined();
        for (const junk of [null, "big", Number.NaN, Number.POSITIVE_INFINITY]) {
            h.stored = junk;
            expect(getMagnifiedSizeAtom()()).toBeUndefined();
        }
    });
});
