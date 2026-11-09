// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The notices for a changed model server (`Agents env_set`): the text says who pointed which agent where,
 * the Undo is offered only inside its window, and srv's reason is passed on when it says no.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const undo = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { UndoAgentEndpointCommand: (...a: unknown[]) => undo(...a) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
let onEvent: ((e: unknown) => void) | null = null;
vi.mock("@/app/store/mps", () => ({ onMuxEvent: (_: string, f: (e: unknown) => void) => (onEvent = f) }));

import type { EndpointNoticeEvent } from "@/types/rpc/EndpointNoticeEvent";
import {
    canUndoEndpoint,
    endpointNoticeOf,
    endpointNoticeText,
    onEndpointNotice,
    startEndpointNotices,
    undoEndpoint,
} from "./endpoint-notices";

const notice = (over: Partial<EndpointNoticeEvent> = {}): EndpointNoticeEvent => ({
    id: "n1",
    agent_id: "lolly",
    agent_slug: "lolly",
    agent_name: "Lolly",
    by: "AgentY",
    base_url: "http://198.51.100.7:11434",
    model: "flint:40b-a2b",
    cleared: false,
    state: "open",
    window_ms: 30_000,
    expires_at: 31_000,
    ...over,
});

beforeEach(() => {
    undo.mockReset();
});

describe("endpointNoticeText", () => {
    it("names who pointed the agent at which server and model", () => {
        expect(endpointNoticeText(notice())).toBe("AgentY pointed Lolly at http://198.51.100.7:11434 (model flint:40b-a2b).");
    });

    it("leaves the model out when there is none", () => {
        expect(endpointNoticeText(notice({ model: null }))).toBe("AgentY pointed Lolly at http://198.51.100.7:11434.");
    });

    it("says so when the agent went back to Anthropic", () => {
        expect(endpointNoticeText(notice({ cleared: true, base_url: null }))).toBe("AgentY pointed Lolly back at Anthropic.");
    });
});

describe("canUndoEndpoint", () => {
    it("is open inside the window only", () => {
        expect(canUndoEndpoint(notice(), 1_000)).toBe(true);
        expect(canUndoEndpoint(notice(), 31_000)).toBe(true);
        expect(canUndoEndpoint(notice(), 31_001)).toBe(false);
    });

    it("is closed once it was undone", () => {
        expect(canUndoEndpoint(notice({ state: "undone" }), 1_000)).toBe(false);
    });
});

describe("undoEndpoint", () => {
    it("asks srv with the notice id and returns null when it was restored", async () => {
        undo.mockResolvedValue({ restored: true });
        expect(await undoEndpoint("n1")).toBeNull();
        expect(undo).toHaveBeenCalledWith({}, { notice_id: "n1" });
    });

    it("passes srv's reason on", async () => {
        undo.mockResolvedValue({ restored: false, reason: "the environment was edited since" });
        expect(await undoEndpoint("n1")).toBe("the environment was edited since");
    });

    it("gives a reason of its own when srv gave none", async () => {
        undo.mockResolvedValue({ restored: false });
        expect(await undoEndpoint("n1")).toBe("it could not be undone");
    });
});

describe("startEndpointNotices", () => {
    it("keeps the latest notice per agent and tells the listeners", () => {
        startEndpointNotices();
        const heard: string[] = [];
        const stop = onEndpointNotice((e) => heard.push(`${e.id}:${e.state}`));
        onEvent!(notice());
        expect(endpointNoticeOf("lolly")?.state).toBe("open");
        onEvent!(notice({ state: "undone" }));
        expect(endpointNoticeOf("lolly")?.state).toBe("undone");
        expect(endpointNoticeOf("other")).toBeUndefined();
        expect(heard).toEqual(["n1:open", "n1:undone"]);
        stop();
        onEvent!(notice({ id: "n2" }));
        expect(heard).toHaveLength(2);
    });
});
