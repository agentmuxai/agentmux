// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's request to drive a native app is a card in its own pane (a window prompt when it names no pane);
 * the user's answer goes to srv through the host
 * (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 4).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal, type Component } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const answer = vi.fn();
const list = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { AppDesktopCommand: (...a: unknown[]) => list(...a) },
}));
vi.mock("@/util/app-api", () => ({
    getApi: () => ({ approvals: { decideDesktop: (...a: unknown[]) => answer(...a) } }),
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
import { desktopGrants, desktopRequests, startDesktopRequests } from "./desktop-requests";

const REQUEST = {
    id: "r1",
    agent: "QTest",
    app: "notepad",
    title: "Untitled - Notepad",
    asked_ms: 1,
    block_id: "b1",
    grant_minutes: 30,
};
const GRANT = { agent: "QTest", app: "notepad", block_id: "b1", until_ms: 99, active: false };

const cardOf = (blockId = "b1") => paneRequests.of(blockId).find((c) => c.source === "desktop");

beforeEach(async () => {
    list.mockResolvedValue({ requests: [], grants: [] });
    startDesktopRequests();
    onEvent!({ requests: [], grants: [] });
    await Promise.resolve();
});

afterEach(() => {
    cleanup();
    answer.mockReset();
});

describe("agents' desktop requests", () => {
    it("a request that names a pane is one card in that pane, not a window prompt, and the same one twice is still one", async () => {
        const before = opened.length;
        onEvent!({ requests: [REQUEST], grants: [] });
        onEvent!({ requests: [REQUEST], grants: [] });
        await Promise.resolve();
        expect(opened).toHaveLength(before);
        expect(paneRequests.of("b1").filter((c) => c.source === "desktop")).toHaveLength(1);
        expect(cardOf()?.presentation).toBe("card");
        expect(cardOf()?.summary).toBe("QTest: control notepad");
        expect(desktopRequests()).toEqual([REQUEST]);
    });

    it("says who wants to control which app, and the answer goes to srv through the host", async () => {
        onEvent!({ requests: [REQUEST], grants: [] });
        await Promise.resolve();
        const card = cardOf()!;
        const close = vi.fn();
        const Card = card.Component;
        render(() => <Card {...card.props} close={close} />);
        expect(screen.getByLabelText("QTest wants to control notepad")).toBeTruthy();
        expect(screen.getByText("Window: Untitled - Notepad")).toBeTruthy();
        answer.mockResolvedValue(undefined);
        fireEvent.click(screen.getByRole("button", { name: "Allow for 30 min" }));
        await vi.waitFor(() => expect(answer).toHaveBeenCalledWith("r1", "timed"));
        await vi.waitFor(() => expect(close).toHaveBeenCalledTimes(1));
    });

    it("has three answers: a while, the session, and no", async () => {
        onEvent!({ requests: [{ ...REQUEST, id: "r2" }], grants: [] });
        await Promise.resolve();
        const card = cardOf()!;
        const Card = card.Component;
        render(() => <Card {...card.props} close={vi.fn()} />);
        answer.mockResolvedValue(undefined);
        fireEvent.click(screen.getByRole("button", { name: "Allow for this session" }));
        await vi.waitFor(() => expect(answer).toHaveBeenLastCalledWith("r2", "session"));
        fireEvent.click(screen.getByRole("button", { name: "Don't allow" }));
        await vi.waitFor(() => expect(answer).toHaveBeenLastCalledWith("r2", "deny"));
    });

    it("shows a failed answer and stays", async () => {
        onEvent!({ requests: [{ ...REQUEST, id: "r4" }], grants: [] });
        await Promise.resolve();
        const card = cardOf()!;
        const close = vi.fn();
        const Card = card.Component;
        render(() => <Card {...card.props} close={close} />);
        answer.mockRejectedValue(new Error("host said no"));
        fireEvent.click(screen.getByRole("button", { name: "Don't allow" }));
        expect(await screen.findByText("host said no")).toBeTruthy();
        expect(close).not.toHaveBeenCalled();
    });

    it("goes when srv's list no longer has it (answered in another window), and the grants stay in force", async () => {
        onEvent!({ requests: [{ ...REQUEST, id: "r3" }], grants: [] });
        await Promise.resolve();
        expect(cardOf()).toBeDefined();
        onEvent!({ requests: [], grants: [GRANT] });
        await Promise.resolve();
        expect(cardOf()).toBeUndefined();
        expect(desktopGrants()).toEqual([GRANT]);
    });

    it("closing the pane with the question open says no", async () => {
        onEvent!({ requests: [{ ...REQUEST, id: "r5", block_id: "b9" }], grants: [] });
        await Promise.resolve();
        expect(cardOf("b9")).toBeDefined();
        answer.mockResolvedValue(undefined);
        pane("b9")[1](undefined);
        await vi.waitFor(() => expect(answer).toHaveBeenCalledWith("r5", "deny"));
    });

    it("a request that names no pane is still one window prompt, answered the same way", async () => {
        const before = opened.length;
        const noPane = { ...REQUEST, id: "r6", block_id: "" };
        onEvent!({ requests: [noPane], grants: [] });
        onEvent!({ requests: [noPane], grants: [] });
        await Promise.resolve();
        expect(opened).toHaveLength(before + 1);
        expect(paneRequests.of("").filter((c) => c.source === "desktop")).toHaveLength(0);

        const { comp: Prompt, props } = opened[opened.length - 1];
        const close = vi.fn();
        render(() => <Prompt {...props} close={close} />);
        expect(screen.getByText("QTest wants to control notepad")).toBeTruthy();
        answer.mockResolvedValue(undefined);
        fireEvent.click(screen.getByRole("button", { name: "Allow for this session" }));
        await vi.waitFor(() => expect(answer).toHaveBeenCalledWith("r6", "session"));
    });

    it("closes a window prompt that was answered elsewhere", async () => {
        onEvent!({ requests: [{ ...REQUEST, id: "r7", block_id: "" }], grants: [] });
        await Promise.resolve();
        const { comp: Prompt, props } = opened[opened.length - 1];
        const close = vi.fn();
        render(() => <Prompt {...props} close={close} />);
        onEvent!({ requests: [], grants: [] });
        await Promise.resolve();
        expect(close).toHaveBeenCalled();
    });
});
