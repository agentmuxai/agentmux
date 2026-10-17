// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { loadRange } from "./pdf-range";

describe("loadRange", () => {
    it("asks for an inclusive end and delivers the bytes at the start offset", async () => {
        const fetchRange = vi.fn(async () => ({ blob: new Blob([new Uint8Array([1, 2, 3])]) }));
        const deliver = vi.fn();
        const fail = vi.fn();
        await loadRange(fetchRange, 65536, 65539, deliver, fail);
        expect(fetchRange).toHaveBeenCalledWith(65536, 65538);
        expect(deliver).toHaveBeenCalledWith(65536, new Uint8Array([1, 2, 3]));
        expect(fail).not.toHaveBeenCalled();
    });

    it("reports a failed fetch instead of leaving pdf.js waiting", async () => {
        const deliver = vi.fn();
        const fail = vi.fn();
        await loadRange(async () => Promise.reject(new Error("404 Not Found")), 0, 10, deliver, fail);
        expect(deliver).not.toHaveBeenCalled();
        expect(fail).toHaveBeenCalledWith("Couldn't read part of this PDF: 404 Not Found");
    });

    it("reports a response that isn't the part asked for", async () => {
        const deliver = vi.fn();
        const fail = vi.fn();
        await loadRange(async () => ({ blob: null }), 0, 10, deliver, fail);
        expect(deliver).not.toHaveBeenCalled();
        expect(fail).toHaveBeenCalledOnce();
    });
});
