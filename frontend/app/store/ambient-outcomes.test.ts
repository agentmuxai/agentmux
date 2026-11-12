// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    describeFailingTip,
    describeFailure,
    purposeWhat,
    summarizeAmbientOutcomes,
    type AmbientFailure,
} from "./ambient-outcomes";

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
        expect(s.rows[1].counts).toEqual({ accepted: 2, kept: 0, refused: 1, skipped: 3, failed: 1 });
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

describe("describeFailure and describeFailingTip", () => {
    const base: AmbientFailure = {
        purpose: "activity_summary",
        entity: "blk-1",
        entity_name: "Deke",
        outcome: "cli_failed",
        count: 2585,
        last_at: Date.UTC(2026, 10, 12, 7, 0),
    };

    it("names the call, the pane and how often it failed", () => {
        expect(describeFailure(base)).toBe("AgentMux can't write the session title for Deke (failed 2,585 times since start)");
    });

    it("says a timeout timed out, and uses the id when srv has no name", () => {
        const text = describeFailure({ ...base, entity_name: null, outcome: "timeout", count: 1 });
        expect(text).toBe("AgentMux can't write the session title for blk-1 (timed out 1 time since start)");
    });

    it("leads with out of credit or needs sign-in instead of failing", () => {
        expect(describeFailure({ ...base, class: "payment" })).toMatch(/^Out of credit: AgentMux can't write the session title for Deke/);
        expect(describeFailure({ ...base, class: "auth" })).toMatch(/^Needs sign-in: /);
        expect(describeFailure({ ...base, class: "other" })).toMatch(/^AgentMux can't/);
    });

    it("says when it tries again while backing off, and not after that time", () => {
        const now = Date.UTC(2026, 10, 12, 7, 0);
        expect(describeFailure({ ...base, retry_at: now + 600_000 }, now)).toContain(", trying again at ");
        expect(describeFailure({ ...base, retry_at: now - 1 }, now)).not.toContain("trying again");
    });

    it("names the first failing pane and counts the rest", () => {
        const s = summarizeAmbientOutcomes({ activity_summary: { accepted: 1, cli_failed: 5 } });
        const other: AmbientFailure = { ...base, entity: "blk-2", entity_name: "Klon", count: 3 };
        const tip = describeFailingTip(s, [base, other], base.last_at);
        expect(tip).toBe("AgentMux can't write the session title for Deke (failed 2,585 times since start) +1 more");
    });

    it("ignores a failure of a purpose that is healthy now", () => {
        const s = summarizeAmbientOutcomes({ activity_summary: { accepted: 50, cli_failed: 1 } });
        expect(describeFailingTip(s, [base])).toBe("");
    });

    it("falls back to the purpose when srv names no pane", () => {
        const s = summarizeAmbientOutcomes({
            activity_summary: { accepted: 1, cli_failed: 3 },
            next_prompt_suggestion: { accepted: 1, "rejected:format": 4 },
        });
        expect(describeFailingTip(s, [])).toBe("AgentMux can't write the session title (failed 3 times since start) +1 more");
        const refused = summarizeAmbientOutcomes({ next_prompt_suggestion: { accepted: 1, "rejected:format": 4 } });
        expect(describeFailingTip(refused, [])).toBe("AgentMux can't suggest the next prompt (refused 4 times since start)");
    });

    it("names a purpose this build doesn't know by its tag", () => {
        expect(purposeWhat("brand_new")).toBe("run brand_new");
    });
});
