// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { loadLastRuntime, storeLastRuntime } from "./runtime-choice";

function stubStorage(initial: Record<string, string> = {}) {
    const data = { ...initial };
    vi.stubGlobal("localStorage", {
        getItem: (k: string) => data[k] ?? null,
        setItem: (k: string, v: string) => void (data[k] = v),
    });
    return data;
}

afterEach(() => vi.unstubAllGlobals());

describe("the remembered runtime", () => {
    it("is none until the person has chosen", () => {
        stubStorage();
        expect(loadLastRuntime()).toBeNull();
    });

    it("keeps the last explicit choice", () => {
        stubStorage();
        storeLastRuntime("host");
        expect(loadLastRuntime()).toBe("host");
        storeLastRuntime("container");
        expect(loadLastRuntime()).toBe("container");
    });

    it("ignores a stored value that is not a runtime", () => {
        stubStorage({ "agentCreate:lastRuntime": "vm" });
        expect(loadLastRuntime()).toBeNull();
    });

    it("survives storage that throws", () => {
        vi.stubGlobal("localStorage", {
            getItem: () => {
                throw new Error("denied");
            },
            setItem: () => {
                throw new Error("denied");
            },
        });
        expect(loadLastRuntime()).toBeNull();
        expect(() => storeLastRuntime("host")).not.toThrow();
    });
});
