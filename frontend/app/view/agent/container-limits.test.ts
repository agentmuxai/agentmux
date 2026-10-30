// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { cpuDraft, cpuNotes, cpuStored, cpuText, memoryDraft, memoryNotes, memoryStored, memoryText } from "./container-limits";

describe("container limits as the panel edits them", () => {
    it("reads what srv stores: empty is auto, 0 is no limit, a number is MB or CPUs", () => {
        expect(memoryDraft("")).toEqual({ mode: "auto", amount: "" });
        expect(memoryDraft("auto")).toEqual({ mode: "auto", amount: "" });
        expect(memoryDraft("0")).toEqual({ mode: "none", amount: "" });
        expect(memoryDraft("8192")).toEqual({ mode: "custom", amount: "8" });
        expect(memoryDraft("1536")).toEqual({ mode: "custom", amount: "1.5" });
        expect(cpuDraft("2.5")).toEqual({ mode: "custom", amount: "2.5" });
    });

    it("writes the text srv stores, memory in MB for GB typed", () => {
        expect(memoryStored({ mode: "auto", amount: "" })).toEqual({ value: "" });
        expect(memoryStored({ mode: "none", amount: "" })).toEqual({ value: "0" });
        expect(memoryStored({ mode: "custom", amount: "8" })).toEqual({ value: "8192" });
        expect(memoryStored({ mode: "custom", amount: "1.5" })).toEqual({ value: "1536" });
        expect(cpuStored({ mode: "custom", amount: "6" })).toEqual({ value: "6" });
    });

    it("refuses a custom amount that is empty, zero, negative or not a number", () => {
        for (const amount of ["", "0", "-2", "lots"]) {
            expect(memoryStored({ mode: "custom", amount })).toEqual({ error: "Memory must be a number above 0" });
        }
        expect(cpuStored({ mode: "custom", amount: "x" })).toEqual({ error: "CPUs must be a number above 0" });
    });

    it("says memory in the unit a person would", () => {
        expect(memoryText(256)).toBe("256 MB");
        expect(memoryText(8192)).toBe("8 GB");
        expect(memoryText(1536)).toBe("1.5 GB");
        expect(cpuText(1)).toBe("1 CPU");
        expect(cpuText(6, 8)).toBe("6 of 8 CPUs");
    });

    it("says what is in force, and when a limit can't help", () => {
        const auto = { mode: "auto" as const, amount: "" };
        expect(memoryNotes(auto, 8192, 16384)).toEqual(["In force now: 8 GB of the 16 GB Docker has."]);
        expect(memoryNotes({ mode: "custom", amount: "32" }, 32768, 16384)).toContain("Docker has 16 GB in total, so more than that has no effect.");
        const small = memoryNotes(auto, 2048, 3072);
        expect(small.some((n) => n.includes("Raise it in Docker Desktop, Settings, Resources."))).toBe(true);
        expect(memoryNotes(auto, 8192, 16384).some((n) => n.includes("Docker Desktop"))).toBe(false);
        expect(cpuNotes(auto, 6, 8)).toEqual(["In force now: 6 of 8 CPUs."]);
        expect(cpuNotes({ mode: "none", amount: "" }, 8, 8)).toEqual(["No limit: a build can use every CPU Docker has."]);
        expect(memoryNotes({ mode: "none", amount: "" }, 16384, 16384)).toEqual(["No limit: a build can use all the memory Docker has."]);
        expect(cpuNotes({ mode: "custom", amount: "16" }, 8, 8)).toContain("Docker has 8 CPUs in total, so AgentMux uses that many.");
    });
});
