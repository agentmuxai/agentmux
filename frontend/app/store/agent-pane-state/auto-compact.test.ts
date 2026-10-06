// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, test } from "vitest";

import { autoCompactPoint, parseAutoCompactReport, sameModel, type AutoCompactReport } from "./auto-compact";
import type { ContextReading } from "./context-reading";

const reading = (over: Partial<ContextReading> = {}): ContextReading => ({
    tokens: 300_000,
    model: "claude-sonnet-5-5",
    window: 1_000_000,
    windowSource: "reported",
    source: "live",
    at: 1,
    switchedTo: null,
    ...over,
});

const report = (over: Partial<AutoCompactReport> = {}): AutoCompactReport => ({
    model: "claude-sonnet-5-5",
    window: 1_000_000,
    threshold: 967_000,
    enabled: true,
    ...over,
});

describe("parseAutoCompactReport", () => {
    test("reads the srv event", () => {
        expect(
            parseAutoCompactReport({
                blockid: "b",
                model: "claude-sonnet-5-5",
                auto_compact_window: 300_000,
                auto_compact_threshold: 267_000,
                auto_compact_enabled: true,
            }),
        ).toEqual({ model: "claude-sonnet-5-5", window: 300_000, threshold: 267_000, enabled: true });
    });

    test("a disabled report has no threshold; junk is refused", () => {
        expect(parseAutoCompactReport({ auto_compact_enabled: false, auto_compact_threshold: 967_000 })).toEqual({
            model: null,
            window: null,
            threshold: null,
            enabled: false,
        });
        expect(parseAutoCompactReport({ model: "m" })).toBeNull();
        expect(parseAutoCompactReport(null)).toBeNull();
        expect(parseAutoCompactReport({ auto_compact_enabled: true, auto_compact_threshold: -1 })?.threshold).toBeNull();
    });
});

describe("sameModel", () => {
    test("ignores case, a [1m] suffix and a date stamp", () => {
        expect(sameModel("claude-haiku-4-5-20251001", "claude-haiku-4-5")).toBe(true);
        expect(sameModel("claude-sonnet-4-6[1m]", "CLAUDE-SONNET-4-6")).toBe(true);
        expect(sameModel("claude-sonnet-4-6", "claude-sonnet-5-5")).toBe(false);
        expect(sameModel(null, "x")).toBe(false);
    });
});

describe("autoCompactPoint", () => {
    test("the CLI's point for the reading's model wins", () => {
        expect(autoCompactPoint(reading(), report({ threshold: 267_000, window: 300_000 }))).toEqual({
            kind: "at",
            tokens: 267_000,
            source: "reported",
        });
    });

    test("off when the CLI says so", () => {
        expect(autoCompactPoint(reading(), report({ enabled: false, threshold: null }))).toEqual({ kind: "off" });
    });

    test("assumed window − 33K before the CLI answered, or when it answered for another model", () => {
        const assumed = { kind: "at", tokens: 967_000, source: "assumed" };
        expect(autoCompactPoint(reading(), null)).toEqual(assumed);
        expect(autoCompactPoint(reading(), report({ model: "claude-haiku-4-5-20251001", threshold: 167_000 }))).toEqual(assumed);
    });

    test("nothing without a window, or after a model switch", () => {
        expect(autoCompactPoint(reading({ window: null, windowSource: null }), null)).toBeNull();
        expect(autoCompactPoint(reading({ window: null, windowSource: null, switchedTo: "haiku" }), report())).toBeNull();
        expect(autoCompactPoint(null, report())).toBeNull();
    });

    test("a reported point applies even when the meter's window is unknown", () => {
        // A model the table doesn't know, but the CLI says where it compacts.
        expect(autoCompactPoint(reading({ model: "x-model", window: null, windowSource: null }), report({ model: "x-model", threshold: 90_000 }))).toEqual({
            kind: "at",
            tokens: 90_000,
            source: "reported",
        });
    });
});
