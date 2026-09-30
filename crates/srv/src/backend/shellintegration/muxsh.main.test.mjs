// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for muxsh.mjs's main()/apiCall() dispatch, HTTP-error handling, and
// fail()/exit-code paths — the one part of muxsh not covered by
// muxsh.test.mjs (pure parsing/rendering, "no network, no running instance"
// per its own header) or muxsh.contract.test.mjs (request-body field
// contracts). Kept in its own file for exactly that reason: these tests mock
// global fetch and process.exit, which would contradict muxsh.test.mjs's
// stated contract.
//
// See docs/reports/REPORT_MUXSH_MAIN_DISPATCH_TEST_COVERAGE_2026_09_18.md.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { main } from "./muxsh.mjs";

class FakeProcessExit extends Error {
    constructor(code) {
        super(`process.exit(${code})`);
        this.code = code;
    }
}

const ENV_KEYS = [
    "AGENTMUX_LOCAL_URL",
    "AGENTMUX_AUTH_KEY",
    "AGENTMUX_BLOCKID",
    "AGENTMUX_TABID",
    "AGENTMUX_CONFIG_DIR",
    "AGENTMUX_DATA_DIR",
    "AGENTMUX_LOG_DIR",
    "AGENTMUX_SHARED_DIR",
];

let originalArgv;
let originalEnv;
let exitSpy;
let stderrSpy;
let logSpy;

beforeEach(() => {
    originalArgv = process.argv;
    originalEnv = Object.fromEntries(ENV_KEYS.map((k) => [k, process.env[k]]));
    exitSpy = vi.spyOn(process, "exit").mockImplementation((code) => {
        throw new FakeProcessExit(code);
    });
    stderrSpy = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    logSpy = vi.spyOn(console, "log").mockImplementation(() => {});
});

afterEach(() => {
    process.argv = originalArgv;
    for (const k of ENV_KEYS) {
        if (originalEnv[k] === undefined) delete process.env[k];
        else process.env[k] = originalEnv[k];
    }
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

function setArgv(...args) {
    process.argv = ["node", "muxsh.mjs", ...args];
}

/** Sets the two vars every non-config-path command requires, plus any extra. */
function setBaseEnv(extra = {}) {
    process.env.AGENTMUX_LOCAL_URL = "http://127.0.0.1:9";
    process.env.AGENTMUX_AUTH_KEY = "test-key";
    Object.assign(process.env, extra);
}

async function expectExitCode(code) {
    await expect(main()).rejects.toBeInstanceOf(FakeProcessExit);
    expect(exitSpy).toHaveBeenCalledWith(code);
}

describe("muxsh main() — env gates", () => {
    it("missing AGENTMUX_LOCAL_URL/AGENTMUX_AUTH_KEY exits 1 with a clear message", async () => {
        setArgv("pane", "list");
        delete process.env.AGENTMUX_LOCAL_URL;
        delete process.env.AGENTMUX_AUTH_KEY;

        await expectExitCode(1);
        expect(stderrSpy).toHaveBeenCalledWith(
            expect.stringContaining("AGENTMUX_LOCAL_URL / AGENTMUX_AUTH_KEY not set"),
        );
    });

    it("'config path' does NOT require AGENTMUX_LOCAL_URL/AGENTMUX_AUTH_KEY", async () => {
        setArgv("config", "path", "data");
        delete process.env.AGENTMUX_LOCAL_URL;
        delete process.env.AGENTMUX_AUTH_KEY;
        process.env.AGENTMUX_DATA_DIR = "/tmp/data";

        await main();
        expect(logSpy).toHaveBeenCalledWith("/tmp/data");
        expect(exitSpy).not.toHaveBeenCalled();
    });

    it("'config path config'/'logs'/'shared' each read their own env var", async () => {
        setArgv("config", "path", "config");
        process.env.AGENTMUX_CONFIG_DIR = "/tmp/config";
        await main();
        expect(logSpy).toHaveBeenLastCalledWith("/tmp/config");

        setArgv("config", "path", "logs");
        process.env.AGENTMUX_LOG_DIR = "/tmp/logs";
        await main();
        expect(logSpy).toHaveBeenLastCalledWith("/tmp/logs");

        setArgv("config", "path", "shared");
        process.env.AGENTMUX_SHARED_DIR = "/tmp/shared";
        await main();
        expect(logSpy).toHaveBeenLastCalledWith("/tmp/shared");
    });

    it("'config path' for an unset dir exits 1", async () => {
        setArgv("config", "path", "data");
        delete process.env.AGENTMUX_DATA_DIR;

        await expectExitCode(1);
        expect(stderrSpy).toHaveBeenCalledWith(expect.stringContaining("AGENTMUX_DATA_DIR is not set"));
    });

    it("'config edit' without AGENTMUX_CONFIG_DIR exits 1, without ever calling fetch", async () => {
        setArgv("config", "edit");
        setBaseEnv();
        delete process.env.AGENTMUX_CONFIG_DIR;
        const fetchMock = vi.fn();
        vi.stubGlobal("fetch", fetchMock);

        await expectExitCode(1);
        expect(stderrSpy).toHaveBeenCalledWith(expect.stringContaining("AGENTMUX_CONFIG_DIR is not set"));
        expect(fetchMock).not.toHaveBeenCalled();
    });
});

describe("muxsh main() — apiCall error handling", () => {
    it("a 404 with a notFoundHint appends the 'may predate' hint", async () => {
        setArgv("pane", "list");
        setBaseEnv();
        vi.stubGlobal(
            "fetch",
            vi.fn().mockResolvedValue({ ok: false, status: 404, json: vi.fn().mockResolvedValue({}) }),
        );

        await expectExitCode(2);
        expect(stderrSpy).toHaveBeenCalledWith(
            expect.stringContaining("HTTP 404 — this AgentMux instance may predate /api/v1/tabs"),
        );
    });

    it("a non-404 error with a non-JSON body falls back to 'HTTP <status>'", async () => {
        setArgv("pane", "list");
        setBaseEnv();
        vi.stubGlobal(
            "fetch",
            vi.fn().mockResolvedValue({
                ok: false,
                status: 500,
                json: vi.fn().mockRejectedValue(new Error("not json")),
            }),
        );

        await expectExitCode(2);
        expect(stderrSpy).toHaveBeenCalledWith(expect.stringContaining("muxsh: HTTP 500"));
    });

    it("an error body's own 'error' field is used verbatim when present", async () => {
        setArgv("pane", "list");
        setBaseEnv();
        vi.stubGlobal(
            "fetch",
            vi.fn().mockResolvedValue({
                ok: false,
                status: 400,
                json: vi.fn().mockResolvedValue({ error: "block_id is required" }),
            }),
        );

        await expectExitCode(2);
        expect(stderrSpy).toHaveBeenCalledWith(expect.stringContaining("muxsh: block_id is required"));
    });

    it("a network failure (fetch rejects) exits 2 with a 'cannot reach' message, not a stack trace", async () => {
        setArgv("agent", "list");
        setBaseEnv();
        vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("ECONNREFUSED")));

        await expectExitCode(2);
        expect(stderrSpy).toHaveBeenCalledWith(
            expect.stringContaining("cannot reach http://127.0.0.1:9: ECONNREFUSED"),
        );
    });
});

describe("muxsh main() — agent-send success flag", () => {
    it("HTTP 200 with success:false still exits 2 (delivery failure isn't an HTTP error)", async () => {
        setArgv("agent", "send", "Scouto", "hello");
        setBaseEnv();
        vi.stubGlobal(
            "fetch",
            vi.fn().mockResolvedValue({
                ok: true,
                status: 200,
                json: vi.fn().mockResolvedValue({ success: false, error: "agent not found" }),
            }),
        );

        await expectExitCode(2);
        expect(stderrSpy).toHaveBeenCalledWith(expect.stringContaining("muxsh: not sent: agent not found"));
    });

    it("HTTP 200 with success:true exits 0 and prints 'sent'", async () => {
        setArgv("agent", "send", "Scouto", "hello");
        setBaseEnv();
        vi.stubGlobal(
            "fetch",
            vi.fn().mockResolvedValue({ ok: true, status: 200, json: vi.fn().mockResolvedValue({ success: true }) }),
        );

        await main();
        expect(exitSpy).not.toHaveBeenCalled();
        expect(logSpy).toHaveBeenCalledWith("sent");
    });
});

describe("muxsh main() — dispatch reaches the right route", () => {
    function okFetch(body) {
        const fetchMock = vi.fn().mockResolvedValue({ ok: true, status: 200, json: vi.fn().mockResolvedValue(body) });
        vi.stubGlobal("fetch", fetchMock);
        return fetchMock;
    }

    it("'open' -> POST /api/v1/pane/open", async () => {
        setArgv("open", "/tmp/foo.md");
        setBaseEnv();
        const fetchMock = okFetch({ view: "editor", block_id: "b-1", tab_id: "t-1" });
        await main();
        expect(fetchMock).toHaveBeenCalledWith(
            expect.stringContaining("/api/v1/pane/open"),
            expect.objectContaining({ method: "POST" }),
        );
    });

    it("'web' -> POST /api/v1/pane/open", async () => {
        setArgv("web", "https://example.com");
        setBaseEnv();
        const fetchMock = okFetch({ view: "browser", block_id: "b-1", tab_id: "t-1" });
        await main();
        expect(fetchMock).toHaveBeenCalledWith(
            expect.stringContaining("/api/v1/pane/open"),
            expect.objectContaining({ method: "POST" }),
        );
    });

    it("'config edit' -> POST /api/v1/pane/open", async () => {
        setArgv("config", "edit");
        setBaseEnv({ AGENTMUX_CONFIG_DIR: "/tmp/config" });
        const fetchMock = okFetch({ view: "editor", block_id: "b-1", tab_id: "t-1" });
        await main();
        expect(fetchMock).toHaveBeenCalledWith(
            expect.stringContaining("/api/v1/pane/open"),
            expect.objectContaining({ method: "POST" }),
        );
    });

    it("'pane list' -> GET /api/v1/tabs", async () => {
        setArgv("pane", "list");
        setBaseEnv();
        const fetchMock = okFetch({ tabs: [] });
        await main();
        expect(fetchMock).toHaveBeenCalledWith(expect.stringContaining("/api/v1/tabs"), expect.objectContaining({ method: "GET" }));
    });

    it("'run <cmd>' -> POST /api/v1/shell/create", async () => {
        setArgv("run", "echo", "hi");
        setBaseEnv({ AGENTMUX_BLOCKID: "b-1" });
        const fetchMock = okFetch({ shell_id: "s-1" });
        await main();
        expect(fetchMock).toHaveBeenCalledWith(
            expect.stringContaining("/api/v1/shell/create"),
            expect.objectContaining({ method: "POST" }),
        );
    });

    it("'run --status' -> POST /api/v1/shell/status", async () => {
        setArgv("run", "--status", "s-1");
        setBaseEnv();
        const fetchMock = okFetch({ running: true, line_count: 0 });
        await main();
        expect(fetchMock).toHaveBeenCalledWith(
            expect.stringContaining("/api/v1/shell/status"),
            expect.objectContaining({ method: "POST" }),
        );
    });

    it("'run --stop' -> POST /api/v1/shell/stop", async () => {
        setArgv("run", "--stop", "s-1");
        setBaseEnv();
        const fetchMock = okFetch({ stopped: true });
        await main();
        expect(fetchMock).toHaveBeenCalledWith(
            expect.stringContaining("/api/v1/shell/stop"),
            expect.objectContaining({ method: "POST" }),
        );
    });

    it("'agent list' -> GET /agentmux/discovery", async () => {
        setArgv("agent", "list");
        setBaseEnv();
        const fetchMock = okFetch({ host: { agents: [], cross_channel: [] }, lan: [] });
        await main();
        expect(fetchMock).toHaveBeenCalledWith(
            expect.stringContaining("/agentmux/discovery"),
            expect.objectContaining({ method: "GET" }),
        );
    });

    it("'agent send' -> POST /agentmux/reactive/inject", async () => {
        setArgv("agent", "send", "Scouto", "hi");
        setBaseEnv();
        const fetchMock = okFetch({ success: true });
        await main();
        expect(fetchMock).toHaveBeenCalledWith(
            expect.stringContaining("/agentmux/reactive/inject"),
            expect.objectContaining({ method: "POST" }),
        );
    });

    it("'run-create' without AGENTMUX_BLOCKID exits 1 without calling fetch", async () => {
        setArgv("run", "echo", "hi");
        setBaseEnv();
        delete process.env.AGENTMUX_BLOCKID;
        const fetchMock = vi.fn();
        vi.stubGlobal("fetch", fetchMock);

        await expectExitCode(1);
        expect(stderrSpy).toHaveBeenCalledWith(expect.stringContaining("AGENTMUX_BLOCKID is not set"));
        expect(fetchMock).not.toHaveBeenCalled();
    });
});
