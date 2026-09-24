// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: vi.fn(() => vi.fn()) }));

import { muxEventSubscribe } from "@/app/store/mps";

import { createInstallSession, MAX_LOG_LINES, type InstallSessionOptions } from "./install-session";
import { NpmStepTracker } from "./npm-steps";

afterEach(() => vi.clearAllMocks());

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

/** Creates a session in its own root; returns it, a sender for install_chunk data, and dispose. */
function setup(over: Partial<InstallSessionOptions> = {}) {
    let dispose!: () => void;
    const session = createRoot((d) => {
        dispose = d;
        return createInstallSession({
            tracker: () => new NpmStepTracker("Pi"),
            begin: vi.fn().mockResolvedValue({ sessionId: "sess-1" }),
            cancel: vi.fn().mockResolvedValue({}),
            ...over,
        });
    });
    const send = (data: Record<string, unknown>) => {
        const call = vi.mocked(muxEventSubscribe).mock.calls.at(-1)![0] as { handler: (e: unknown) => void };
        call.handler({ data: { sessionId: "sess-1", ...data } });
    };
    return { session, send, dispose };
}

describe("createInstallSession", () => {
    it("shows the idle plan, then subscribes to its own session scope on start", async () => {
        const { session, dispose } = setup();
        expect(session.state()).toBe("idle");
        expect(session.steps().every((s) => s.status === "pending")).toBe(true);

        await session.start();
        expect(session.state()).toBe("running");
        expect(session.sessionId()).toBe("sess-1");
        expect(muxEventSubscribe).toHaveBeenCalledWith(
            expect.objectContaining({ eventType: "install_chunk", scope: "install:sess-1" }),
        );
        dispose();
    });

    it("feeds lines through the tracker into steps and the tone-tagged log", async () => {
        const onDone = vi.fn();
        const { session, send, dispose } = setup({ onDone });
        await session.start();
        send({ line: "npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms", stream: "stderr" });
        send({ line: "npm warn deprecated x@1: gone", stream: "stderr" });
        expect(session.steps().find((s) => s.id === "download")?.status).toBe("active");
        expect(session.lines().map((l) => l.tone)).toEqual(["normal", "warning"]);

        send({ op: "done", ok: true });
        expect(session.state()).toBe("done");
        expect(onDone).toHaveBeenCalledWith(true);
        // Late lines after done are ignored.
        send({ line: "stray", stream: "stdout" });
        expect(session.lines()).toHaveLength(2);
        dispose();
    });

    it("on failure records the Layer 1 failure and appends the backend's reason to the log", async () => {
        const { session, send, dispose } = setup();
        await session.start();
        send({ line: "npm error code ENOTFOUND", stream: "stderr" });
        send({ op: "done", ok: false, error: "npm exited Some(1)" });
        expect(session.state()).toBe("failed");
        expect(session.failure()).toMatchObject({ category: "network", firstErrorLine: 0 });
        expect(session.lines().at(-1)).toEqual({ text: "Install failed: npm exited Some(1)", tone: "error" });
        dispose();
    });

    it("explains a start RPC that rejects", async () => {
        const { session, dispose } = setup({ begin: vi.fn().mockRejectedValue(new Error("install already in progress")) });
        await session.start();
        expect(session.state()).toBe("failed");
        expect(session.lines().map((l) => l.text)).toEqual(["Install failed: install already in progress"]);
        expect(muxEventSubscribe).not.toHaveBeenCalled();
        dispose();
    });

    it("trims the oldest lines past the cap and says so in Copy all", async () => {
        const { session, send, dispose } = setup();
        await session.start();
        for (let i = 0; i <= MAX_LOG_LINES; i++) send({ line: `line ${i}`, stream: "stdout" });
        expect(session.lines().length).toBeLessThanOrEqual(MAX_LOG_LINES);
        expect(session.trimmedLines()).toBeGreaterThan(0);
        expect(session.lines().at(-1)?.text).toBe(`line ${MAX_LOG_LINES}`);
        expect(session.logText().split("\n")[0]).toBe(`[${session.trimmedLines()} earlier lines trimmed]`);
        dispose();
    });

    it("Retry starts clean", async () => {
        const { session, send, dispose } = setup();
        await session.start();
        send({ line: "npm error code ENOTFOUND", stream: "stderr" });
        send({ op: "done", ok: false, error: "npm exited Some(1)" });
        await session.start();
        expect(session.state()).toBe("running");
        expect(session.lines()).toHaveLength(0);
        expect(session.failure()).toBeNull();
        expect(session.steps()[0]).toMatchObject({ id: "requirements", status: "active" });
        dispose();
    });

    it("cancels a running npm install on unmount only when asked to", async () => {
        const cancel = vi.fn().mockResolvedValue({});
        const a = setup({ cancel, cancelOnDispose: true });
        await a.session.start();
        a.dispose();
        expect(cancel).toHaveBeenCalledWith("sess-1");

        cancel.mockClear();
        const b = setup({ cancel, cancelOnDispose: false });
        await b.session.start();
        b.dispose();
        expect(cancel).not.toHaveBeenCalled();
    });

    it("cancels a session whose start RPC resolved after unmount", async () => {
        let resolveBegin!: (v: { sessionId: string }) => void;
        const cancel = vi.fn().mockResolvedValue({});
        const { session, dispose } = setup({
            cancel,
            cancelOnDispose: true,
            begin: () => new Promise((r) => (resolveBegin = r)),
        });
        const started = session.start();
        dispose();
        resolveBegin({ sessionId: "late" });
        await started;
        await flush();
        expect(cancel).toHaveBeenCalledWith("late");
        expect(muxEventSubscribe).not.toHaveBeenCalled();
    });

    it("has no cancel when the install can't be stopped safely", async () => {
        const { session, dispose } = setup({ cancel: undefined });
        await session.start();
        expect(session.canCancel()).toBe(false);
        dispose();
    });
});
