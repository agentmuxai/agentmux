// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { summarizeAmbientOutcomes } from "./ambient-outcomes";

describe("summarizeAmbientOutcomes", () => {
    it("gives every purpose its own row, named, in a fixed order", () => {
        const s = summarizeAmbientOutcomes({
            next_prompt_suggestion: { accepted: 99 },
            activity_summary_pushed: { accepted: 2, "rejected:refusal": 1, timeout: 1, empty_digest: 3 },
            activity_summary: { accepted: 10, kept: 30, "rejected:absence_pattern": 1, superseded: 4 },
        })!;
        expect(s.rows.map((r) => r.name)).toEqual(["Session titles", "Session titles (recovery)", "Prompt suggestions"]);
        expect(s.rows[0].text).toBe("10 accepted · 30 kept · 1 refused · 4 skipped");
        expect(s.rows[1].text).toBe("2 accepted · 0 kept · 1 refused · 3 skipped · 1 failed");
        expect(s.rows[1].detail).toContain("rejected:refusal: 1");
        expect(s.total).toBe(45 + 7 + 99);
        expect(s.unhealthyCount).toBe(0);
    });

    it("flags a purpose that refuses or fails more than it answers", () => {
        const s = summarizeAmbientOutcomes({ activity_summary: { accepted: 1, "rejected:shape": 2, cli_failed: 2, not_run: 1 } })!;
        expect(s.rows[0].text).toBe("1 accepted · 0 kept · 2 refused · 3 failed");
        expect(s.rows[0].unhealthy).toBe(true);
        expect(s.unhealthyCount).toBe(1);
    });

    it("counts a kept title as a good answer, not a failure", () => {
        const s = summarizeAmbientOutcomes({ activity_summary: { kept: 30, timeout: 1 } })!;
        expect(s.rows[0].unhealthy).toBe(false);
    });

    it("counts a SKIP as a good answer and a gated call as skipped, not as failures", () => {
        const s = summarizeAmbientOutcomes({
            next_prompt_suggestion: { skipped: 20, gated: 15, "rejected:format": 1, timeout: 2 },
        })!;
        expect(s.rows[0].unhealthy).toBe(false);
        expect(s.rows[0].text).toBe("0 accepted · 20 kept · 1 refused · 15 skipped · 2 failed");
    });

    it("still shows a purpose or outcome label this build doesn't know", () => {
        const s = summarizeAmbientOutcomes({ brand_new_purpose: { accepted: 1, some_new_label: 2 } })!;
        expect(s.rows[0].name).toBe("brand_new_purpose");
        expect(s.rows[0].total).toBe(3);
        expect(s.rows[0].detail).toContain("some_new_label: 2");
    });

    it("has nothing to say before any call has ended", () => {
        expect(summarizeAmbientOutcomes(null)).toBeNull();
        expect(summarizeAmbientOutcomes({})).toBeNull();
        expect(summarizeAmbientOutcomes({ activity_summary: {} })).toBeNull();
    });
});
