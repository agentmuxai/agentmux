// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createMemoryReinjectionController } from "./memory-reinjection-controller";
import type { MemoryEntryInput } from "./memory-reinjection";

function globalEntry(label: string, body: string): MemoryEntryInput {
    return { label, source: "global", body, sizeBytes: body.length };
}
function personalEntry(label: string, body: string): MemoryEntryInput {
    return { label, source: "personal", body, sizeBytes: body.length };
}

function makeController(overrides: {
    contextWindow?: () => number;
    now?: () => number;
    dispatchTurnStart?: ReturnType<typeof vi.fn<(content: string) => void>>;
    dispatchTurnReset?: ReturnType<typeof vi.fn<() => void>>;
    sendRpc?: ReturnType<typeof vi.fn<(message: string) => Promise<void>>>;
    fetchEntries?: ReturnType<typeof vi.fn<() => Promise<MemoryEntryInput[]>>>;
} = {}) {
    // Each mock is resolved to "override if given, else default" explicitly
    // per field (rather than a generic object spread/merge) so the returned
    // value keeps its concrete Mock type instead of widening to the plain
    // function-shaped interface type — that widening was the root cause of
    // an earlier bug here: an override was correctly wired into the
    // controller, but the test then asserted against a same-named value
    // whose type no longer exposed `.mock`, so the two silently diverged.
    const contextWindow = overrides.contextWindow ?? (() => 10_000);
    const now = overrides.now ?? (() => 1_758_534_000_000);
    const dispatchTurnStart = overrides.dispatchTurnStart ?? vi.fn<(content: string) => void>();
    const dispatchTurnReset = overrides.dispatchTurnReset ?? vi.fn<() => void>();
    const sendRpc = overrides.sendRpc ?? vi.fn<(message: string) => Promise<void>>().mockResolvedValue(undefined);
    const fetchEntries =
        overrides.fetchEntries ??
        vi.fn<() => Promise<MemoryEntryInput[]>>().mockResolvedValue([globalEntry("g1", "global body")]);

    const controller = createMemoryReinjectionController({
        contextWindow,
        now,
        dispatchTurnStart,
        dispatchTurnReset,
        sendRpc,
        fetchEntries,
    });
    return { controller, dispatchTurnStart, dispatchTurnReset, sendRpc, fetchEntries };
}

describe("createMemoryReinjectionController", () => {
    it("starts NOT hiding", () => {
        const { controller } = makeController();
        expect(controller.isHiding()).toBe(false);
    });

    it("on trigger: fetches entries, dispatches TurnStart with the composed message, sends via RPC, and enters hiding", async () => {
        const { controller, dispatchTurnStart, sendRpc, fetchEntries } = makeController();

        await controller.trigger("2026-09-22T10:00:00.000Z");

        expect(fetchEntries).toHaveBeenCalledTimes(1);
        expect(dispatchTurnStart).toHaveBeenCalledTimes(1);
        const composedMessage = dispatchTurnStart.mock.calls[0][0] as string;
        expect(composedMessage).toContain("<system-reminder>");
        expect(composedMessage).toContain("global body");
        expect(sendRpc).toHaveBeenCalledWith(composedMessage);
        expect(controller.isHiding()).toBe(true);
    });

    it("does NOT dispatch or send when there are no entries — §3.1 suppression, at the trigger layer too", async () => {
        const { controller, dispatchTurnStart, sendRpc, fetchEntries } = makeController({
            fetchEntries: vi.fn().mockResolvedValue([]),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z");

        expect(fetchEntries).toHaveBeenCalledTimes(1);
        expect(dispatchTurnStart).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);
    });

    it("ignores a trigger call while already hiding — re-entrancy guard, never stacks a second hidden turn", async () => {
        const { controller, dispatchTurnStart, fetchEntries } = makeController();

        await controller.trigger("2026-09-22T10:00:00.000Z");
        expect(controller.isHiding()).toBe(true);

        await controller.trigger("2026-09-22T10:05:00.000Z");
        // Still only the first trigger's calls — the second was a no-op.
        expect(fetchEntries).toHaveBeenCalledTimes(1);
        expect(dispatchTurnStart).toHaveBeenCalledTimes(1);
    });

    it("on RPC send failure: clears hiding, calls dispatchTurnReset, never leaves the pane stuck", async () => {
        const { controller, dispatchTurnReset } = makeController({
            sendRpc: vi.fn().mockRejectedValue(new Error("network down")),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z");

        expect(dispatchTurnReset).toHaveBeenCalledTimes(1);
        expect(controller.isHiding()).toBe(false);
    });

    it("on fetchEntries failure: never dispatches TurnStart, never hides — nothing was sent, so nothing needs resetting", async () => {
        const { controller, dispatchTurnStart, dispatchTurnReset, sendRpc } = makeController({
            fetchEntries: vi.fn().mockRejectedValue(new Error("rpc down")),
        });

        await controller.trigger("2026-09-22T10:00:00.000Z");

        expect(dispatchTurnStart).not.toHaveBeenCalled();
        expect(sendRpc).not.toHaveBeenCalled();
        expect(dispatchTurnReset).not.toHaveBeenCalled();
        expect(controller.isHiding()).toBe(false);
    });

    it("onSessionEnd returns null and stays not-hiding when no hidden turn is in flight — a normal turn's session_end must be a no-op here", () => {
        const { controller } = makeController();
        expect(controller.onSessionEnd()).toBeNull();
        expect(controller.isHiding()).toBe(false);
    });

    it("onSessionEnd returns the built node and clears hiding, once, after a successful hidden turn", async () => {
        const { controller } = makeController();
        await controller.trigger("2026-09-22T10:00:00.000Z");
        expect(controller.isHiding()).toBe(true);

        const node = controller.onSessionEnd();
        expect(node).not.toBeNull();
        expect(node?.type).toBe("memory_reinjection");
        expect(node?.globalMemoryCount).toBe(1);
        expect(controller.isHiding()).toBe(false);

        // A second call (e.g. a stray extra session_end) must not resurrect
        // a stale node or leave hiding toggled back on.
        expect(controller.onSessionEnd()).toBeNull();
    });

    it("builds the node's sizeBand from Personal entries only, threading contextWindow through from the controller's own opts", async () => {
        // "x".repeat(3800) -> estimateTokenCount = 950 -> critical against a
        // 10_000-token window at the default 0.10 fraction (threshold 1000).
        const { controller } = makeController({
            fetchEntries: vi.fn().mockResolvedValue([personalEntry("p1", "x".repeat(3800))]),
        });
        await controller.trigger("2026-09-22T10:00:00.000Z");
        const node = controller.onSessionEnd();
        expect(node?.sizeBand).toBe("critical");
    });
});
