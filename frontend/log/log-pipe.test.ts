// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/ipc", () => ({ invokeCommand: vi.fn() }));

import { invokeCommand } from "@/app/platform/ipc";
import { createLogBatcher, hostSendLogBatch, LOG_BUFFER_MAX, LOG_FLUSH_MS, type LogEntry } from "./log-pipe";

const line = (message: string): LogEntry => ({ level: "info", module: "console", message, data: null });

function harness() {
    const sent: LogEntry[][] = [];
    const resolvers: (() => void)[] = [];
    const timers: { cb: () => void; ms: number }[] = [];
    const b = createLogBatcher(
        (entries) => {
            sent.push(entries);
            return new Promise<void>((r) => resolvers.push(r));
        },
        (cb, ms) => timers.push({ cb, ms })
    );
    const fire = () => timers.shift()!.cb();
    const settle = async () => {
        resolvers.shift()!();
        await Promise.resolve();
        await Promise.resolve();
    };
    return { b, sent, timers, fire, settle };
}

describe("createLogBatcher", () => {
    it("sends the lines of one flush window in one batch", () => {
        const { b, sent, timers, fire } = harness();
        b.push(line("a"));
        b.push(line("b"));
        expect(timers).toHaveLength(1);
        expect(timers[0].ms).toBe(LOG_FLUSH_MS);
        fire();
        expect(sent).toEqual([[line("a"), line("b")]]);
    });

    it("keeps one batch in flight and sends what piled up when it returns", async () => {
        const { b, sent, timers, fire, settle } = harness();
        b.push(line("a"));
        fire();
        b.push(line("b"));
        b.push(line("c"));
        expect(timers).toHaveLength(0);
        await settle();
        expect(timers).toHaveLength(1);
        fire();
        expect(sent[1]).toEqual([line("b"), line("c")]);
    });

    it("drops the oldest lines past the cap and says how many", () => {
        const { b, sent, fire } = harness();
        for (let i = 0; i < LOG_BUFFER_MAX + 5; i++) b.push(line(`m${i}`));
        fire();
        const batch = sent[0];
        expect(batch).toHaveLength(LOG_BUFFER_MAX + 1);
        expect(batch[0].message).toContain("5 console lines dropped");
        expect(batch[1].message).toBe("m5");
        expect(batch.at(-1)!.message).toBe(`m${LOG_BUFFER_MAX + 4}`);
    });
});

describe("hostSendLogBatch", () => {
    it("sends one fe_log_batch request", async () => {
        vi.mocked(invokeCommand).mockReset().mockResolvedValue(null);
        await hostSendLogBatch()([line("a"), line("b")]);
        expect(vi.mocked(invokeCommand).mock.calls).toEqual([["fe_log_batch", { entries: [line("a"), line("b")] }]]);
    });

    it("falls back to one fe_log_structured per line on a host without the batch command", async () => {
        vi.mocked(invokeCommand)
            .mockReset()
            .mockImplementation((cmd: string) =>
                cmd === "fe_log_batch" ? Promise.reject(new Error("Unknown command: fe_log_batch")) : Promise.resolve(null)
            );
        const send = hostSendLogBatch();
        await send([line("a")]);
        await send([line("b")]);
        expect(vi.mocked(invokeCommand).mock.calls.map((c) => c[0])).toEqual([
            "fe_log_batch",
            "fe_log_structured",
            "fe_log_structured",
        ]);
    });
});
