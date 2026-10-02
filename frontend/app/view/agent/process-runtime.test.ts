// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/store/mos", () => ({ makeORef: (t: string, id: string) => `${t}:${id}` }));

import { compareRuntime, parseAgentRuntimeEvent, runtimeFlagsFromArgs, type ProcessRuntime } from "./process-runtime";
import { getProvider } from "./providers";
import { DEFAULT_RUNTIME_CONFIG, type AgentRuntimeConfig } from "./types";

const claude = getProvider("claude")!;
const run = (over: Partial<ProcessRuntime> = {}): ProcessRuntime => ({
    running: true,
    restartPending: false,
    model: "sonnet",
    effort: "high",
    permissionMode: "default",
    ...over,
});
const cmp = (requested: AgentRuntimeConfig, process: ProcessRuntime | undefined, mode = "host", flags: unknown = "") =>
    compareRuntime(claude, mode, requested, flags, process);

describe("runtimeFlagsFromArgs (mirrors spawn_runtime_from_args in agent_runtime.rs)", () => {
    it("reads the flags from the final argv", () => {
        expect(
            runtimeFlagsFromArgs(["--permission-mode", "default", "--model", "opus", "--effort", "max", "--resume", "s"]),
        ).toEqual({ permissionMode: "default", model: "opus", effort: "max" });
    });
    it("an absent flag is absent, which means the CLI default", () => {
        const f = runtimeFlagsFromArgs(["--permission-mode", "default"]);
        expect(f.model).toBeUndefined();
        expect(f.effort).toBeUndefined();
    });
    it("reads every spelling, the last of a repeated flag wins, and a bare flag has no value", () => {
        expect(runtimeFlagsFromArgs(["--model=opus"]).model).toBe("opus");
        expect(runtimeFlagsFromArgs(["-m", "gpt-5.5"]).model).toBe("gpt-5.5");
        expect(runtimeFlagsFromArgs(["--effort=low"]).effort).toBe("low");
        expect(runtimeFlagsFromArgs(["--model", "sonnet", "--model", "haiku"]).model).toBe("haiku");
        expect(runtimeFlagsFromArgs(["--dangerously-skip-permissions"]).permissionMode).toBe("bypass");
        expect(runtimeFlagsFromArgs(["--model"]).model).toBeUndefined();
    });
});

describe("parseAgentRuntimeEvent", () => {
    it("reads the server's event", () => {
        expect(
            parseAgentRuntimeEvent({ blockid: "b", running: true, model: "opus", permission_mode: "default", restart_pending: true }),
        ).toEqual({ running: true, restartPending: true, model: "opus", effort: undefined, permissionMode: "default" });
    });
    it("a process that is gone is running:false", () => {
        expect(parseAgentRuntimeEvent({ running: false, restart_pending: false })).toMatchObject({ running: false });
    });
    it("is not fooled by anything else", () => {
        expect(parseAgentRuntimeEvent(undefined)).toBeNull();
        expect(parseAgentRuntimeEvent({ model: "opus" })).toBeNull();
        expect(parseAgentRuntimeEvent("x")).toBeNull();
    });
});

describe("compareRuntime", () => {
    it("a normal pane agrees: bypass is DELIBERATELY spawned as --permission-mode default", () => {
        // The default selection is bypass/sonnet/high; its process runs
        // `--permission-mode default --model sonnet --effort high`. Comparing the
        // raw selection would flag every healthy pane.
        expect(cmp(DEFAULT_RUNTIME_CONFIG, run())).toEqual({ kind: "agrees" });
    });

    it("the incident: a process spawned with no --model/--effort under a Sonnet/high menu", () => {
        const r = cmp(DEFAULT_RUNTIME_CONFIG, run({ model: undefined, effort: undefined }));
        expect(r).toEqual({
            kind: "differs",
            drift: [
                { axis: "model", wanted: "sonnet", running: undefined },
                { axis: "effort", wanted: "high", running: undefined },
            ],
        });
    });

    it("a different model is a difference", () => {
        const r = cmp(DEFAULT_RUNTIME_CONFIG, run({ model: "opus" }));
        expect(r).toMatchObject({ kind: "differs", drift: [{ axis: "model", wanted: "sonnet", running: "opus" }] });
    });

    it("a different permission mode is a difference", () => {
        const r = cmp({ ...DEFAULT_RUNTIME_CONFIG, permissionMode: "plan" }, run());
        expect(r).toMatchObject({ kind: "differs", drift: [{ axis: "permissionMode", wanted: "plan", running: "default" }] });
    });

    it("a selection that is on its way in is pending, not a mismatch", () => {
        const r = cmp({ ...DEFAULT_RUNTIME_CONFIG, model: "opus" }, run({ restartPending: true }));
        expect(r).toMatchObject({ kind: "pending", drift: [{ axis: "model", wanted: "opus", running: "sonnet" }] });
    });

    it("Haiku takes no --effort, so no effort is missing", () => {
        const haiku = { ...DEFAULT_RUNTIME_CONFIG, model: "haiku" };
        expect(cmp(haiku, run({ model: "haiku", effort: undefined }))).toEqual({ kind: "agrees" });
    });

    it("says nothing when there is nothing to judge", () => {
        expect(cmp(DEFAULT_RUNTIME_CONFIG, undefined)).toEqual({ kind: "unknown" });
        expect(cmp(DEFAULT_RUNTIME_CONFIG, run({ running: false }))).toEqual({ kind: "unknown" });
        expect(compareRuntime(undefined, "host", DEFAULT_RUNTIME_CONFIG, "", run())).toEqual({ kind: "unknown" });
    });

    it("judges against what the args really contain, including the agent's own flags", () => {
        // provider_flags is appended after the runtime flags and wins (last
        // occurrence), so the process runs opus; the comparison sees the same
        // argv the server built and agrees with the process.
        expect(cmp(DEFAULT_RUNTIME_CONFIG, run({ model: "opus" }), "host", "--model opus")).toEqual({ kind: "agrees" });
    });
});

describe("parseAgentRuntimeEvent — what the CLI reported (get_settings)", () => {
    it("reads the effective model and effort", () => {
        const r = parseAgentRuntimeEvent({ running: true, effective_model: "claude-sonnet-5-5", effective_effort: "high" })!;
        expect(r.effectiveModel).toBe("claude-sonnet-5-5");
        expect(r.effectiveEffort).toBe("high");
    });
    it("leaves them undefined when the CLI has not reported (or has no effort)", () => {
        const r = parseAgentRuntimeEvent({ running: true, effective_model: "claude-haiku-4-5-20251001" })!;
        expect(r.effectiveEffort).toBeUndefined();
        const none = parseAgentRuntimeEvent({ running: true })!;
        expect(none.effectiveModel).toBeUndefined();
    });
});
