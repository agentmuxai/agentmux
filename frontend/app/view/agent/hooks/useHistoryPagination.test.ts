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

const flushMicrotasks = () => new Promise<void>((r) => setTimeout(r, 0));

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
        const onContinuationModts = vi.fn();

        createRoot((d) => {
            dispose = d;
            useHistoryPagination({
                blockId: "blk-1",
                model,
                outputFormat: () => "claude-stream-json",
                definitionId: "def-claude",
                onContinuationModts,
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

        // modts surfaced for the continuation chip.
        expect(onContinuationModts).toHaveBeenCalledTimes(1);
        expect(onContinuationModts.mock.calls[0][0]).toBeGreaterThan(0);
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
