// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The bucket table and the rules that sort an operation into a bucket (SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §3-§5). */

import { describe, expect, it } from "vitest";

import { activityBucket, BUCKETS, bucketFrame, bucketTooltip, type BucketId, type BucketInput } from "./activity-bucket";
import { SPINNER_FRAMES } from "./AsciiSpinner";
import { TICKER_MS } from "./shared-ticker";

const all = Object.values(BUCKETS);

describe("the bucket table", () => {
    it("every frame is exactly 3 cells, so a row never shifts", () => {
        for (const b of all) for (const f of [...b.frames, b.still]) expect(f, `${b.id} "${f}"`).toHaveLength(3);
    });

    it("the slash is reserved: no frame is a bare slash glyph", () => {
        for (const b of all) {
            for (const f of b.frames) {
                const ink = f.trim();
                expect(SPINNER_FRAMES.map(String).includes(ink), `${b.id} "${f}"`).toBe(false);
            }
        }
    });

    it("every frame is printable ASCII", () => {
        for (const b of all) for (const f of [...b.frames, b.still]) expect(f).toMatch(/^[\x20-\x7e]{3}$/);
    });

    it("every period is a multiple of the ticker, and every still is one of the frames", () => {
        for (const b of all) {
            expect(b.periodMs % TICKER_MS, b.id).toBe(0);
            expect(b.frames, b.id).toContain(b.still);
        }
    });

    it("no two buckets share their frames, so each is recognisable at a glance", () => {
        expect(new Set(all.map((b) => b.frames.join("|"))).size).toBe(all.length);
        expect(new Set(all.map((b) => b.still)).size).toBe(all.length);
    });

    it("steps with the shared tick: floor(tick / (period / 120)) % frames", () => {
        const build = BUCKETS.build; // 240 ms: two ticks a frame
        expect([0, 1, 2, 3, 4].map((t) => bucketFrame(build, t))).toEqual(["+x+", "+x+", "x+x", "x+x", "+x+"]);
        const wait = BUCKETS.wait; // 360 ms: three ticks a frame, six frames
        expect(bucketFrame(wait, 2)).toBe("( )");
        expect(bucketFrame(wait, 3)).toBe("(.)");
        expect(bucketFrame(wait, 18)).toBe("( )");
    });

    it("the tooltip names the bucket and the time", () => {
        expect(bucketTooltip(BUCKETS.build, "4m 12s")).toBe("Building, 4m 12s");
    });
});

describe("activityBucket: first match wins", () => {
    const cases: Array<[string, BucketInput, BucketId]> = [
        ["a source: compaction", { source: "compaction" }, "compact"],
        ["a source: subagent", { source: "subagent" }, "delegate"],
        ["a source: quota hold", { source: "quota-hold" }, "hold"],
        ["a source: a scheduled start", { source: "schedule" }, "hold"],
        ["a source: a hibernated pane waking", { source: "hibernation" }, "hold"],
        ["a source: an approval prompt", { source: "approval" }, "wait"],
        ["a source: a CI or review wait", { source: "review-wait" }, "wait"],
        ["a source: a page loading", { source: "navigation" }, "receive"],
        ["a source: a container start", { source: "container" }, "install"],
        ["the source outranks the tool and the command", { source: "compaction", tool: "Grep", command: "cargo build" }, "compact"],
        ["a tool: Grep", { tool: "Grep" }, "search"],
        ["a tool: Glob", { tool: "Glob" }, "search"],
        ["a tool: history search", { tool: "search_history" }, "search"],
        ["a tool: WebFetch", { tool: "WebFetch" }, "receive"],
        ["a tool: Agent", { tool: "Agent" }, "delegate"],
        ["a tool: Task", { tool: "task" }, "delegate"],
        ["the tool outranks the command", { tool: "Grep", command: "cargo build" }, "search"],
        ["a Bash command, through the tool name Bash", { tool: "Bash", command: "cargo build --release" }, "build"],
        ["a command with no rule", { command: "my-own-tool --flag" }, "other"],
        ["nothing at all", {}, "other"],
    ];
    it.each(cases)("%s", (_name, input, id) => {
        expect(activityBucket(input).id).toBe(id);
    });
});
