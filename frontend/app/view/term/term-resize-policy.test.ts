// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { LIVE_REFIT_LARGE_BUFFER_LINES, liveRefit } from "./term-resize-policy";

describe("liveRefit (SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md §4.1)", () => {
    const cur = { cols: 80, rows: 24 };

    it("does nothing when the grid already fits", () => {
        expect(liveRefit({ cols: 80, rows: 24 }, cur, 10_000)).toBe("none");
    });

    it("applies a row change immediately, whatever the buffer", () => {
        expect(liveRefit({ cols: 80, rows: 30 }, cur, 10_000)).toBe("now");
    });

    it("applies a column change immediately on a short buffer", () => {
        expect(liveRefit({ cols: 100, rows: 24 }, cur, LIVE_REFIT_LARGE_BUFFER_LINES)).toBe("now");
    });

    it("throttles a column change on a long buffer instead of freezing it", () => {
        expect(liveRefit({ cols: 100, rows: 24 }, cur, LIVE_REFIT_LARGE_BUFFER_LINES + 1)).toBe("throttle");
        expect(liveRefit({ cols: 100, rows: 30 }, cur, LIVE_REFIT_LARGE_BUFFER_LINES + 1)).toBe("throttle");
    });
});
