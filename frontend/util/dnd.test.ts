// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { baseName, isFileDrag } from "./dnd";

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
