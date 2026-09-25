// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { snapshotFitsPane } from "./use-drag-snapshot";

describe("snapshotFitsPane", () => {
    const pane = { x: 0, y: 0, width: 815, height: 463 };

    it("accepts this pane's own size, allowing for rounding", () => {
        expect(snapshotFitsPane({ width: 815, height: 463 }, pane)).toBe(true);
        expect(snapshotFitsPane({ width: 816, height: 464 }, pane)).toBe(true);
    });

    // Measured live: with several panes on one URL, the host returned
    // another pane's 815x604 page for this 815x463 pane.
    it("rejects another pane's page", () => {
        expect(snapshotFitsPane({ width: 815, height: 604 }, pane)).toBe(false);
        expect(snapshotFitsPane({ width: 986, height: 463 }, pane)).toBe(false);
    });
});
