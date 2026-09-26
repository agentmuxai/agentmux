// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * #3868 / ReAgent P1 on #3880: a window whose init path put up the recovery
 * card used to return normally, so bootstrap logged "✅ Main application
 * loaded successfully" and reset the auto-reload budget for a window that had
 * failed. Init paths now throw StartupFailureHandled; bootstrap must treat it
 * as a failure that is already handled — no success, no budget reset, and no
 * second recovery attempt.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    initApp: vi.fn(),
    clear: vi.fn(),
    recover: vi.fn((_m: string) => true),
    logs: [] as string[],
}));

vi.mock("./log/log-pipe", () => ({ initLogPipe: () => {} }));
vi.mock("./log/error-forwarder", () => ({ initErrorForwarder: () => {} }));
vi.mock("./cef-init", () => ({ setupCefApi: async () => {} }));
vi.mock("./app-init", () => ({ initApp: () => h.initApp() }));
vi.mock("@/util/startup-bench", () => ({ benchMark: () => {} }));
vi.mock("@/perf", () => ({ initPerf: () => {} }));
vi.mock("@/app/platform/ipc", () => ({ invokeCommand: async () => {} }));
vi.mock("overlayscrollbars/overlayscrollbars.css", () => ({}));
vi.mock("./app/app.scss", () => ({}));
vi.mock("./tailwindsetup.css", () => ({}));
vi.mock("./app/init/error-display", async (orig) => {
    const real = await orig<typeof import("./app/init/error-display")>();
    return {
        StartupFailureHandled: real.StartupFailureHandled,
        clearStartupReloadCount: () => h.clear(),
        tryAutoRecover: (m: string) => h.recover(m),
    };
});

async function runBootstrap() {
    vi.resetModules();
    await import("./bootstrap");
    // bootstrap() is fire-and-forget at module load; let it settle.
    for (let i = 0; i < 20; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
}

describe("bootstrap — a handled startup failure is not a success", () => {
    beforeEach(() => {
        h.initApp.mockReset();
        h.clear.mockReset();
        h.recover.mockClear();
        h.logs.length = 0;
        vi.spyOn(console, "log").mockImplementation((...a: unknown[]) => void h.logs.push(a.map(String).join(" ")));
        vi.spyOn(console, "error").mockImplementation((...a: unknown[]) => void h.logs.push(a.map(String).join(" ")));
        vi.spyOn(console, "warn").mockImplementation(() => {});
    });

    it("does not log success, reset the reload budget, or recover again", async () => {
        const { StartupFailureHandled } = await import("./app/init/error-display");
        h.initApp.mockRejectedValue(new StartupFailureHandled('startup: client "c1" did not load'));

        await runBootstrap();

        expect(h.clear).not.toHaveBeenCalled();
        expect(h.recover).not.toHaveBeenCalled();
        expect(h.logs.join("\n")).not.toContain("loaded successfully");
        expect(h.logs.join("\n")).toContain("Window startup failed");
    });

    it("still resets the budget on a real success", async () => {
        h.initApp.mockResolvedValue(undefined);

        await runBootstrap();

        expect(h.clear).toHaveBeenCalledTimes(1);
        expect(h.logs.join("\n")).toContain("loaded successfully");
    });

    it("still auto-recovers an unhandled init failure", async () => {
        h.initApp.mockRejectedValue(new TypeError("Failed to fetch"));

        await runBootstrap();

        expect(h.recover).toHaveBeenCalledTimes(1);
        expect(h.clear).not.toHaveBeenCalled();
    });
});
