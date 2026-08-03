// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for `useHistoryPagination` covering the Option E
 * agent-anchored snapshot fast-path (PR #1007 backend, this PR
 * frontend).
 *
 * The hook reads the persisted-session snapshot from the agent zone
 * `agent:<defId>:current` when `opts.definitionId` is set, falling
 * back to the per-block NDJSON ring-buffer replay when the read
 * returns no content (or when no `definitionId` is passed).
 *
 * Spec: SPEC_CONTINUATION_SESSION_PERSISTENCE_2026_05_23.md.
 */

import { createRoot, type Owner } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { sleep } from "@/util/async";

vi.mock("@/app/store/rpc-api", () => {
    const RpcApi = {
        AgentSessionReadCommand: vi.fn(),
        BlockfileLineCountCommand: vi.fn(),
        BlockfileReadRangeCommand: vi.fn(),
        BlockfileReadStateCommand: vi.fn(),
    };
    return { RpcApi };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

import { useHistoryPagination } from "./useHistoryPagination";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";

const makeMockModel = (): AgentPaneModel & {
    paneEvents: any[];
    docEvents: any[];
} => {
    const paneEvents: any[] = [];
    const docEvents: any[] = [];
    const m: any = {
        blockId: "blk-1",
        disposed: false,
        dispatchPane: (cmd: any) => {
            paneEvents.push(cmd);
            return [];
        },
        dispatchDoc: (cmd: any) => {
            docEvents.push(cmd);
            return [];
        },
        paneEvents,
        docEvents,
    };
    return m as AgentPaneModel & { paneEvents: any[]; docEvents: any[] };
};

let dispose: (() => void) | null = null;

const flushMicrotasks = () => sleep(0);

beforeEach(async () => {
    vi.clearAllMocks();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
});

afterEach(() => {
    if (dispose) {
        dispose();
        dispose = null;
    }
});

describe("useHistoryPagination — Option E agent-anchored snapshot read", () => {
    it("reads from agent:session:read when definitionId is set and restores nodes", async () => {
        const snapshot = {
            schemaVersion: 1,
            savedAt: "2026-05-23T00:00:00Z",
            highWaterMark: 5,
            historyOffset: 0,
            nodes: [
                { id: "n1", type: "user_message", text: "hello" },
                { id: "n2", type: "assistant_message", text: "hi" },
            ],
        };
        vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({
            content: JSON.stringify(snapshot),
            modts: Date.now() - 3_600_000, // 1h ago
        });

        const model = makeMockModel();

        createRoot((d) => {
            dispose = d;
            useHistoryPagination({
                blockId: "blk-1",
                model,
                outputFormat: () => "claude-stream-json",
                definitionId: "def-claude",
                log: () => {},
            });
        });

        // Two microtasks: onMount + async snapshot fetch.
        await flushMicrotasks();
        await flushMicrotasks();
        await flushMicrotasks();

        expect(RpcApi.AgentSessionReadCommand).toHaveBeenCalledWith(
            {},
            { definition_id: "def-claude" },
            { timeout: 5000 },
        );

        // The state-resp from the per-block zone should NOT be invoked
        // when the agent-zone read succeeded.
        expect(RpcApi.BlockfileReadStateCommand).not.toHaveBeenCalled();

        // HistoryRestored dispatched with snapshot nodes.
        const restored = model.docEvents.find((e) => e.type === "HistoryRestored");
        expect(restored).toBeTruthy();
        expect(restored.nodes).toEqual(snapshot.nodes);
        expect(restored.fromSnapshot).toBe(true);

        // InitReady fired.
        expect(model.paneEvents.some((e) => e.type === "InitReady")).toBe(true);
    });

    it("falls through to NDJSON replay when AgentSessionRead returns no content", async () => {
        vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({
            content: null,
            modts: null,
        });
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockResolvedValue({ count: 0 });

        const model = makeMockModel();
        createRoot((d) => {
            dispose = d;
            useHistoryPagination({
                blockId: "blk-1",
                model,
                outputFormat: () => "claude-stream-json",
                definitionId: "def-claude",
                log: () => {},
            });
        });

        await flushMicrotasks();
        await flushMicrotasks();
        await flushMicrotasks();

        expect(RpcApi.AgentSessionReadCommand).toHaveBeenCalled();
        // No snapshot → falls through to line-count probe.
        expect(RpcApi.BlockfileLineCountCommand).toHaveBeenCalled();
        // No HistoryRestored — only InitReady (total=0 early-exit).
        expect(model.docEvents.find((e) => e.type === "HistoryRestored")).toBeFalsy();
        expect(model.paneEvents.some((e) => e.type === "InitReady")).toBe(true);
    });

    it("skips the snapshot fast-path entirely when definitionId is unset", async () => {
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockResolvedValue({ count: 0 });

        const model = makeMockModel();
        createRoot((d) => {
            dispose = d;
            useHistoryPagination({
                blockId: "blk-1",
                model,
                outputFormat: () => "claude-stream-json",
                // no definitionId
                log: () => {},
            });
        });

        await flushMicrotasks();
        await flushMicrotasks();

        // AgentSessionRead must NOT be called when the caller didn't
        // provide a definitionId — falls straight to NDJSON.
        expect(RpcApi.AgentSessionReadCommand).not.toHaveBeenCalled();
        expect(RpcApi.BlockfileLineCountCommand).toHaveBeenCalled();
    });
});

describe("useHistoryPagination — cross-block continuation restore (#1397)", () => {
    it("restores via the fast path (not NDJSON) for a v2 snapshot from a DIFFERENT block, when definitionId is set", async () => {
        const snapshot = {
            schemaVersion: 2,
            savedAt: "2026-07-21T00:00:00Z",
            highWaterMark: 5,
            sourceBlockId: "blk-OLD",
            documentState: {},
        };
        vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({
            content: JSON.stringify(snapshot),
            modts: Date.now() - 3_600_000,
        });
        // BlockfileLineCountCommand resolves to the agent's global output
        // zone by this block's own agentId meta (server-side), independent
        // of the snapshot's sourceBlockId — simulate that returning the
        // full cross-block history, larger than the stored hwm.
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockResolvedValue({ count: 40 });
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockResolvedValue({
            lines: ['{"type":"user","message":{"content":[{"type":"text","text":"hi"}]}}'],
            total: 40,
        });

        const model = makeMockModel();
        let hook!: ReturnType<typeof useHistoryPagination>;
        createRoot((d) => {
            dispose = d;
            hook = useHistoryPagination({
                blockId: "blk-NEW",
                model,
                outputFormat: () => "claude-stream-json",
                definitionId: "def-claude",
                log: () => {},
            });
        });

        await flushMicrotasks();
        await flushMicrotasks();
        await flushMicrotasks();

        // A successful cross-block restore must NOT permanently suppress
        // future snapshot writes from this pane (reagent P1) — its live
        // state now correctly reflects the full history via the global
        // zone, so it's safe (and expected) to keep persisting from here.
        expect(hook.snapshotIsForeignBlock()).toBe(false);

        // Fast path taken: line-count widening + range read scoped to THIS
        // (new) block, not a fall-through to legacy NDJSON replay.
        expect(RpcApi.BlockfileLineCountCommand).toHaveBeenCalledWith(
            {},
            { block_id: "blk-NEW", filename: "output" },
            { timeout: 5000 },
        );
        expect(RpcApi.BlockfileReadRangeCommand).toHaveBeenCalledWith(
            {},
            expect.objectContaining({ block_id: "blk-NEW", filename: "output" }),
            { timeout: 30_000 },
        );
        const restored = model.docEvents.find((e) => e.type === "HistoryRestored");
        expect(restored).toBeTruthy();
        expect(restored.fromSnapshot).toBe(true);
        expect(model.paneEvents.some((e) => e.type === "InitReady")).toBe(true);
    });

    it("still falls back to NDJSON replay for a cross-block v2 snapshot when no definitionId is available", async () => {
        // No definitionId passed below, so AgentSessionReadCommand's own
        // gate (opts.definitionId) is never reached — assert the hook falls
        // straight to the legacy line-count probe, same as the existing
        // "skips the snapshot fast-path entirely" case above. A cross-block
        // v2 snapshot has no way to resolve the global zone without a
        // definitionId to key it, so this path is unchanged by #1397's fix.
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockResolvedValue({ count: 0 });

        const model = makeMockModel();
        createRoot((d) => {
            dispose = d;
            useHistoryPagination({
                blockId: "blk-NEW",
                model,
                outputFormat: () => "claude-stream-json",
                // no definitionId
                log: () => {},
            });
        });

        await flushMicrotasks();
        await flushMicrotasks();

        expect(RpcApi.AgentSessionReadCommand).not.toHaveBeenCalled();
        expect(RpcApi.BlockfileLineCountCommand).toHaveBeenCalled();
        expect(model.docEvents.find((e) => e.type === "HistoryRestored")).toBeFalsy();
    });

    it("still widens a stale highWaterMark for an agent-anchored (sourceBlockId=\"\") v2 snapshot reopened on the SAME block (reagent P1 regression)", async () => {
        // Regression guard: an earlier version of the #1397 fix computed
        // `needsHwmWidening` as `!sourceMatchesThisBlock`, which treats
        // sourceBlockId==="" (agent-anchored) as "matches this block" and
        // skipped the live line-count widening probe entirely for that
        // case — silently reintroducing a stale-highWaterMark cap on the
        // pre-existing, common same-block-reopen-after-agent-anchored-write
        // path this hook already handled correctly before #1397.
        const snapshot = {
            schemaVersion: 2,
            savedAt: "2026-07-22T00:00:00Z",
            highWaterMark: 5, // stale — real global-zone total is larger
            sourceBlockId: "", // agent-anchored, not tied to one block
            documentState: {},
        };
        vi.mocked(RpcApi.AgentSessionReadCommand).mockResolvedValue({
            content: JSON.stringify(snapshot),
            modts: Date.now() - 3_600_000,
        });
        vi.mocked(RpcApi.BlockfileLineCountCommand).mockResolvedValue({ count: 500 });
        vi.mocked(RpcApi.BlockfileReadRangeCommand).mockResolvedValue({
            lines: ['{"type":"user","message":{"content":[{"type":"text","text":"hi"}]}}'],
            total: 500,
        });

        const model = makeMockModel();
        createRoot((d) => {
            dispose = d;
            useHistoryPagination({
                blockId: "blk-1", // SAME block the snapshot's read is scoped to
                model,
                outputFormat: () => "claude-stream-json",
                definitionId: "def-claude",
                log: () => {},
            });
        });

        await flushMicrotasks();
        await flushMicrotasks();
        await flushMicrotasks();

        // The widening probe must still fire — the stale hwm=5 must NOT be
        // trusted as-is when the snapshot is agent-anchored, even though
        // this pane's own blockId matches where the read is scoped.
        expect(RpcApi.BlockfileLineCountCommand).toHaveBeenCalledWith(
            {},
            { block_id: "blk-1", filename: "output" },
            { timeout: 5000 },
        );
        // The widened count (500), not the stale stored hwm (5), must drive
        // the range read's limit.
        expect(RpcApi.BlockfileReadRangeCommand).toHaveBeenCalledWith(
            {},
            expect.objectContaining({ block_id: "blk-1", filename: "output", limit: 500 }),
            { timeout: 30_000 },
        );
    });
});
