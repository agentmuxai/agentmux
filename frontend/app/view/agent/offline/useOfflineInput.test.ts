// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    meta: {} as Record<string, Record<string, unknown>>,
    writes: [] as { blockId: string; meta: Record<string, unknown> }[],
}));

vi.mock("@/app/store/global", () => ({
    getBlockMetaKeyAtom: (blockId: string, key: string) => () => hub.meta[blockId]?.[key],
    getSettingsKeyAtom: () => () => undefined,
}));
vi.mock("@/app/store/block-meta", () => ({
    setBlockMeta: vi.fn(async (blockId: string, meta: Record<string, unknown>) => {
        hub.writes.push({ blockId, meta });
    }),
}));
vi.mock("@/app/store/connectivity", () => ({ connectivity: () => null, loadConnectivity: async () => {} }));
vi.mock("@/app/store/agent-pane-state-store", () => ({ addEventListener: () => () => {} }));
vi.mock("../hooks/useApiRetryState", () => ({ useApiRetryState: () => () => null }));
vi.mock("@/store/muxbus-delivery", () => ({ clockTime: String }));

import { blockMetaStorage, blockRetryMarkStorage, HELD_INPUT_META_KEY, RETRY_MARK_META_KEY } from "./useOfflineInput";

const msg = (id: string) => ({ id, text: id, heldAt: 1 });

describe("blockMetaStorage", () => {
    beforeEach(() => {
        hub.meta = {};
        hub.writes = [];
    });

    it("reads held messages saved for this agent, as after a restart", () => {
        hub.meta["b1"] = { [HELD_INPUT_META_KEY]: { agentId: "agent-a", messages: [msg("one")] } };
        expect(blockMetaStorage("b1", () => "agent-a").held()).toEqual([msg("one")]);
    });

    it("never hands one agent's held messages to another the pane switched to", () => {
        hub.meta["b2"] = { [HELD_INPUT_META_KEY]: { agentId: "agent-a", messages: [msg("one")] } };
        expect(blockMetaStorage("b2", () => "agent-b").held()).toEqual([]);
    });

    it("saves with the agent, deletes the key when nothing is held, and reads its own write before the meta catches up", () => {
        const storage = blockMetaStorage("b3", () => "agent-a");
        storage.saveHeld([msg("one")]);
        expect(hub.writes.at(-1)).toEqual({ blockId: "b3", meta: { [HELD_INPUT_META_KEY]: { agentId: "agent-a", messages: [msg("one")] } } });
        // The meta atom still reads nothing: a remount sees the write anyway.
        expect(blockMetaStorage("b3", () => "agent-a").held()).toEqual([msg("one")]);
        storage.saveHeld([]);
        expect(hub.writes.at(-1)?.meta).toEqual({ [HELD_INPUT_META_KEY]: null });
        // A stale meta copy must not bring a sent message back.
        hub.meta["b3"] = { [HELD_INPUT_META_KEY]: { agentId: "agent-a", messages: [msg("one")] } };
        expect(blockMetaStorage("b3", () => "agent-a").held()).toEqual([]);
    });
});

describe("a block's held list once no pane uses it", () => {
    beforeEach(() => {
        hub.meta = {};
        hub.writes = [];
        vi.useFakeTimers();
    });
    afterEach(() => {
        vi.useRealTimers();
    });

    it("is read from the meta again, because srv edits it while the block is parked", async () => {
        const storage = blockMetaStorage("b9", () => "agent-a");
        storage.saveHeld([msg("one"), msg("two")]);
        await vi.advanceTimersByTimeAsync(0);
        // srv sent "one" while the block was parked: the meta has only "two", marked possibly gone.
        const two = { ...msg("two"), unconfirmed: true, attempt: "srv_1" };
        hub.meta["b9"] = { [HELD_INPUT_META_KEY]: { agentId: "agent-a", messages: [two] } };
        // A pane attaching right away still reads its own write, which the meta atom may not have caught up to.
        expect(blockMetaStorage("b9", () => "agent-a").held()).toEqual([msg("one"), msg("two")]);
        await vi.advanceTimersByTimeAsync(2500);
        expect(blockMetaStorage("b9", () => "agent-a").held()).toEqual([two]);
    });
});

describe("blockRetryMarkStorage", () => {
    beforeEach(() => {
        hub.meta = {};
        hub.writes = [];
    });

    const mark = { text: "go", at: 5, auto: true, state: "sending" as const };

    it("stores the retry with its agent before it goes, and every mount sees it on its way", () => {
        const storage = blockRetryMarkStorage("r1", () => "agent-a");
        storage.saveMark(mark);
        expect(hub.writes.at(-1)).toEqual({ blockId: "r1", meta: { [RETRY_MARK_META_KEY]: { agentId: "agent-a", ...mark } } });
        expect(blockRetryMarkStorage("r1", () => "agent-a").mark()).toEqual(mark);
        storage.saveMark(null);
        expect(hub.writes.at(-1)?.meta).toEqual({ [RETRY_MARK_META_KEY]: null });
    });

    it("reads one an earlier load left on its way as possibly gone, and never another agent's", () => {
        hub.meta["r2"] = { [RETRY_MARK_META_KEY]: { agentId: "agent-a", ...mark } };
        expect(blockRetryMarkStorage("r2", () => "agent-a").mark()).toEqual({ ...mark, state: "unknown" });
        hub.meta["r3"] = { [RETRY_MARK_META_KEY]: { agentId: "agent-a", ...mark } };
        expect(blockRetryMarkStorage("r3", () => "agent-b").mark()).toBeNull();
    });
});
