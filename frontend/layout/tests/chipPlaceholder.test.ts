// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A dragged chip's drop placeholder: the chip's size, centred on the seam it is inserted at,
// flush inside the edge at the first or last place. SPEC_CHIP_DROP_PLACEHOLDER_ON_THE_SEAM_2026_10_21.md.

import { describe, expect, it } from "vitest";
import { chipPlaceholderRect } from "../lib/chipPlaceholder";
import { FlexDirection } from "../lib/types";

const chip = { width: 600, height: 36 };
// A column of three chips, 36 px each.
const chipAt = (i: number): Dimensions => ({ top: i * 36, left: 0, width: 600, height: 36 });

describe("chipPlaceholderRect", () => {
    it("between two chips, straddles the seam instead of covering the chip", () => {
        const rect = chipPlaceholderRect(chipAt(0), chip, FlexDirection.Column, true, false);
        expect(rect).toEqual({ top: 18, left: 0, width: 600, height: 36 });
    });

    it("at the first place, is flush inside the top edge", () => {
        const rect = chipPlaceholderRect(chipAt(0), chip, FlexDirection.Column, false, true);
        expect(rect).toEqual({ top: 0, left: 0, width: 600, height: 36 });
    });

    it("at the last place, is flush inside the bottom edge", () => {
        const rect = chipPlaceholderRect(chipAt(2), chip, FlexDirection.Column, true, true);
        expect(rect).toEqual({ top: 72, left: 0, width: 600, height: 36 });
    });

    it("after an expanded pane with a pane below, is centred on the seam", () => {
        const pane = { top: 0, left: 0, width: 600, height: 400 };
        const rect = chipPlaceholderRect(pane, chip, FlexDirection.Column, true, false);
        expect(rect).toEqual({ top: 382, left: 0, width: 600, height: 36 });
    });

    it("in a row, straddles the vertical seam with the chip's width", () => {
        const pane = { top: 10, left: 0, width: 300, height: 500 };
        const rect = chipPlaceholderRect(pane, { width: 200, height: 36 }, FlexDirection.Row, true, false);
        expect(rect).toEqual({ top: 10, left: 200, width: 200, height: 36 });
    });

    it("is never bigger than the pane it is placed against", () => {
        const small = { top: 0, left: 0, width: 100, height: 20 };
        const rect = chipPlaceholderRect(small, chip, FlexDirection.Row, false, true);
        expect(rect).toEqual({ top: 0, left: 0, width: 100, height: 20 });
    });
});
