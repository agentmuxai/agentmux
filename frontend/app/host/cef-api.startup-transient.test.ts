// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * #3868: a refused loopback connect (Windows WSAENOBUFS) on the very first
 * host IPC call, `get_backend_endpoints`, was read as "backend not ready yet".
 * initCefApi then waited up to 30 s for a `backend-ready` event that had fired
 * long before, with the window blank the whole time. A request that never got
 * a response is now retried, and if it keeps failing initCefApi fails fast
 * (bootstrap auto-recovers) instead of waiting for the event. A real
 * "not ready" answer from the host still waits for the event.
 */

import { beforeEach, describe, expect, test, vi } from "vitest";

const h = vi.hoisted(() => ({
    invoke: vi.fn(),
    listen: vi.fn(),
}));

vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: (cmd: string, args?: unknown) => h.invoke(cmd, args),
    listenEvent: (name: string, cb: unknown) => h.listen(name, cb),
}));
vi.mock("@/app/platform/pane-overlay", () => ({
    registerPaneOverlay: () => ({ update: () => {}, release: () => {} }),
}));

const endpoints = { ws: "127.0.0.1:2", web: "127.0.0.1:1" };
const refused = () => Promise.reject(new TypeError("Failed to fetch"));

function answer(cmd: string): Promise<unknown> {
    if (cmd === "get_backend_endpoints") return Promise.resolve(endpoints);
    if (cmd === "get_zoom_factor") return Promise.resolve(1);
    if (cmd === "get_is_dev") return Promise.resolve(true);
    if (cmd === "get_about_modal_details") return Promise.resolve({});
    return Promise.resolve("x");
}

describe("initCefApi — transient IPC failures at startup", () => {
    beforeEach(() => {
        vi.resetModules();
        vi.useRealTimers();
        h.invoke.mockReset();
        h.listen.mockReset();
        h.listen.mockResolvedValue(() => {});
    });

    test("retries a refused get_backend_endpoints instead of waiting for backend-ready", async () => {
        let refusedOnce = false;
        h.invoke.mockImplementation((cmd: string) => {
            if (cmd === "get_backend_endpoints" && !refusedOnce) {
                refusedOnce = true;
                return refused();
            }
            return answer(cmd);
        });
        const { initCefApi } = await import("./cef-api");

        await initCefApi();

        expect(h.listen).not.toHaveBeenCalledWith("backend-ready", expect.anything());
        expect(window.__WAVE_SERVER_WEB_ENDPOINT__).toBe(endpoints.web);
    });

    test("retries a refused read-only getter in the startup batch", async () => {
        let refusedOnce = false;
        h.invoke.mockImplementation((cmd: string) => {
            if (cmd === "get_platform" && !refusedOnce) {
                refusedOnce = true;
                return refused();
            }
            return answer(cmd);
        });
        const { initCefApi } = await import("./cef-api");

        await expect(initCefApi()).resolves.toBeUndefined();
        expect(h.invoke.mock.calls.filter(([c]) => c === "get_platform")).toHaveLength(2);
    });

    test("fails fast when the host stays unreachable, rather than waiting 30 s", async () => {
        vi.useFakeTimers();
        h.invoke.mockImplementation(refused);
        const { initCefApi } = await import("./cef-api");

        const settled = initCefApi().then(
            () => "resolved",
            (e) => String(e)
        );
        await vi.advanceTimersByTimeAsync(2000);

        await expect(settled).resolves.toContain("Failed to fetch");
        expect(h.listen).not.toHaveBeenCalledWith("backend-ready", expect.anything());
    });

    test("still waits for backend-ready when the host answers that it isn't ready", async () => {
        h.invoke.mockImplementation((cmd: string) =>
            cmd === "get_backend_endpoints" ? Promise.reject(new Error("backend not ready")) : answer(cmd)
        );
        h.listen.mockImplementation((name: string, cb: (p: unknown) => void) => {
            if (name === "backend-ready") setTimeout(() => cb(endpoints), 0);
            return Promise.resolve(() => {});
        });
        const { initCefApi } = await import("./cef-api");

        await initCefApi();

        expect(h.listen).toHaveBeenCalledWith("backend-ready", expect.anything());
        expect(window.__WAVE_SERVER_WEB_ENDPOINT__).toBe(endpoints.web);
    });
});
