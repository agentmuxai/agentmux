// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * My Agents' line for a changed model server: who pointed the agent where, and an Undo only while the
 * window is open.
 */
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const undo = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { UndoAgentEndpointCommand: (...a: unknown[]) => undo(...a) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
let onEvent: ((e: unknown) => void) | null = null;
vi.mock("@/app/store/mps", () => ({ onMuxEvent: (_: string, f: (e: unknown) => void) => (onEvent = f) }));

import { startEndpointNotices } from "@/app/store/endpoint-notices";
import type { EndpointNoticeEvent } from "@/types/rpc/EndpointNoticeEvent";
import { EndpointNoticeLine } from "./EndpointNoticeLine";

const notice = (over: Partial<EndpointNoticeEvent> = {}): EndpointNoticeEvent => ({
    id: "n1",
    agent_id: "lolly",
    agent_slug: "lolly",
    agent_name: "Lolly",
    by: "AgentY",
    base_url: "http://198.51.100.7:11434",
    model: null,
    cleared: false,
    state: "open",
    window_ms: 30_000,
    expires_at: Date.now() + 30_000,
    ...over,
});

beforeEach(() => {
    undo.mockReset();
    startEndpointNotices();
});

afterEach(() => cleanup());

describe("EndpointNoticeLine", () => {
    it("shows nothing until a notice names the agent", () => {
        render(() => <EndpointNoticeLine agentId="lolly" />);
        expect(screen.queryByTestId("agent-env-endpoint-notice")).toBeNull();
    });

    it("says who pointed the agent where, with an Undo that asks srv", async () => {
        undo.mockResolvedValue({ restored: true });
        render(() => <EndpointNoticeLine agentId="lolly" />);
        onEvent!(notice());
        expect((await screen.findByTestId("agent-env-endpoint-notice")).textContent).toContain(
            "AgentY pointed Lolly at http://198.51.100.7:11434."
        );
        fireEvent.click(screen.getByText("Undo"));
        expect(undo).toHaveBeenCalledWith({}, { notice_id: "n1" });
    });

    it("offers no Undo once the window is over", async () => {
        render(() => <EndpointNoticeLine agentId="lolly" />);
        onEvent!(notice({ expires_at: Date.now() - 1 }));
        await screen.findByTestId("agent-env-endpoint-notice");
        expect(screen.queryByText("Undo")).toBeNull();
    });

    it("drops the line when the change was undone", async () => {
        render(() => <EndpointNoticeLine agentId="lolly" />);
        onEvent!(notice({ id: "n2" }));
        await screen.findByTestId("agent-env-endpoint-notice");
        onEvent!(notice({ id: "n2", state: "undone" }));
        expect(screen.queryByTestId("agent-env-endpoint-notice")).toBeNull();
    });
});
