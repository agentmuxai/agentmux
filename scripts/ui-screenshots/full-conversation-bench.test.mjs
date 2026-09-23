// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The bench's pure parts. The in-page harness (lib/bench-page.js) is exercised
// against a live dev build; here it is only checked to parse as a single
// evaluable expression, since that is how it is sent over CDP.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { metricsDelta, parseArgs } from "./full-conversation-bench.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));

describe("parseArgs", () => {
    it("defaults", () => {
        const o = parseArgs([]);
        expect(o).toMatchObject({
            port: 9223,
            panes: [],
            history: [0, 25, 100, 200, 500],
            turnKb: 20,
            streamKb: 20,
            secs: 10,
            typeInto: 0,
            kps: 20,
            soakMinutes: null,
        });
    });

    it("reads panes, history, typing target and soak", () => {
        const o = parseArgs([
            "--panes",
            "a,b",
            "--history",
            "0,10",
            "--type-into",
            "none",
            "--soak",
            "480",
            "--sample-every",
            "30",
        ]);
        expect(o).toMatchObject({
            panes: ["a", "b"],
            history: [0, 10],
            typeInto: null,
            soakMinutes: 480,
            sampleEvery: 30,
        });
    });

    it("rejects history that decreases (it is cumulative) or is not integral", () => {
        expect(() => parseArgs(["--history", "10,5"])).toThrow(/non-decreasing/);
        expect(() => parseArgs(["--history", "1.5"])).toThrow(/non-negative integers/);
        expect(() => parseArgs(["--secs", "-1"])).toThrow(/non-negative/);
    });
});

describe("metricsDelta", () => {
    it("reports counts as deltas, durations in ms, heap in MB", () => {
        const before = {
            LayoutCount: 10,
            RecalcStyleCount: 20,
            LayoutDuration: 0.1,
            RecalcStyleDuration: 0.2,
            ScriptDuration: 1,
            TaskDuration: 2,
            JSHeapUsedSize: 0,
            JSHeapTotalSize: 0,
            Nodes: 0,
            JSEventListeners: 0,
        };
        const after = {
            LayoutCount: 15,
            RecalcStyleCount: 26,
            LayoutDuration: 0.35,
            RecalcStyleDuration: 0.3,
            ScriptDuration: 1.5,
            TaskDuration: 3,
            JSHeapUsedSize: 50 * 1024 * 1024,
            JSHeapTotalSize: 80 * 1024 * 1024,
            Nodes: 1234,
            JSEventListeners: 99,
        };
        expect(metricsDelta(before, after)).toEqual({
            layouts: 5,
            styleRecalcs: 6,
            layoutMs: 250,
            styleMs: 100,
            scriptMs: 500,
            taskMs: 1000,
            jsHeapUsedMB: 50,
            jsHeapTotalMB: 80,
            domNodesLive: 1234,
            listeners: 99,
        });
    });
});

describe("in-page harness", () => {
    it("is a single expression that parses", () => {
        const src = readFileSync(join(HERE, "lib", "bench-page.js"), "utf8");
        // Runtime.evaluate treats the source as a script; it must parse as one.
        expect(
            () =>
                new Function(
                    `return ${src
                        .replace(/^\s*\/\/.*$/gm, "")
                        .trim()
                        .replace(/;\s*$/, "")}`
                )
        ).not.toThrow();
    });
});
