// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's ask for the go-ahead with a host agent that shares a folder with a container agent is a card in its own
 * pane. The card names the asking agent and both agents, and the person's click is the window's own answer
 * (`folderoverlapaccept`, `folderoverlapdecline`); the card goes when srv's list no longer has the pair.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const accept = vi.fn();
const decline = vi.fn();
const list = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        FolderOverlapAcceptCommand: (...a: unknown[]) => accept(...a),
        FolderOverlapDeclineCommand: (...a: unknown[]) => decline(...a),
        FolderOverlapAsksCommand: (...a: unknown[]) => list(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/agent/shared-folder-ask", async () => {
    const { RpcApi } = await import("@/app/store/rpc-api");
    return {
        goAheadWithSharedFolder: async (pairs: { host_id: string; container_id: string }[]) => {
            for (const p of pairs) await RpcApi.FolderOverlapAcceptCommand({} as never, { host_id: p.host_id, container_id: p.container_id });
        },
        declineSharedFolder: (p: { host_id: string; container_id: string }) => RpcApi.FolderOverlapDeclineCommand({} as never, { host_id: p.host_id, container_id: p.container_id }),
    };
});
let onEvent: ((data: unknown) => void) | null = null;
vi.mock("@/app/store/mps", () => ({ onMuxEvent: (_: string, f: (d: unknown) => void) => (onEvent = f) }));
vi.mock("@/app/notification/waiting-for-you", () => ({ startWaitingForYou: () => {}, endWaitingForYou: () => {} }));
// The panes that exist, by id: a test deletes one by setting it to undefined.
const panes = new Map<string, ReturnType<typeof createSignal<object | undefined>>>();
const pane = (id: string) => {
    if (!panes.has(id)) panes.set(id, createSignal<object | undefined>({ oid: id }));
    return panes.get(id)!;
};
vi.mock("@/app/store/global", () => ({
    MOS: { getMuxObjectAtom: (key: string) => pane(key.replace("block:", ""))[0] },
}));

import { paneRequests } from "@/app/store/pane-requests";
import { startFolderOverlapAsks } from "./folder-overlap-ask-cards";

const ASK = {
    host_id: "h-1",
    host_name: "Hosty",
    container_id: "c-1",
    container_name: "Aria",
    agent: "Lead",
    block_id: "b1",
    requested_ms: 1,
};

const cardOf = (blockId = "b1") => paneRequests.of(blockId).find((c) => c.source === "folder-overlap-ask");

beforeEach(async () => {
    list.mockResolvedValue({ asks: [] });
    startFolderOverlapAsks();
    onEvent!({ asks: [] });
    await Promise.resolve();
});

afterEach(() => {
    cleanup();
    accept.mockReset();
    decline.mockReset();
});

function renderCard() {
    const card = cardOf()!;
    const close = vi.fn();
    const Card = card.Component;
    render(() => <Card {...card.props} close={close} />);
    return close;
}

describe("agents' asks for the go-ahead with a shared folder", () => {
    it("an ask is one card in the asking agent's pane that only the person can answer, and the same ask twice is still one", async () => {
        onEvent!({ asks: [ASK] });
        onEvent!({ asks: [ASK] });
        await Promise.resolve();
        expect(paneRequests.of("b1").filter((c) => c.source === "folder-overlap-ask")).toHaveLength(1);
        expect(cardOf()?.presentation).toBe("card");
        expect(cardOf()?.operatorOnly).toBe(true);
        expect(cardOf()?.summary).toBe("Lead: open Hosty");
    });

    it("names the asking agent and both agents, and says the container agent can change what the host agent runs", async () => {
        onEvent!({ asks: [ASK] });
        await Promise.resolve();
        renderCard();
        expect(screen.getByLabelText("Lead asks you to open Hosty")).toBeTruthy();
        const text = screen.getByTestId("folder-overlap-ask-text").textContent ?? "";
        expect(text).toContain("Aria");
        expect(text).toMatch(/can change what Hosty runs/);
    });

    it("Go ahead is the window's go-ahead for that pair, and the card closes", async () => {
        onEvent!({ asks: [ASK] });
        await Promise.resolve();
        const close = renderCard();
        accept.mockResolvedValue({ chosen: true });
        fireEvent.click(screen.getByTestId("folder-overlap-ask-go-ahead"));
        await vi.waitFor(() => expect(accept).toHaveBeenCalledWith(expect.anything(), { host_id: "h-1", container_id: "c-1" }));
        await vi.waitFor(() => expect(close).toHaveBeenCalledTimes(1));
        expect(decline).not.toHaveBeenCalled();
    });

    it("Don't open is the window's no for that pair, and the card closes", async () => {
        onEvent!({ asks: [ASK] });
        await Promise.resolve();
        const close = renderCard();
        decline.mockResolvedValue({ declined: true });
        fireEvent.click(screen.getByTestId("folder-overlap-ask-decline"));
        await vi.waitFor(() => expect(decline).toHaveBeenCalledWith(expect.anything(), { host_id: "h-1", container_id: "c-1" }));
        await vi.waitFor(() => expect(close).toHaveBeenCalledTimes(1));
        expect(accept).not.toHaveBeenCalled();
    });

    it("shows a failed answer and stays", async () => {
        onEvent!({ asks: [ASK] });
        await Promise.resolve();
        const close = renderCard();
        accept.mockRejectedValue(new Error("srv said no"));
        fireEvent.click(screen.getByTestId("folder-overlap-ask-go-ahead"));
        expect(await screen.findByText("srv said no")).toBeTruthy();
        expect(close).not.toHaveBeenCalled();
    });

    it("goes when srv's list no longer has the pair (answered in another window, or the agent was deleted)", async () => {
        onEvent!({ asks: [ASK] });
        await Promise.resolve();
        expect(cardOf()).toBeDefined();
        onEvent!({ asks: [] });
        await Promise.resolve();
        expect(cardOf()).toBeUndefined();
    });

    it("follows the newest asker: a pair asked from another pane has its card there", async () => {
        onEvent!({ asks: [ASK] });
        await Promise.resolve();
        onEvent!({ asks: [{ ...ASK, agent: "Other", block_id: "b2" }] });
        await Promise.resolve();
        expect(cardOf("b1")).toBeUndefined();
        expect(cardOf("b2")?.summary).toBe("Other: open Hosty");
    });

    it("two pairs asked at once are two cards", async () => {
        onEvent!({ asks: [ASK, { ...ASK, host_id: "h-2", host_name: "Korp", block_id: "b3" }] });
        await Promise.resolve();
        expect(cardOf("b1")).toBeDefined();
        expect(cardOf("b3")?.summary).toBe("Lead: open Korp");
    });
});
