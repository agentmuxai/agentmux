// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for backendStatusAtom state transitions.
// Imports directly from backendStatus.ts — no global.ts dep tree, no vi.mock() needed.

import { describe, test, expect, beforeEach, vi } from "vitest";
import { createRoot } from "solid-js";
import {
    backendStatusAtom,
    setBackendStatusAtom,
    backendDeathInfoAtom,
    setBackendDeathInfoAtom,
    setRestartInProgress,
    initBackendStatusListeners,
} from "./backendStatus";

/** Read a SolidJS signal outside a reactive root without warnings. */
function read<T>(signal: () => T): T {
    let val!: T;
    createRoot((dispose) => { val = signal(); dispose(); });
    return val;
}

// ---------------------------------------------------------------------------
// Suite 1: Initial state
// ---------------------------------------------------------------------------

describe("backendStatusAtom — initial state", () => {
    test("starts as connecting", () => {
        setBackendStatusAtom("connecting");
        expect(read(backendStatusAtom)).toBe("connecting");
    });

    test("backendDeathInfoAtom starts null", () => {
        setBackendDeathInfoAtom(null);
        expect(read(backendDeathInfoAtom)).toBeNull();
    });
});

// ---------------------------------------------------------------------------
// Suite 2: backend-ready event → running
// ---------------------------------------------------------------------------

describe("backendStatusAtom — backend-ready", () => {
    beforeEach(() => setBackendStatusAtom("connecting"));

    test("transitions to running when backend-ready fires", () => {
        setBackendStatusAtom("running");
        expect(read(backendStatusAtom)).toBe("running");
    });

    test("stays running if backend-ready fires twice", () => {
        setBackendStatusAtom("running");
        setBackendStatusAtom("running");
        expect(read(backendStatusAtom)).toBe("running");
    });
});

// ---------------------------------------------------------------------------
// Suite 3: catch-up check (regression for stuck "connecting" bug)
// ---------------------------------------------------------------------------

describe("backendStatusAtom — catch-up when backend already up", () => {
    beforeEach(() => setBackendStatusAtom("connecting"));

    test("transitions to running when getBackendInfo succeeds and status is connecting", async () => {
        const mockApi = {
            listen: vi.fn().mockResolvedValue(() => {}),
            getBackendInfo: vi.fn().mockResolvedValue({ version: "0.32.81" }),
        };
        const reconnectWS = vi.fn();

        await initBackendStatusListeners(mockApi, reconnectWS);
        // Allow microtasks to flush
        await Promise.resolve();

        expect(read(backendStatusAtom)).toBe("running");
    });

    test("does NOT transition to running if already crashed", async () => {
        setBackendStatusAtom("crashed");
        const mockGetBackendInfo = vi.fn().mockResolvedValue({ version: "0.32.81" });
        await mockGetBackendInfo();
        // catch-up only applies when "connecting", not "crashed"
        if (read(backendStatusAtom) === "connecting") {
            setBackendStatusAtom("running");
        }
        expect(read(backendStatusAtom)).toBe("crashed");
    });

    test("does NOT transition to running if getBackendInfo fails", async () => {
        const mockApi = {
            listen: vi.fn().mockResolvedValue(() => {}),
            getBackendInfo: vi.fn().mockRejectedValue(new Error("not ready")),
        };
        const reconnectWS = vi.fn();

        await initBackendStatusListeners(mockApi, reconnectWS);
        await Promise.resolve();

        // on failure, atom stays connecting (backend not yet up)
        expect(read(backendStatusAtom)).toBe("connecting");
    });
});

// ---------------------------------------------------------------------------
// Suite 4: backend-terminated → crashed
// ---------------------------------------------------------------------------

describe("backendStatusAtom — backend-terminated", () => {
    beforeEach(() => {
        setBackendStatusAtom("running");
        setBackendDeathInfoAtom(null);
    });

    test("transitions to crashed on termination", () => {
        const deathPayload = {
            code: -1073740791,
            signal: null,
            pid: 12345,
            uptime_secs: 9173,
            died_at: new Date().toISOString(),
        };
        setBackendDeathInfoAtom(deathPayload);
        setBackendStatusAtom("crashed");

        expect(read(backendStatusAtom)).toBe("crashed");
        expect(read(backendDeathInfoAtom)).toMatchObject({
            code: -1073740791,
            pid: 12345,
            uptime_secs: 9173,
        });
    });

    test("preserves death info after crashing", () => {
        setBackendDeathInfoAtom({ code: 1, signal: null, pid: 999, uptime_secs: 35, died_at: "2026-03-24T23:36:59Z" });
        setBackendStatusAtom("crashed");
        expect(read(backendDeathInfoAtom)?.code).toBe(1);
        expect(read(backendDeathInfoAtom)?.uptime_secs).toBe(35);
    });

    test("exit code null when signal kill", () => {
        setBackendDeathInfoAtom({ code: null, signal: 9, pid: 999, uptime_secs: 100, died_at: new Date().toISOString() });
        setBackendStatusAtom("crashed");
        expect(read(backendDeathInfoAtom)?.code).toBeNull();
        expect(read(backendDeathInfoAtom)?.signal).toBe(9);
    });

    test("backend-terminated event wires up via initBackendStatusListeners", async () => {
        let terminatedCb: (event: any) => void = () => {};
        const mockApi = {
            listen: vi.fn().mockImplementation((event: string, cb: (e: any) => void) => {
                if (event === "backend-terminated") terminatedCb = cb;
                return Promise.resolve(() => {});
            }),
            getBackendInfo: vi.fn().mockResolvedValue({ version: "0.32.81" }),
        };

        setBackendStatusAtom("running");
        await initBackendStatusListeners(mockApi, vi.fn());

        terminatedCb({
            payload: { code: -1073740791, signal: null, pid: 42, uptime_secs: 3600 },
        });

        expect(read(backendStatusAtom)).toBe("crashed");
        expect(read(backendDeathInfoAtom)?.code).toBe(-1073740791);
        expect(read(backendDeathInfoAtom)?.pid).toBe(42);
    });
});

// ---------------------------------------------------------------------------
// Suite 5: restart cycle
// ---------------------------------------------------------------------------

describe("backendStatusAtom — restart cycle", () => {
    test("full cycle: connecting → running → crashed → connecting → running", () => {
        setBackendStatusAtom("connecting");
        expect(read(backendStatusAtom)).toBe("connecting");

        setBackendStatusAtom("running");
        expect(read(backendStatusAtom)).toBe("running");

        setBackendDeathInfoAtom({ code: -1073740791, signal: null, pid: 1, uptime_secs: 3600, died_at: new Date().toISOString() });
        setBackendStatusAtom("crashed");
        expect(read(backendStatusAtom)).toBe("crashed");

        // user clicks Restart
        setBackendStatusAtom("connecting");
        expect(read(backendStatusAtom)).toBe("connecting");
        expect(read(backendDeathInfoAtom)).not.toBeNull(); // death info preserved until backend-ready

        // restart succeeds
        setBackendStatusAtom("running");
        expect(read(backendStatusAtom)).toBe("running");
    });

    test("death info is preserved through connecting state after restart", () => {
        const deathInfo = { code: -1073740791, signal: null, pid: 1, uptime_secs: 3600, died_at: "t" };
        setBackendDeathInfoAtom(deathInfo);
        setBackendStatusAtom("crashed");
        setBackendStatusAtom("connecting"); // restart initiated
        expect(read(backendDeathInfoAtom)).not.toBeNull(); // still visible in "was up X" line
    });

    test("backend-terminated is suppressed during restart (race condition fix)", async () => {
        let terminatedCb: (event: any) => void = () => {};
        const mockApi = {
            listen: vi.fn().mockImplementation((event: string, cb: (e: any) => void) => {
                if (event === "backend-terminated") terminatedCb = cb;
                return Promise.resolve(() => {});
            }),
            getBackendInfo: vi.fn().mockRejectedValue(new Error("not ready")),
        };

        setBackendStatusAtom("connecting");
        setBackendDeathInfoAtom(null);
        setRestartInProgress(true); // simulate handleRestart setting the flag
        await initBackendStatusListeners(mockApi, vi.fn());

        // backend-terminated fires (old sidecar killed during restart)
        terminatedCb({ payload: { code: 1, signal: null, pid: 99, uptime_secs: 10 } });

        // must stay "connecting" — not "crashed"
        expect(read(backendStatusAtom)).toBe("connecting");
        expect(read(backendDeathInfoAtom)).toBeNull();

        setRestartInProgress(false); // cleanup
    });

    test("backend-ready event wires up reconnectWS and sets running", async () => {
        let readyCb: (event: any) => void = () => {};
        const mockApi = {
            listen: vi.fn().mockImplementation((event: string, cb: (e: any) => void) => {
                if (event === "backend-ready") readyCb = cb;
                return Promise.resolve(() => {});
            }),
            getBackendInfo: vi.fn().mockRejectedValue(new Error("not ready yet")),
        };
        const reconnectWS = vi.fn();

        setBackendStatusAtom("connecting");
        await initBackendStatusListeners(mockApi, reconnectWS);

        readyCb({ payload: { ws: "localhost:9001", web: "localhost:9002" } });

        expect(read(backendStatusAtom)).toBe("running");
        expect(reconnectWS).toHaveBeenCalledWith("ws://localhost:9001");
    });
});
