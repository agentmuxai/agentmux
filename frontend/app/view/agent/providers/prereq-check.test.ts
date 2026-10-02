// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { NODE_PREREQ, PROVIDERS } from "./catalog";
import { needsProbe, prereqShortfall, shortfallLabel } from "./prereq-check";

const NODE_24_16 = { ...NODE_PREREQ, minVersion: "24.16.0" };

describe("prereq-check", () => {
    it("treats a missing tool as missing, with or without a minimum", () => {
        expect(prereqShortfall(NODE_PREREQ, { found: false })).toEqual({ kind: "missing" });
        expect(prereqShortfall(NODE_24_16, { found: false, version: null })).toEqual({ kind: "missing" });
    });

    it("flags a found tool older than its minimum, with node's v prefix", () => {
        const s = prereqShortfall(NODE_24_16, { found: true, version: "v22.22.2" });
        expect(s).toEqual({ kind: "too-old", found: "22.22.2", min: "24.16.0" });
        expect(shortfallLabel(NODE_24_16, s!)).toBe("Node.js 24.16.0+ (you have 22.22.2)");
    });

    it("passes a version at or above the minimum", () => {
        expect(prereqShortfall(NODE_24_16, { found: true, version: "24.16.0" })).toBeNull();
        expect(prereqShortfall(NODE_24_16, { found: true, version: "26.1.0" })).toBeNull();
    });

    it("doesn't block on an unreadable version, like a failed probe", () => {
        expect(prereqShortfall(NODE_24_16, { found: true, version: null })).toBeNull();
        expect(prereqShortfall(NODE_24_16, undefined)).toBeNull();
    });

    it("ignores the version for a prereq without a minimum", () => {
        expect(prereqShortfall(NODE_PREREQ, { found: true, version: "18.0.0" })).toBeNull();
    });

    it("re-probes a path-only cache entry once a minimum needs the version", () => {
        expect(needsProbe(NODE_24_16, undefined)).toBe(true);
        expect(needsProbe(NODE_24_16, { found: true })).toBe(true);
        expect(needsProbe(NODE_24_16, { found: true, version: null })).toBe(false);
        expect(needsProbe(NODE_24_16, { found: false })).toBe(false);
        expect(needsProbe(NODE_PREREQ, { found: true })).toBe(false);
    });

    it("pins OpenClaw's Node minimum to 24.16.0 and leaves other providers unversioned", () => {
        const node = (id: string) => PROVIDERS[id]?.systemPrereqs?.find((p) => p.tool === "node");
        expect(node("openclaw")?.minVersion).toBe("24.16.0");
        expect(node("claude")?.minVersion).toBeUndefined();
    });
});
