// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { pickHandler } from "./handler-map";

describe("pickHandler", () => {
    it("returns the handler for a type in the map, without logging", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        try {
            const onA = () => "a";
            expect(pickHandler("t", { A: onA }, "A")).toBe(onA);
            expect(warn).not.toHaveBeenCalled();
        } finally {
            warn.mockRestore();
        }
    });

    it("returns undefined for an unknown type and logs the label and type once", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        try {
            const map = { A: () => "a" };
            for (let i = 0; i < 3; i++) expect(pickHandler("lbl", map, "Future1")).toBeUndefined();
            expect(pickHandler("lbl", map, "Future2")).toBeUndefined();
            const lines = warn.mock.calls.map((c) => String(c[0]));
            expect(lines).toHaveLength(2);
            expect(lines[0]).toContain("[lbl]");
            expect(lines[0]).toContain("Future1");
            expect(lines[1]).toContain("Future2");
        } finally {
            warn.mockRestore();
        }
    });

    it("keeps its record per map, so two reducers each report a type", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        try {
            pickHandler("one", { A: () => 1 }, "Shared");
            pickHandler("two", { A: () => 2 }, "Shared");
            expect(warn).toHaveBeenCalledTimes(2);
        } finally {
            warn.mockRestore();
        }
    });

    it("does not take a name from Object.prototype for a handler", () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        try {
            const map = { A: () => "a" };
            for (const t of ["constructor", "toString", "hasOwnProperty", "__proto__"]) {
                expect(pickHandler("p", map, t)).toBeUndefined();
            }
        } finally {
            warn.mockRestore();
        }
    });
});
