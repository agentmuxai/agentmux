// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    calls: [] as string[],
    listeners: new Map<string, (payload: unknown) => void>(),
    checkFails: false,
}));

vi.mock("@/store/global", () => ({
    getApi: () => ({
        listen: (event: string, cb: (payload: unknown) => void) => {
            h.listeners.set(event, cb);
            return Promise.resolve(() => {});
        },
        closeRequestSeen: async (seq: number) => {
            h.calls.push(`seen:${seq}`);
        },
        closeWindow: async () => {
            h.calls.push("closeWindow");
        },
    }),
}));
vi.mock("./window-close", () => ({
    requestCloseWindow: async () => {
        if (h.checkFails) throw new Error("no agents list");
        h.calls.push("requestCloseWindow");
    },
}));

import { installOsCloseRequests, onOsCloseRequest, OS_CLOSE_REQUEST } from "./os-close";

const tick = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
    h.calls = [];
    h.listeners.clear();
    h.checkFails = false;
});

describe("an OS close the host held back", () => {
    it("is answered first, then closes the way the close button does", async () => {
        await onOsCloseRequest(7);
        expect(h.calls).toEqual(["seen:7", "requestCloseWindow"]);
    });

    it("closes the window anyway when the close check can't run", async () => {
        h.checkFails = true;
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        await onOsCloseRequest(8);
        expect(h.calls).toEqual(["seen:8", "closeWindow"]);
        warn.mockRestore();
    });

    it("arrives as the host's event", async () => {
        installOsCloseRequests();
        h.listeners.get(OS_CLOSE_REQUEST)?.({ seq: 3 });
        await tick();
        await tick();
        expect(h.calls).toEqual(["seen:3", "requestCloseWindow"]);
    });
});
