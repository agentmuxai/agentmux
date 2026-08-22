// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for muxlog.mjs's glob() — pure filesystem reads against a real
// temp directory tree (no mocking needed, no network, no process.exit). Runs
// as part of `npm test` (vitest), same discipline as muxspect.test.mjs.
//
// Pins the fix in docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md
// §2.2/§2.3: the channels/ log-discovery glob had one wildcard segment more
// than any real on-disk channel-build layout actually has
// (`channels/*/versions/*/*/logs` vs. the real `channels/*/versions/*/logs`),
// so it silently matched zero channel-build logs on every platform, the
// entire time that source existed — logRoots() now tries both depths.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { glob } from "./muxlog.mjs";

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
