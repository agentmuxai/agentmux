// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    compactionEstimate,
    compactionModelKey,
    compactionProgress,
    DEFAULT_COMPACTION_MS,
    estimateCompactionMs,
    parseCompactionSample,
    readCompactionSamples,
    recordCompactionSample,
    samplesForModel,
    type CompactionSample,
} from "./compaction-estimate";

/** An in-memory `Storage` stand-in with only what the module uses. */
function memoryStore(initial?: string): Pick<Storage, "getItem" | "setItem"> & { value: string | null } {
    const s = {
        value: initial ?? null,
        getItem: () => s.value,
        setItem: (_k: string, v: string) => {
            s.value = v;
        },
    };
    return s;
}

const sample = (uuid: string, preTokens: number, durationMs: number): CompactionSample => ({ uuid, preTokens, durationMs });

// The frame the real CLI 2.1.287 wrote to stdout for a manual /compact
// (SPEC_COMPACTION_ESTIMATED_PROGRESS_AND_STREAM_FRAMES_2026_10_01.md §2).
const REAL_STDOUT_FRAME = {
    type: "system",
    subtype: "compact_boundary",
    session_id: "6b8e7806-028f-415d-8382-5073daa09bdc",
    uuid: "7acdebbc-9447-45a0-ba43-45509913fd7b",
    compact_metadata: {
        trigger: "manual",
        pre_tokens: 25040,
        post_tokens: 733,
        cumulative_dropped_tokens: 24307,
        duration_ms: 1513,
    },
    logical_parent_uuid: "741f4dfc-fb15-4af7-b938-5d5439e6b4cf",
};

describe("parseCompactionSample", () => {
    it("reads the real stdout frame (snake_case, no timestamp)", () => {
        expect(parseCompactionSample(REAL_STDOUT_FRAME)).toEqual({
            uuid: "7acdebbc-9447-45a0-ba43-45509913fd7b",
            preTokens: 25040,
            durationMs: 1513,
        });
    });

    it("reads the camelCase transcript form too", () => {
        const f = {
            type: "system",
            subtype: "compact_boundary",
            uuid: "u1",
            compactMetadata: { trigger: "auto", preTokens: 90000, postTokens: 1000, durationMs: 31000 },
        };
        expect(parseCompactionSample(f)).toEqual({ uuid: "u1", preTokens: 90000, durationMs: 31000 });
    });

    it("falls back to a content key when the frame has no uuid", () => {
        const f = { ...REAL_STDOUT_FRAME, uuid: undefined };
        expect(parseCompactionSample(f)?.uuid).toBe("25040:1513");
    });

    it.each([
        ["not an object", "x"],
        ["null", null],
        ["another system subtype", { type: "system", subtype: "status", status: "compacting" }],
        ["an assistant frame", { type: "assistant" }],
        ["no metadata", { type: "system", subtype: "compact_boundary", uuid: "u" }],
        ["zero duration", { ...REAL_STDOUT_FRAME, compact_metadata: { pre_tokens: 5, duration_ms: 0 } }],
        ["negative duration", { ...REAL_STDOUT_FRAME, compact_metadata: { pre_tokens: 5, duration_ms: -1 } }],
        ["string tokens", { ...REAL_STDOUT_FRAME, compact_metadata: { pre_tokens: "lots", duration_ms: 5 } }],
        ["non-finite duration", { ...REAL_STDOUT_FRAME, compact_metadata: { pre_tokens: 5, duration_ms: Infinity } }],
    ])("rejects %s", (_name, frame) => {
        expect(parseCompactionSample(frame)).toBeNull();
    });
});

describe("estimateCompactionMs", () => {
    it("is null with no samples (the row keeps showing the plain elapsed counter)", () => {
        expect(estimateCompactionMs([], 50000)).toBeNull();
    });

    it("uses the median duration, so one slow outlier doesn't move it", () => {
        const s = [sample("a", 50000, 20000), sample("b", 50000, 22000), sample("c", 50000, 300000)];
        expect(estimateCompactionMs(s, 50000)).toBe(22000);
    });

    it("scales with the current context size relative to the samples", () => {
        const s = [sample("a", 40000, 20000), sample("b", 40000, 20000)];
        expect(estimateCompactionMs(s, 60000)).toBe(30000); // 1.5x the context
    });

    it("clamps the scale to 0.5x and 2x", () => {
        const s = [sample("a", 40000, 20000)];
        expect(estimateCompactionMs(s, 4_000_000)).toBe(40000); // 100x context, capped at 2x
        expect(estimateCompactionMs(s, 400)).toBe(10000); // 0.01x context, floored at 0.5x
    });

    it("does not scale when the context size is unknown", () => {
        const s = [sample("a", 40000, 20000)];
        expect(estimateCompactionMs(s, null)).toBe(20000);
        expect(estimateCompactionMs(s, 0)).toBe(20000);
    });

    it("clamps the result to 5s..600s", () => {
        expect(estimateCompactionMs([sample("a", 1000, 800)], 1000)).toBe(5000);
        expect(estimateCompactionMs([sample("a", 1000, 9_000_000)], 1000)).toBe(600000);
    });
});

describe("compactionEstimate", () => {
    it("is the built-in default before any compaction, whatever the context size", () => {
        expect(DEFAULT_COMPACTION_MS).toBe(75_000);
        expect(compactionEstimate([], 50000)).toEqual({ estimateMs: 75_000, fromHistory: false });
        expect(compactionEstimate([], 950_000)).toEqual({ estimateMs: 75_000, fromHistory: false });
        expect(compactionEstimate([], null)).toEqual({ estimateMs: 75_000, fromHistory: false });
    });

    it("uses earlier compactions as soon as there is one", () => {
        expect(compactionEstimate([sample("a", 50000, 40000)], 50000)).toEqual({ estimateMs: 40000, fromHistory: true });
    });
});

describe("compactionProgress", () => {
    it("is the elapsed fraction of the estimate", () => {
        expect(compactionProgress(15000, 30000)).toEqual({ fraction: 0.5, over: false });
    });

    it("never reaches 100% on its own", () => {
        expect(compactionProgress(29999, 30000).fraction).toBeLessThanOrEqual(0.95);
    });

    it("reports overshoot once the estimate is exceeded, with the fill held at 95%", () => {
        expect(compactionProgress(45000, 30000)).toEqual({ fraction: 0.95, over: true });
    });

    it("is zero at the start and treats a negative elapsed as zero", () => {
        expect(compactionProgress(0, 30000)).toEqual({ fraction: 0, over: false });
        expect(compactionProgress(-5, 30000).fraction).toBe(0);
    });
});

describe("sample store", () => {
    it("round-trips through storage", () => {
        const st = memoryStore();
        recordCompactionSample(sample("a", 1000, 20000), st);
        expect(readCompactionSamples(st)).toEqual([sample("a", 1000, 20000)]);
    });

    it("counts a repeated uuid once", () => {
        const st = memoryStore();
        recordCompactionSample(sample("a", 1000, 20000), st);
        recordCompactionSample(sample("a", 1000, 20000), st);
        expect(readCompactionSamples(st)).toHaveLength(1);
    });

    it.each([
        ["not JSON", "{{"],
        ["not an array", '{"a":1}'],
        ["bad entries only", '[{"uuid":1},null,{"uuid":"x","preTokens":-1,"durationMs":5}]'],
    ])("reads %s as empty", (_n, raw) => {
        expect(readCompactionSamples(memoryStore(raw))).toEqual([]);
    });

    it("drops bad entries but keeps the good ones", () => {
        const raw = JSON.stringify([sample("ok", 1000, 20000), { uuid: "bad" }]);
        expect(readCompactionSamples(memoryStore(raw))).toEqual([sample("ok", 1000, 20000)]);
    });

    it("behaves as empty, and does not throw, when storage throws", () => {
        const boom = {
            getItem: () => {
                throw new Error("denied");
            },
            setItem: () => {
                throw new Error("quota");
            },
        };
        expect(readCompactionSamples(boom)).toEqual([]);
        expect(() => recordCompactionSample(sample("a", 1000, 20000), boom)).not.toThrow();
    });

    it("behaves as empty when there is no storage at all", () => {
        expect(readCompactionSamples(null)).toEqual([]);
        expect(() => recordCompactionSample(sample("a", 1000, 20000), null)).not.toThrow();
    });
});

describe("per-model samples", () => {
    it("records the model a compaction ran on, and keeps it through storage", () => {
        const store = memoryStore();
        recordCompactionSample(parseCompactionSample(REAL_STDOUT_FRAME, "claude-opus-5-5"), store);
        const [s] = readCompactionSamples(store);
        expect(s.model).toBe("claude-opus-5-5");
        expect(parseCompactionSample(REAL_STDOUT_FRAME)?.model).toBeUndefined();
    });

    it("estimates from the current model's samples when it has any", () => {
        const all: CompactionSample[] = [
            { ...sample("a", 50000, 10000), model: "fast" },
            { ...sample("b", 50000, 60000), model: "slow" },
            { ...sample("c", 50000, 62000), model: "slow" },
        ];
        expect(samplesForModel(all, "slow").map((s) => s.uuid)).toEqual(["b", "c"]);
        expect(estimateCompactionMs(samplesForModel(all, "fast"), 50000)).toBe(10000);
    });

    it("falls back to every sample for an unseen or unknown model", () => {
        const all: CompactionSample[] = [sample("old", 50000, 20000), { ...sample("a", 50000, 10000), model: "fast" }];
        expect(samplesForModel(all, "new-model")).toHaveLength(2);
        expect(samplesForModel(all, null)).toHaveLength(2);
    });

    it("keeps up to 30 samples so several models don't crowd each other out", () => {
        const store = memoryStore();
        for (let i = 0; i < 40; i++) recordCompactionSample(sample(`s${i}`, 1000, 1000 + i), store);
        const kept = readCompactionSamples(store);
        expect(kept).toHaveLength(30);
        expect(kept[0].uuid).toBe("s10");
    });
});

describe("compactionModelKey", () => {
    it("uses the configured model, so a /model switch counts before the next reply", () => {
        expect(compactionModelKey({ "agent:runtime": { model: "opus" } }, "claude-sonnet-5-5")).toBe("opus");
    });

    it("lets a --model in the agent's own flags win, as the process does", () => {
        const meta = { "agent:runtime": { model: "opus" }, "agent:provider_flags": "--model haiku" };
        expect(compactionModelKey(meta, null)).toBe("haiku");
    });

    it("uses the runtime default when nothing is configured", () => {
        expect(compactionModelKey(undefined, "claude-opus-5-5")).toBe("sonnet");
    });
});
