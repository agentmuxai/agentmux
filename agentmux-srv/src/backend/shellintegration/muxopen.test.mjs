// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for muxopen.mjs's argument parsing and output rendering — pure,
// no network, no running instance. Same vitest arrangement as
// muxspect.test.mjs (NOT node:test — see that file's header for why).

import { describe, expect, it } from "vitest";

import { parseArgs, renderResult } from "./muxopen.mjs";

describe("muxopen parseArgs", () => {
    it("a bare agent name parses with defaults", () => {
        expect(parseArgs(["Scouto"])).toEqual({ agent: "Scouto", tabId: null, focus: true });
    });

    it("no arguments is help with a failing exit code", () => {
        expect(parseArgs([])).toEqual({ help: true, exitCode: 1 });
    });

    it("explicit help exits 0", () => {
        expect(parseArgs(["help"])).toEqual({ help: true, exitCode: 0 });
        expect(parseArgs(["--help"])).toEqual({ help: true, exitCode: 0 });
    });

    it("--tab captures the id and --no-focus clears focus", () => {
        expect(parseArgs(["Scouto", "--tab", "t-1", "--no-focus"])).toEqual({
            agent: "Scouto",
            tabId: "t-1",
            focus: false,
        });
    });

    it("--tab without a value is an error, not a silent undefined", () => {
        expect(parseArgs(["Scouto", "--tab"]).error).toMatch(/--tab requires/);
    });

    it("a flag in the agent position is an error rather than a bogus target", () => {
        // `muxopen --no-focus` must not try to open an agent named "--no-focus".
        expect(parseArgs(["--no-focus"]).error).toMatch(/first argument/);
    });

    it("an unknown flag is rejected loudly", () => {
        expect(parseArgs(["Scouto", "--wat"]).error).toMatch(/unknown argument '--wat'/);
    });
});

describe("muxopen renderResult", () => {
    const base = {
        agent_id: "Scouto",
        provider: "claude",
        controller_type: "subprocess",
        block_id: "b-123",
        tab_id: "t-456",
        status: "running",
    };

    it("a fresh open says 'opened' without a status line", () => {
        const out = renderResult({ ...base, created: true });
        expect(out).toContain("opened: Scouto (claude, subprocess)");
        expect(out).toContain("block b-123");
        expect(out).not.toContain("status");
    });

    it("an idempotent hit says 'already open' and shows the live status", () => {
        const out = renderResult({ ...base, created: false });
        expect(out).toContain("already open: Scouto");
        expect(out).toContain("status running");
    });
});
