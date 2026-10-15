// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { holdRedock, redockInProgress } from "./floating-redock-guard";

describe("the redock guard", () => {
    it("is held while the work runs and released when it settles", async () => {
        expect(redockInProgress()).toBe(false);
        let during = false;
        const result = await holdRedock(async () => {
            during = redockInProgress();
            return 7;
        });
        expect(result).toBe(7);
        expect(during).toBe(true);
        expect(redockInProgress()).toBe(false);
    });
    it("is released when the work throws, and the error still reaches the caller", async () => {
        await expect(holdRedock(async () => { throw new Error("boom"); })).rejects.toThrow("boom");
        expect(redockInProgress()).toBe(false);
    });
    it("stays held until the LAST of overlapping redocks settles", async () => {
        let releaseA!: () => void;
        const a = holdRedock(() => new Promise<void>((r) => (releaseA = r)));
        const b = holdRedock(async () => {});
        await b;
        expect(redockInProgress()).toBe(true); // A is still running
        releaseA();
        await a;
        expect(redockInProgress()).toBe(false);
    });
});
