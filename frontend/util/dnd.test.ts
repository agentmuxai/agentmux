// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { baseName, copyFilesToDir, isFileDrag } from "./dnd";

const invokeCommandMock = vi.fn<(cmd: string, args: Record<string, unknown>) => Promise<unknown>>();
vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: (cmd: string, args: Record<string, unknown>) => invokeCommandMock(cmd, args),
}));

function dragEventWithTypes(types: string[]): DragEvent {
    return { dataTransfer: { types } } as unknown as DragEvent;
}

describe("isFileDrag", () => {
    it("returns true when dataTransfer.types includes Files", () => {
        expect(isFileDrag(dragEventWithTypes(["Files"]))).toBe(true);
        expect(isFileDrag(dragEventWithTypes(["text/uri-list", "Files"]))).toBe(true);
    });

    it("returns false for text/URL-only drags", () => {
        expect(isFileDrag(dragEventWithTypes(["text/plain"]))).toBe(false);
        expect(isFileDrag(dragEventWithTypes(["text/uri-list"]))).toBe(false);
        expect(isFileDrag(dragEventWithTypes([]))).toBe(false);
    });

    it("returns false when dataTransfer is missing", () => {
        expect(isFileDrag({ dataTransfer: null } as unknown as DragEvent)).toBe(false);
    });
});

describe("baseName", () => {
    it("extracts the filename from posix and windows paths", () => {
        expect(baseName("/home/user/report.csv")).toBe("report.csv");
        expect(baseName("C:\\Users\\me\\report.csv")).toBe("report.csv");
        expect(baseName("report.csv")).toBe("report.csv");
    });
});

describe("copyFilesToDir concurrency", () => {
    // Regression test for PR #2744 review discussion: the dnd:concurrency
    // setting is documented as "absent means unlimited" (schema/settings.json),
    // and the Settings UI's "leave blank for unlimited" copy depends on that
    // actually being true. A prior version of this function silently defaulted
    // an absent concurrency to 4, breaking that contract.

    it("runs all files at once (no artificial cap) when concurrency is omitted", async () => {
        let maxConcurrentInFlight = 0;
        let inFlight = 0;
        invokeCommandMock.mockImplementation(async () => {
            inFlight++;
            maxConcurrentInFlight = Math.max(maxConcurrentInFlight, inFlight);
            await new Promise((r) => setTimeout(r, 5));
            inFlight--;
            return "/dest/path";
        });

        const sources = Array.from({ length: 10 }, (_, i) => `/src/file${i}.txt`);
        await copyFilesToDir(sources, "/dest");

        expect(maxConcurrentInFlight).toBe(10);
    });

    it("still respects an explicit concurrency value", async () => {
        let maxConcurrentInFlight = 0;
        let inFlight = 0;
        invokeCommandMock.mockImplementation(async () => {
            inFlight++;
            maxConcurrentInFlight = Math.max(maxConcurrentInFlight, inFlight);
            await new Promise((r) => setTimeout(r, 5));
            inFlight--;
            return "/dest/path";
        });

        const sources = Array.from({ length: 10 }, (_, i) => `/src/file${i}.txt`);
        await copyFilesToDir(sources, "/dest", { concurrency: 3 });

        expect(maxConcurrentInFlight).toBe(3);
    });
});
