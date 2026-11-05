// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's request to install a widget is a card in its own pane (a window prompt when it names no pane);
 * the user's answer goes to srv through the host
 * (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 5).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal, type Component } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const decideWidget = vi.fn();
const list = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { WidgetsRequestsCommand: (...a: unknown[]) => list(...a) },
}));
vi.mock("@/util/app-api", () => ({
    getApi: () => ({ approvals: { decideWidget: (...a: unknown[]) => decideWidget(...a) } }),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
let onEvent: ((data: unknown) => void) | null = null;
vi.mock("@/app/store/mps", () => ({ onMuxEvent: (_: string, f: (d: unknown) => void) => (onEvent = f) }));
const opened: { comp: Component<any>; props: any }[] = [];
vi.mock("@/app/store/modalmodel", () => ({
    modalsModel: { openModal: (comp: Component<any>, props: any) => opened.push({ comp, props }) },
}));
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
import { startWidgetInstallRequests } from "./widget-install-requests";

const REQUEST = {
    id: "acme.clock",
    hash: "h1",
    name: "Clock",
    version: "1.0.0",
    description: "Shows the time",
    author: "Acme",
    kind: "sandboxed",
    permissions: [],
    signature: { state: "unsigned", fingerprint: null },
    agent: "QTest",
    block_id: "b1",
    requested_ms: 1,
};

const cardOf = (blockId = "b1") => paneRequests.of(blockId).find((c) => c.source === "widget-install");

beforeEach(async () => {
    list.mockResolvedValue({ requests: [] });
    startWidgetInstallRequests();
    onEvent!({ requests: [] });
    await Promise.resolve();
});

afterEach(() => {
    cleanup();
    decideWidget.mockReset();
});

describe("agents' widget install requests", () => {
    it("a request that names a pane is one card in that pane, not a window prompt, and the same one twice is still one", async () => {
        const before = opened.length;
        onEvent!({ requests: [REQUEST] });
        onEvent!({ requests: [REQUEST] });
        await Promise.resolve();
        expect(opened).toHaveLength(before);
        expect(paneRequests.of("b1").filter((c) => c.source === "widget-install")).toHaveLength(1);
        expect(cardOf()?.presentation).toBe("card");
        expect(cardOf()?.summary).toBe("QTest: install Clock");
    });

    it("says who wants to install what, and the answers go to srv through the host", async () => {
        onEvent!({ requests: [REQUEST] });
        await Promise.resolve();
        const card = cardOf()!;
        const close = vi.fn();
        const Card = card.Component;
        render(() => <Card {...card.props} close={close} />);
        expect(screen.getByLabelText("QTest wants to install Clock")).toBeTruthy();
        decideWidget.mockResolvedValue(undefined);
        fireEvent.click(screen.getByRole("button", { name: "Don't install" }));
        await vi.waitFor(() => expect(decideWidget).toHaveBeenCalledWith("acme.clock", "h1", false, ""));
        await vi.waitFor(() => expect(close).toHaveBeenCalledTimes(1));
    });

    it("shows a failed answer and stays", async () => {
        onEvent!({ requests: [REQUEST] });
        await Promise.resolve();
        const card = cardOf()!;
        const close = vi.fn();
        const Card = card.Component;
        render(() => <Card {...card.props} close={close} />);
        decideWidget.mockRejectedValue(new Error("host said no"));
        fireEvent.click(screen.getByRole("button", { name: "Don't install" }));
        expect(await screen.findByText("host said no")).toBeTruthy();
        expect(close).not.toHaveBeenCalled();
    });

    it("goes when srv's list no longer has it (answered in Settings or another window)", async () => {
        onEvent!({ requests: [REQUEST] });
        await Promise.resolve();
        expect(cardOf()).toBeDefined();
        onEvent!({ requests: [] });
        await Promise.resolve();
        expect(cardOf()).toBeUndefined();
    });

    it("closing the pane with the question open says no", async () => {
        onEvent!({ requests: [{ ...REQUEST, block_id: "b9" }] });
        await Promise.resolve();
        expect(cardOf("b9")).toBeDefined();
        decideWidget.mockResolvedValue(undefined);
        pane("b9")[1](undefined);
        await vi.waitFor(() => expect(decideWidget).toHaveBeenCalledWith("acme.clock", "h1", false, ""));
    });

    it("a request that names no pane (a bundle import, an unverified asker) is still the window prompt", async () => {
        const before = opened.length;
        onEvent!({ requests: [{ ...REQUEST, id: "acme.nopane", block_id: "" }] });
        await Promise.resolve();
        expect(opened).toHaveLength(before + 1);
        expect(paneRequests.of("").filter((c) => c.source === "widget-install")).toHaveLength(0);
    });
});
