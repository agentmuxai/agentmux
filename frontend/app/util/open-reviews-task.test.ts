// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const open = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: open }));

import { openReviewsForTask } from "./open-reviews-task";

describe("openReviewsForTask", () => {
    beforeEach(() => open.mockClear());

    it("opens the Reviews pane filtered to the task, by the pane's own view id and meta key", async () => {
        await openReviewsForTask("work#12");
        expect(open).toHaveBeenCalledWith("reviews", { meta: { view: "reviews", "reviews:task": "work#12" } }, { "reviews:task": "work#12" });
    });
});
