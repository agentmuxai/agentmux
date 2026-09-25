// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Handler = (event: MuxEvent) => void;
const handlers = new Map<string, Handler>();
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (...subs: { eventType: string; handler: Handler }[]) => {
        for (const s of subs) handlers.set(s.eventType, s.handler);
        return () => {};
    },
}));
let resolveHistory: Array<(events: MuxEvent[]) => void> = [];
const keepCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        EventReadHistoryCommand: () => new Promise<MuxEvent[]>((r) => resolveHistory.push(r)),
        AgentShutdownKeepCommand: (...a: unknown[]) => keepCommand(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const tone = vi.fn();
vi.mock("@/app/notification/sound/sound-service", () => ({ playShutdownPendingTone: () => tone() }));

import { ShutdownPendingBanner } from "./ShutdownPendingBanner";

const flush = () => new Promise((r) => setTimeout(r, 0));

const pending = (id: string) =>
    ({
        event: "agent:shutdown-pending",
        data: { request_id: id, block_id: "b1", by: "Korp", via: "ClosePane", reason: "stuck", deadline_ms: Date.now() + 15_000 },
    }) as unknown as MuxEvent;
const cleared = (id: string) =>
    ({ event: "agent:shutdown-pending-cleared", data: { request_id: id, outcome: "kept_by_user" } }) as unknown as MuxEvent;

describe("ShutdownPendingBanner (SPEC_AGENT_SELF_QUIT §6.5)", () => {
    beforeEach(() => {
        handlers.clear();
        resolveHistory = [];
        vi.clearAllMocks();
    });
    afterEach(cleanup);

    const mount = () => render(() => <ShutdownPendingBanner blockId="b1" agentId="camper" agentName="Camper" />);

    it("shows the live request with a countdown and chimes", () => {
        mount();
        handlers.get("agent:shutdown-pending")!(pending("r1"));
        expect(screen.getByText(/Korp asked to shut down Camper: stuck\. Closing in 1[45] s\./)).toBeTruthy();
        expect(tone).toHaveBeenCalledTimes(1);
    });

    it("a pane opened mid-countdown shows it from history, without a chime", async () => {
        mount();
        resolveHistory[0]([pending("r1")]);
        resolveHistory[1]([]);
        await flush();
        expect(screen.queryByText(/Korp asked to shut down Camper/)).toBeTruthy();
        expect(tone).not.toHaveBeenCalled();
    });

    it("a history replay that lands after a live -cleared doesn't bring the banner back", async () => {
        mount();
        handlers.get("agent:shutdown-pending")!(pending("r1"));
        handlers.get("agent:shutdown-pending-cleared")!(cleared("r1"));
        expect(screen.queryByText(/Korp asked/)).toBeNull();
        // The stale replay resolves now, still saying r1 is pending.
        resolveHistory[0]([pending("r1")]);
        resolveHistory[1]([]);
        await flush();
        expect(screen.queryByText(/Korp asked/)).toBeNull();
    });

    it("Keep running asks srv once and hides the banner once kept", async () => {
        keepCommand.mockResolvedValue({ outcome: "kept_by_user" });
        mount();
        handlers.get("agent:shutdown-pending")!(pending("r1"));
        const button = screen.getByText("Keep running");
        fireEvent.click(button);
        fireEvent.click(button);
        expect(keepCommand).toHaveBeenCalledTimes(1);
        expect(keepCommand.mock.calls[0][1]).toEqual({ blockid: "b1", request_id: "r1" });
        await flush();
        expect(screen.queryByText(/Korp asked/)).toBeNull();
    });
});
