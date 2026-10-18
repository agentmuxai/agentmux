// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { TowerProcess } from "@/app/store/rpc-api";
import { describe, expect, it } from "vitest";
import { isKind, KIND_ORDER, kindGroups, KINDS, sortKindGroups } from "./tower-kinds";
import { cpuShare, groupProcesses, loadLevel, memShare } from "./tower-util";

const p = (id: number, kind: TowerProcess["kind"], cpu?: number, mem?: number): TowerProcess => ({
    id: `${id}:1`,
    pid: id,
    name: `p${id}`,
    kind,
    cpu,
    mem,
});

describe("process kinds", () => {
    it("every kind has a label and an icon, and exactly five have colour", () => {
        for (const k of KIND_ORDER) {
            expect(KINDS[k].label, k).toBeTruthy();
            expect(KINDS[k].icon, k).toBeTruthy();
        }
        expect(KIND_ORDER.filter((k) => KINDS[k].coloured)).toEqual([
            "agents",
            "agentmux",
            "shells",
            "builds",
            "runtimes",
        ]);
        expect(isKind("builds")).toBe(true);
        expect(isKind("nope")).toBe(false);
    });

    it("groups add up to every process, in kind order, a kindless row counted as an app", () => {
        const groups = kindGroups([
            p(1, "system", 0.1, 10),
            p(2, "builds", 1, 100),
            p(3, undefined, 0.5),
            p(4, "builds", 2),
        ]);
        expect(groups.map((g) => g.kind)).toEqual(["builds", "apps", "system"]);
        const builds = groups[0];
        expect(builds.processes).toHaveLength(2);
        expect(builds.cpu).toBe(3);
        expect(builds.mem).toBe(100);
        expect(groups.reduce((n, g) => n + g.processes.length, 0)).toBe(4);
    });

    it("sorted by a total, the system stays last; by name, in kind order", () => {
        const groups = kindGroups([p(1, "system", 9), p(2, "builds", 1), p(3, "browsers", 2)]);
        expect(sortKindGroups(groups, { key: "cpu", desc: true }).map((g) => g.kind)).toEqual([
            "browsers",
            "builds",
            "system",
        ]);
        expect(sortKindGroups(groups, { key: "name", desc: false }).map((g) => g.kind)).toEqual([
            "builds",
            "browsers",
            "system",
        ]);
    });
});

describe("load levels", () => {
    it("a process or agent: warning from a quarter of the machine's CPU, high from half", () => {
        expect(loadLevel("cpu", "item", 0.249)).toBeUndefined();
        expect(loadLevel("cpu", "item", 0.25)).toBe("warn");
        expect(loadLevel("cpu", "item", 0.499)).toBe("warn");
        expect(loadLevel("cpu", "item", 0.5)).toBe("high");
    });

    it("the machine: warning from 60% CPU, high from 85%; memory 70% and 90%", () => {
        expect(loadLevel("cpu", "machine", 0.59)).toBeUndefined();
        expect(loadLevel("cpu", "machine", 0.6)).toBe("warn");
        expect(loadLevel("cpu", "machine", 0.85)).toBe("high");
        expect(loadLevel("mem", "machine", 0.69)).toBeUndefined();
        expect(loadLevel("mem", "machine", 0.7)).toBe("warn");
        expect(loadLevel("mem", "machine", 0.9)).toBe("high");
        expect(loadLevel("mem", "item", 0.1)).toBe("warn");
        expect(loadLevel("mem", "item", 0.25)).toBe("high");
    });

    it("an unknown share has no level", () => {
        expect(loadLevel("cpu", "item", undefined)).toBeUndefined();
        expect(loadLevel("cpu", "item", Number.NaN)).toBeUndefined();
    });

    it("shares: CPU of the machine's cores, memory of what is installed", () => {
        expect(cpuShare(1.5, 4)).toBe(0.375);
        expect(cpuShare(undefined, 4)).toBeUndefined();
        expect(memShare(2, 8)).toBe(0.25);
        expect(memShare(2, undefined)).toBeUndefined();
    });
});

describe("app groups and kinds", () => {
    it("one executable of several kinds is a group per kind, each with its own totals", () => {
        const node = (id: number, kind: TowerProcess["kind"], cpu: number) => ({
            ...p(id, kind, cpu),
            name: "node.exe",
        });
        const groups = groupProcesses([
            node(1, "agents", 1),
            node(2, "builds", 2),
            node(3, "builds", 3),
            node(4, "runtimes", 4),
        ]);
        const byKey = new Map(groups.map((g) => [g.key, g]));
        expect([...byKey.keys()].sort()).toEqual(["node.exe#agents", "node.exe#builds", "node.exe#runtimes"]);
        expect(byKey.get("node.exe#builds")!.cpu).toBe(5);
        expect(groups.every((g) => g.name === "node.exe")).toBe(true);
    });

    it("rows without a kind group by name alone, as before", () => {
        const groups = groupProcesses([
            { ...p(1, undefined), name: "chrome.exe" },
            { ...p(2, undefined), name: "Chrome.exe" },
        ]);
        expect(groups.map((g) => g.key)).toEqual(["chrome.exe"]);
    });
});
