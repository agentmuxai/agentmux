// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * shellStatusCorrection — the pure decision function useShellNodeStream
 * uses to fast-correct a `shell_node_create`-spawned node whose
 * `ShellStatusCommand` check reveals it already exited (a replay of a
 * long-dead shell, not a live spawn). See
 * docs/retro/retro-activity-dock-stale-shell-flash-on-load-2026-08-22.md.
 *
 * Also covers the hook-level ordering guard (reagent P1 round 3 / codex on
 * PR #2770): the synthesized ShellStatusCommand correction and the shell's
 * REAL exit/stop event (delivered via the independently-subscribed
 * `shell:<id>` chunk ring) race with no ordering guarantee — once the real
 * one lands, the synthesized one must never overwrite it.
 */

import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
    shellStatus: vi.fn(),
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; scope: string; handler: (e: unknown) => void }) => {
        const key = `${sub.eventType}:${sub.scope}`;
        hub.handlers.set(key, sub.handler);
        return () => hub.handlers.delete(key);
    }),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ShellStatusCommand: (...args: unknown[]) => hub.shellStatus(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { WpsEvent } from "@/app/store/wps-events";
import { shellStatusCorrection, useShellNodeStream } from "./useShellNodeStream";
import type { StreamFlushQueue } from "../stream-flush-queue";

describe("shellStatusCorrection", () => {
    it("returns null when the shell is still running", () => {
        expect(shellStatusCorrection({ known: true, running: true }, 1000)).toBeNull();
    });

    // Reagent P1 round 2 on PR #2770: `known: false` means the backend has
    // no registry entry yet — this is the routine race window for a
    // genuinely live, freshly-spawned shell (shell_node_create publishes
    // BEFORE the runner registers), not a confirmed exit. Must never be
    // treated as "exited," or a real `task dev` gets misreported as failed
    // for its whole run.
    it("returns null when the backend doesn't know this shell yet (registration race)", () => {
        expect(shellStatusCorrection({ known: false, running: false }, 1000)).toBeNull();
    });

    it("maps a clean exit (code 0) to exited-ok, using the fallback timestamp", () => {
        expect(shellStatusCorrection({ known: true, running: false, exit_code: 0 }, 1000)).toEqual({
            status: "exited-ok",
            exitCode: 0,
            exitedAt: 1000,
        });
    });

    it("maps a nonzero exit code to exited-err", () => {
        expect(shellStatusCorrection({ known: true, running: false, exit_code: 1 }, 1000)).toEqual({
            status: "exited-err",
            exitCode: 1,
            exitedAt: 1000,
        });
    });

    it("maps a known-but-missing exit_code to exited-err with -1", () => {
        expect(shellStatusCorrection({ known: true, running: false }, 1000)).toEqual({
            status: "exited-err",
            exitCode: -1,
            exitedAt: 1000,
        });
    });
});

describe("useShellNodeStream — real exit vs. synthesized correction ordering", () => {
    afterEach(() => {
        hub.handlers.clear();
        hub.shellStatus.mockReset();
    });

    function setup() {
        const queue = {
            pushShellCreate: vi.fn(),
            pushShellChunk: vi.fn(),
            pushShellExit: vi.fn(),
            scheduleFlush: vi.fn(),
        } as unknown as StreamFlushQueue;
        let dispose: (() => void) | undefined;
        createRoot((d) => {
            dispose = d;
            useShellNodeStream({ blockId: "b1", queue });
        });
        const createHandler = hub.handlers.get(`${WpsEvent.ShellNodeCreate}:block:b1`);
        if (!createHandler) throw new Error("shell_node_create handler not registered");
        return { queue, createHandler, dispose: dispose! };
    }

    // Reagent P1 round 3 / codex on PR #2770: the shell exits/stops for
    // real BEFORE the ShellStatusCommand round trip resolves. The real
    // event must win — the synthesized correction must be a no-op once it
    // finally resolves, not overwrite the already-correct "stopped" row
    // with a synthesized (and here, deliberately WRONG) "exited-err".
    it("does not overwrite an already-landed real exit/stop event", async () => {
        let resolveStatus!: (v: { known: boolean; running: boolean; exit_code?: number }) => void;
        hub.shellStatus.mockReturnValue(new Promise((resolve) => { resolveStatus = resolve; }));
        const { queue, createHandler } = setup();

        createHandler({ data: { shell_id: "sh1", cmd: "task dev", timestamp: 1000 } });
        expect(queue.pushShellCreate).toHaveBeenCalledTimes(1);

        // The real exit/stop event lands first, via the per-shell scope.
        const chunkHandler = hub.handlers.get(`${WpsEvent.ShellChunk}:shell:sh1`);
        if (!chunkHandler) throw new Error("shell_chunk handler not registered");
        chunkHandler({ data: { shell_id: "sh1", op: "exit", exit_code: 0, stopped: true, timestamp: 2000 } });
        expect(queue.pushShellExit).toHaveBeenCalledTimes(1);
        expect(queue.pushShellExit).toHaveBeenLastCalledWith("sh1", "stopped", 0, 2000);

        // The status check finally resolves — with a DELIBERATELY wrong
        // "already exited with a failure" reading, simulating a stale
        // registry snapshot read before the real exit — to prove the guard
        // actually suppresses it rather than happening to agree.
        resolveStatus({ known: true, running: false, exit_code: 1 });
        await Promise.resolve();
        await Promise.resolve();

        // Still exactly one call — the real "stopped" one. Not stomped by
        // a second, synthesized "exited-err" call.
        expect(queue.pushShellExit).toHaveBeenCalledTimes(1);
        expect(queue.pushShellExit).toHaveBeenLastCalledWith("sh1", "stopped", 0, 2000);
    });

    it("still applies the correction normally when it resolves before any real event", async () => {
        let resolveStatus!: (v: { known: boolean; running: boolean; exit_code?: number }) => void;
        hub.shellStatus.mockReturnValue(new Promise((resolve) => { resolveStatus = resolve; }));
        const { queue, createHandler } = setup();

        createHandler({ data: { shell_id: "sh2", cmd: "old command", timestamp: 500 } });
        resolveStatus({ known: true, running: false, exit_code: 1 });
        await Promise.resolve();
        await Promise.resolve();

        expect(queue.pushShellExit).toHaveBeenCalledTimes(1);
        expect(queue.pushShellExit).toHaveBeenLastCalledWith("sh2", "exited-err", 1, 500);
    });
});
