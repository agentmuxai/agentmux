// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { summarizeTitleOutcomes } from "./ambient-outcomes";

describe("summarizeTitleOutcomes", () => {
    it("adds up the pane's requests and the backend recovery", () => {
        const s = summarizeTitleOutcomes({
            activity_summary: { accepted: 10, kept: 30, "rejected:absence_pattern": 1, superseded: 4 },
            activity_summary_pushed: { accepted: 2, "rejected:refusal": 1, timeout: 1, empty_digest: 3 },
            next_prompt_suggestion: { accepted: 99 },
        })!;
        expect(s.text).toBe("12 accepted · 30 kept · 2 refused · 1 failed");
        expect(s.detail).toContain("activity_summary_pushed rejected:refusal: 1");
        expect(s.detail).not.toContain("next_prompt_suggestion");
        expect(s.unhealthy).toBe(false);
    });

    it("flags a pipeline that refuses or fails more than it produces", () => {
        const s = summarizeTitleOutcomes({ activity_summary: { accepted: 1, "rejected:shape": 2, cli_failed: 2, not_run: 1 } })!;
        expect(s.text).toBe("1 accepted · 0 kept · 2 refused · 3 failed");
        expect(s.unhealthy).toBe(true);
    });

    it("has nothing to say before any title call has ended", () => {
        expect(summarizeTitleOutcomes(null)).toBeNull();
        expect(summarizeTitleOutcomes({})).toBeNull();
        expect(summarizeTitleOutcomes({ next_prompt_suggestion: { accepted: 3 } })).toBeNull();
    });
});
