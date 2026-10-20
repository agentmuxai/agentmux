// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

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

import { AUTO_RETRY_META_KEY, blockMetaStorage, HELD_INPUT_META_KEY } from "./useOfflineInput";

const msg = (id: string) => ({ id, text: id, heldAt: 1 });

describe("blockMetaStorage", () => {
    beforeEach(() => {
        hub.meta = {};
        hub.writes = [];
    });

    it("reads held messages saved for this agent, as after a restart", () => {
        hub.meta["b1"] = { [HELD_INPUT_META_KEY]: { agentId: "agent-a", messages: [msg("one")] } };
        expect(blockMetaStorage("b1", () => "agent-a").loadHeld()).toEqual([msg("one")]);
    });

    it("never hands one agent's held messages to another the pane switched to", () => {
        hub.meta["b2"] = { [HELD_INPUT_META_KEY]: { agentId: "agent-a", messages: [msg("one")] } };
        expect(blockMetaStorage("b2", () => "agent-b").loadHeld()).toEqual([]);
    });

    it("saves with the agent, deletes the key when nothing is held, and reads its own write before the meta catches up", () => {
        const storage = blockMetaStorage("b3", () => "agent-a");
        storage.saveHeld([msg("one")]);
        expect(hub.writes.at(-1)).toEqual({ blockId: "b3", meta: { [HELD_INPUT_META_KEY]: { agentId: "agent-a", messages: [msg("one")] } } });
        // The meta atom still reads nothing: a remount sees the write anyway.
        expect(blockMetaStorage("b3", () => "agent-a").loadHeld()).toEqual([msg("one")]);
        storage.saveHeld([]);
        expect(hub.writes.at(-1)?.meta).toEqual({ [HELD_INPUT_META_KEY]: null });
        // A stale meta copy must not bring a sent message back.
        hub.meta["b3"] = { [HELD_INPUT_META_KEY]: { agentId: "agent-a", messages: [msg("one")] } };
        expect(blockMetaStorage("b3", () => "agent-a").loadHeld()).toEqual([]);
    });

    it("keeps the automatic-retry marker in the block's meta", () => {
        const storage = blockMetaStorage("b4", () => "agent-a");
        storage.saveMarker({ text: "go", at: 5 });
        expect(hub.writes.at(-1)?.meta).toEqual({ [AUTO_RETRY_META_KEY]: { text: "go", at: 5 } });
        hub.meta["b4"] = { [AUTO_RETRY_META_KEY]: { text: "go", at: 5 } };
        expect(storage.loadMarker()).toEqual({ text: "go", at: 5 });
    });
});
