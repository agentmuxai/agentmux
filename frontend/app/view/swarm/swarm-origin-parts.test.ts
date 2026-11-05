// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { LOCAL_PART_ID, localPartLabel, partsByOrigin } from "./swarm-origin-parts";

const section = (key: string, title: string) => ({ key, title });
const SECTIONS = [section("h/other", "narko · dev"), section("lan/a", "Area54"), section("lan/b", "charlie")];
const remote = new Map([
    ["r1", { section: SECTIONS[1] }],
    ["r2", { section: SECTIONS[2] }],
    ["r3", { section: SECTIONS[1] }],
]);

describe("partsByOrigin", () => {
    it("puts this machine first, then each other place in the sections' order, keeping the list's order inside each", () => {
        const parts = partsByOrigin(["r2", "w1", "r1", "w2", "r3"], remote, SECTIONS, "narko", false);
        expect(parts.map((p) => [p.label, p.keys])).toEqual([
            ["narko", ["w1", "w2"]],
            ["Area54", ["r1", "r3"]],
            ["charlie", ["r2"]],
        ]);
        expect(parts[0].id).toBe(LOCAL_PART_ID);
        expect(parts.map((p) => p.local)).toEqual([true, false, false]);
    });

    it("leaves out a place with no row left, which is how a filter hides its label", () => {
        expect(partsByOrigin(["r2"], remote, SECTIONS, "narko", false).map((p) => p.label)).toEqual(["charlie"]);
    });

    it("keeps this machine's part while it has stopped agents to list, rows or not", () => {
        expect(partsByOrigin([], remote, SECTIONS, "narko", true).map((p) => p.label)).toEqual(["narko"]);
        expect(partsByOrigin([], remote, SECTIONS, "narko", false)).toEqual([]);
    });

    it("drops a remote row whose section is gone", () => {
        const gone = new Map([["x", { section: section("gone/z", "Gone") }]]);
        expect(partsByOrigin(["x", "w1"], gone, SECTIONS, "narko", false).map((p) => p.keys)).toEqual([["w1"]]);
    });
});

describe("localPartLabel", () => {
    it("names the channel only when others run on this machine", () => {
        expect(localPartLabel("narko", "stable", false)).toBe("narko");
        expect(localPartLabel("narko", "stable", true)).toBe("narko · stable");
        expect(localPartLabel(undefined, undefined, false)).toBe("This machine");
    });
});
