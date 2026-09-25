// Copyright 2026-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { sent, pushed, removed } = vi.hoisted(() => ({
    sent: [] as any[],
    pushed: [] as any[],
    removed: [] as string[],
}));

vi.mock("@/app/store/ws", () => ({ sendWSCommand: (cmd: any) => sent.push(cmd) }));
vi.mock("@/app/store/flash-notifications", () => ({
    pushNotification: (n: any) => pushed.push(n),
    removeNotificationById: (id: string) => removed.push(id),
}));

import { BlockInputSender, PASTE_CHUNK_BYTES } from "./block-input-sender";

const decode = (cmd: any) => Buffer.from(cmd.inputdata64, "base64").toString("utf8");
const frameBytes = (cmd: any) => Buffer.from(cmd.inputdata64, "base64").length;
/** Let a queued chunked send (5 ms between chunks) run to completion. */
const settle = (ms = 400) => new Promise((r) => setTimeout(r, ms));

beforeEach(() => {
    sent.length = 0;
    pushed.length = 0;
    removed.length = 0;
});
afterEach(() => vi.clearAllMocks());

describe("BlockInputSender", () => {
    it("sends small input immediately as one blockinput frame for its block", () => {
        new BlockInputSender("blk-1").send("ls\r");
        expect(sent).toHaveLength(1);
        expect(sent[0]).toMatchObject({ wscommand: "blockinput", blockid: "blk-1" });
        expect(decode(sent[0])).toBe("ls\r");
    });

    it("splits input over the chunk size into ordered frames that reassemble exactly", async () => {
        const text = "abcdefghij".repeat(1500); // 15,000 bytes
        new BlockInputSender("blk-1").send(text);
        await settle();
        expect(sent.length).toBeGreaterThan(1);
        for (const f of sent) expect(frameBytes(f)).toBeLessThanOrEqual(PASTE_CHUNK_BYTES);
        expect(sent.map(decode).join("")).toBe(text);
    });

    it("does not corrupt a multi-byte character straddling a chunk boundary", async () => {
        // 3-byte chars; 4096 % 3 !== 0, so a boundary lands mid-character.
        const text = "€".repeat(3000);
        new BlockInputSender("blk-1").send(text);
        await settle();
        const out = sent.map(decode).join("");
        expect(out).toBe(text);
        expect(out).not.toContain("�");
    });

    it("queues small input behind an in-flight chunked paste instead of slipping between its chunks", async () => {
        const sender = new BlockInputSender("blk-1");
        const big = "x".repeat(PASTE_CHUNK_BYTES * 3);
        sender.send(big);
        sender.send("\r"); // keystroke arriving mid-paste
        await settle();
        const decoded = sent.map(decode);
        expect(decoded[decoded.length - 1]).toBe("\r");
        expect(decoded.slice(0, -1).join("")).toBe(big);
    });

    it("shows a progress toast only for large chunked sends, and always removes it", async () => {
        const sender = new BlockInputSender("blk-1");
        sender.send("y".repeat(PASTE_CHUNK_BYTES + 10)); // chunked, but under the toast threshold
        await settle();
        expect(pushed).toHaveLength(0);

        sender.send("z".repeat(9 * 1024));
        await settle();
        expect(pushed).toHaveLength(1);
        expect(pushed[0]).toMatchObject({ title: "Pasting…" });
        expect(removed).toEqual([pushed[0].id]);
    });

    it("gives overlapping pastes distinct toast ids", async () => {
        const sender = new BlockInputSender("blk-1");
        sender.send("a".repeat(9 * 1024));
        sender.send("b".repeat(9 * 1024));
        await settle(800);
        expect(pushed).toHaveLength(2);
        expect(new Set(pushed.map((n) => n.id)).size).toBe(2);
        expect(removed.sort()).toEqual(pushed.map((n) => n.id).sort());
    });
});
