// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for muxlog.mjs's glob()/filterByInstance()/pickCandidate() —
// pure logic (filesystem reads against a real temp dir for glob(), plain
// data for the other two — no mocking needed, no network, no process.exit).
// Runs as part of `npm test` (vitest), same discipline as muxspect.test.mjs.
//
// Pins two fixes from docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md:
// - §2.2/§2.3: the channels/ log-discovery glob had one wildcard segment more
//   than any real on-disk channel-build layout actually has
//   (`channels/*/versions/*/*/logs` vs. the real `channels/*/versions/*/logs`),
//   so it silently matched zero channel-build logs on every platform, the
//   entire time that source existed — logRoots() now tries both depths.
// - §2.1: `resolveFile` picked "freshest across every instance on the
//   machine" with no way to prefer the CALLER's own running instance —
//   `swarm` resolved to a stale same-version sibling's log on the very first
//   live repro. `pickCandidate` now prefers a candidate matching the
//   caller's own $AGENTMUX_CHANNEL when no explicit `-i` is given.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { filterByInstance, glob, pickCandidate } from "./muxlog.mjs";

let root;

beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "muxlog-glob-test-"));
});

afterEach(() => {
    fs.rmSync(root, { recursive: true, force: true });
});

function makeDir(...segments) {
    const p = path.join(root, ...segments);
    fs.mkdirSync(p, { recursive: true });
    return p;
}

describe("muxlog glob", () => {
    it("matches a literal-only pattern that exists", () => {
        makeDir("a", "b", "c");
        expect(glob(path.join(root, "a", "b", "c"))).toEqual([path.join(root, "a", "b", "c")]);
    });

    it("returns empty for a literal-only pattern that doesn't exist", () => {
        expect(glob(path.join(root, "nope"))).toEqual([]);
    });

    it("matches the real 1-level channel-build depth: channels/*/versions/*/logs", () => {
        const logs = makeDir("channels", "local-main-abc123", "versions", "0.55.19", "logs");
        const found = glob(path.join(root, "channels", "*", "versions", "*", "logs"));
        expect(found).toEqual([logs]);
    });

    it("does NOT match a 2-level layout with the 1-level pattern (proves the depths are genuinely distinct)", () => {
        makeDir("channels", "local-main-abc123", "versions", "0.55.19", "extra", "logs");
        const found = glob(path.join(root, "channels", "*", "versions", "*", "logs"));
        expect(found).toEqual([]);
    });

    it("matches a 2-level layout with the 2-level pattern (in case some install shape uses it)", () => {
        const logs = makeDir("channels", "local-main-abc123", "versions", "0.55.19", "extra", "logs");
        const found = glob(path.join(root, "channels", "*", "versions", "*", "*", "logs"));
        expect(found).toEqual([logs]);
    });

    it("matches multiple channels and versions", () => {
        const a = makeDir("channels", "chan-a", "versions", "0.55.18", "logs");
        const b = makeDir("channels", "chan-a", "versions", "0.55.19", "logs");
        const c = makeDir("channels", "chan-b", "versions", "0.55.19", "logs");
        const found = glob(path.join(root, "channels", "*", "versions", "*", "logs")).sort();
        expect(found).toEqual([a, b, c].sort());
    });

    it("a sibling directory (data/cef-cache/runtime) next to logs is never picked up by the logs glob", () => {
        makeDir("channels", "chan-a", "versions", "0.55.19", "data");
        makeDir("channels", "chan-a", "versions", "0.55.19", "cef-cache");
        const logs = makeDir("channels", "chan-a", "versions", "0.55.19", "logs");
        const found = glob(path.join(root, "channels", "*", "versions", "*", "logs"));
        expect(found).toEqual([logs]);
    });
});

describe("muxlog filterByInstance", () => {
    const cands = [
        { file: "/logs/agentmuxsrv-v0.55.19.log", source: "channel:local-main-abc123", version: "0.55.19" },
        { file: "/logs/agentmuxsrv-v0.55.18.log", source: "shared", version: "0.55.18" },
        { file: "/logs/agentmux-host-v0.55.19.log", source: "dev:agenta-feature", version: "0.55.19" },
    ];

    it("matches on file path substring", () => {
        expect(filterByInstance(cands, "0.55.18")).toEqual([cands[1]]);
    });

    it("matches on source label", () => {
        expect(filterByInstance(cands, "local-main-abc123")).toEqual([cands[0]]);
    });

    it("matches on version", () => {
        expect(filterByInstance(cands, "0.55.19")).toEqual([cands[0], cands[2]]);
    });

    it("is case-insensitive across all three fields (the original inline filter was NOT — only .file was lowercased)", () => {
        expect(filterByInstance(cands, "LOCAL-MAIN-ABC123")).toEqual([cands[0]]);
        expect(filterByInstance(cands, "DEV:AGENTA-FEATURE")).toEqual([cands[2]]);
    });

    it("returns empty when nothing matches", () => {
        expect(filterByInstance(cands, "no-such-instance")).toEqual([]);
    });
});

describe("muxlog pickCandidate", () => {
    const cands = [
        { file: "/logs/agentmuxsrv-v0.55.19.log.fresh", source: "shared", version: "0.55.19" },
        { file: "/logs/channels/local-main-abc123/agentmuxsrv-v0.55.19.log.mine", source: "channel:local-main-abc123", version: "0.55.19" },
    ];

    it("an explicit -i always wins, regardless of $AGENTMUX_CHANNEL", () => {
        const opt = { instance: "local-main-abc123" };
        expect(pickCandidate(cands, opt, "some-other-channel")).toBe(cands[1].file);
    });

    it("no explicit -i: prefers a candidate matching the caller's own $AGENTMUX_CHANNEL over 'freshest first'", () => {
        // cands[0] is first in the list (would win under old "freshest/first" behavior);
        // cands[1] is what should win once we know our own channel.
        const opt = {};
        expect(pickCandidate(cands, opt, "local-main-abc123")).toBe(cands[1].file);
    });

    it("no explicit -i, no own channel found among candidates: falls back to the first (freshest) candidate", () => {
        const opt = {};
        expect(pickCandidate(cands, opt, "a-channel-with-no-log-here")).toBe(cands[0].file);
    });

    it("no explicit -i, $AGENTMUX_CHANNEL unset entirely: falls back to the first (freshest) candidate — old behavior preserved", () => {
        const opt = {};
        expect(pickCandidate(cands, opt, undefined)).toBe(cands[0].file);
    });

    it("explicit -i matching nothing returns null (caller turns this into an error+exit)", () => {
        const opt = { instance: "nonexistent" };
        expect(pickCandidate(cands, opt, "local-main-abc123")).toBeNull();
    });

    it("empty candidate list returns null", () => {
        expect(pickCandidate([], {}, "local-main-abc123")).toBeNull();
    });
});
