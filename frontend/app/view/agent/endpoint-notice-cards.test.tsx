// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A changed model server is a card in every pane of the agent it names, with an Undo while the window is
 * open; the card goes when the change is undone or the window is over.
 */
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const undo = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { UndoAgentEndpointCommand: (...a: unknown[]) => undo(...a) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => {} }));
vi.mock("@/app/notification/waiting-for-you", () => ({ startWaitingForYou: () => {}, endWaitingForYou: () => {} }));
const panes = new Map<string, { agentId: string }>();
vi.mock("./agent-models", () => ({
    agentModels: {
        all: () =>
            [...panes].map(([blockId, meta]) => ({ blockId, blockAtom: () => ({ meta: { agentId: meta.agentId } }) })),
    },
}));
const present = new Map<string, ReturnType<typeof createSignal<object | undefined>>>();
vi.mock("@/app/store/global", () => ({
    MOS: {
        getMuxObjectAtom: (key: string) => {
            const id = key.replace("block:", "");
            if (!present.has(id)) present.set(id, createSignal<object | undefined>({ oid: id }));
            return present.get(id)![0];
        },
    },
}));

import { __resetPaneRequests, paneRequests } from "@/app/store/pane-requests";
import type { EndpointNoticeEvent } from "@/types/rpc/EndpointNoticeEvent";
import { blocksOf, showNotice } from "./endpoint-notice-cards";

const NOW = 1_000_000;
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
    expires_at: NOW + 30_000,
    ...over,
});
const cardsIn = (blockId: string) => paneRequests.of(blockId).filter((c) => c.source === "endpoint-notice");

beforeEach(() => {
    vi.useFakeTimers();
    undo.mockReset();
    panes.clear();
    panes.set("b1", { agentId: "lolly" });
    panes.set("b2", { agentId: "lolly" });
    panes.set("b3", { agentId: "someone-else" });
});

afterEach(() => {
    cleanup();
    __resetPaneRequests();
    vi.useRealTimers();
});

describe("blocksOf", () => {
    it("lists the panes that run the agent", () => {
        expect(blocksOf("lolly")).toEqual(["b1", "b2"]);
        expect(blocksOf("nobody")).toEqual([]);
    });
});

describe("showNotice", () => {
    it("puts one card in each pane of the agent and none in others", () => {
        showNotice(notice(), NOW);
        expect(cardsIn("b1")).toHaveLength(1);
        expect(cardsIn("b2")).toHaveLength(1);
        expect(cardsIn("b3")).toHaveLength(0);
    });

    it("does not stack a card for the same notice twice", () => {
        showNotice(notice({ id: "n2" }), NOW);
        showNotice(notice({ id: "n2" }), NOW);
        expect(cardsIn("b1")).toHaveLength(1);
    });

    it("takes the cards away when the change was undone", () => {
        showNotice(notice({ id: "n3" }), NOW);
        showNotice(notice({ id: "n3", state: "undone" }), NOW);
        expect(cardsIn("b1")).toHaveLength(0);
        expect(cardsIn("b2")).toHaveLength(0);
    });

    it("takes the cards away when the window is over", () => {
        showNotice(notice({ id: "n4" }), NOW);
        vi.advanceTimersByTime(31_500);
        expect(cardsIn("b1")).toHaveLength(0);
    });

    it("shows nothing for a notice that is already over", () => {
        showNotice(notice({ id: "n5", expires_at: NOW - 1 }), NOW);
        expect(cardsIn("b1")).toHaveLength(0);
    });

    it("does not wait on the person", () => {
        showNotice(notice({ id: "n6" }), NOW);
        expect(cardsIn("b1")[0]!.waits).toBe(false);
    });
});

describe("the card", () => {
    const draw = (id: string) => {
        showNotice(notice({ id, expires_at: Date.now() + 30_000 }));
        const card = cardsIn("b1")[0]!;
        render(() => <card.Component {...card.props} close={() => paneRequests.close(card.id)} />);
        return card;
    };

    it("says who pointed the agent where and that the sign-in stays home", () => {
        draw("n7");
        expect(screen.getByText("AgentY pointed Lolly at http://198.51.100.7:11434.")).toBeTruthy();
        expect(screen.getByText(/Claude sign-in is sent only to Anthropic/)).toBeTruthy();
    });

    it("undoes the change and closes the card", async () => {
        undo.mockResolvedValue({ restored: true });
        const card = draw("n8");
        fireEvent.click(screen.getByText(/Undo \(\d+ s\)/));
        await vi.advanceTimersByTimeAsync(0);
        expect(undo).toHaveBeenCalledWith({}, { notice_id: "n8" });
        expect(paneRequests.of("b1").some((c) => c.id === card.id)).toBe(false);
    });

    it("shows srv's reason and keeps the card when it could not undo", async () => {
        undo.mockResolvedValue({ restored: false, reason: "the environment was edited since" });
        draw("n9");
        fireEvent.click(screen.getByText(/Undo \(\d+ s\)/));
        await vi.advanceTimersByTimeAsync(0);
        expect(screen.getByText("the environment was edited since")).toBeTruthy();
        expect(cardsIn("b1")).toHaveLength(1);
    });

    it("keeps the change when asked to", () => {
        draw("n10");
        fireEvent.click(screen.getByText("Keep it"));
        expect(undo).not.toHaveBeenCalled();
        expect(cardsIn("b1")).toHaveLength(0);
    });
});
