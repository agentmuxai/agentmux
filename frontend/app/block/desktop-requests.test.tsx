// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's request to drive a native app is a card in its own pane (a window prompt when it names no
 * pane); the user's answer goes to srv through the host.
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
const NO_PANE = { ...REQUEST, id: "p1", block_id: "" };
const GRANT = { agent: "QTest", app: "notepad", block_id: "b1", until_ms: 99, active: false };

afterEach(() => {
    cleanup();
    answer.mockReset();
});

describe("agents' desktop requests", () => {
    it("a request that names no pane opens one window prompt, and the answer goes to srv through the host", async () => {
        list.mockResolvedValue({ requests: [], grants: [] });
        startDesktopRequests();
        onEvent!({ requests: [NO_PANE], grants: [] });
        onEvent!({ requests: [NO_PANE], grants: [] });
        await Promise.resolve();
        expect(opened).toHaveLength(1);
        expect(desktopRequests()).toEqual([NO_PANE]);

        const close = vi.fn();
        const { comp: Prompt, props } = opened[0];
        render(() => <Prompt {...props} close={close} />);
        expect(screen.getByText("QTest wants to control notepad")).toBeTruthy();
        expect(screen.getByText("Window: Untitled - Notepad")).toBeTruthy();

        answer.mockResolvedValue(undefined);
        fireEvent.click(screen.getByRole("button", { name: "Allow for 30 min" }));
        await Promise.resolve();
        expect(answer).toHaveBeenCalledWith("p1", "timed");
    });

    it("has three answers: a while, the session, and no", async () => {
        onEvent!({ requests: [{ ...NO_PANE, id: "r2" }], grants: [] });
        await Promise.resolve();
        const { comp: Prompt, props } = opened[opened.length - 1];
        render(() => <Prompt {...props} close={vi.fn()} />);
        answer.mockResolvedValue(undefined);
        fireEvent.click(screen.getByRole("button", { name: "Allow for this session" }));
        await Promise.resolve();
        expect(answer).toHaveBeenLastCalledWith("r2", "session");
        fireEvent.click(screen.getByRole("button", { name: "Don't allow" }));
        await Promise.resolve();
        expect(answer).toHaveBeenLastCalledWith("r2", "deny");
    });

    it("close a prompt that was answered elsewhere, and keep the grants in force", async () => {
        onEvent!({ requests: [{ ...NO_PANE, id: "r3" }], grants: [] });
        await Promise.resolve();
        const { comp: Prompt, props } = opened[opened.length - 1];
        const close = vi.fn();
        render(() => <Prompt {...props} close={close} />);
        onEvent!({ requests: [], grants: [GRANT] });
        await Promise.resolve();
        expect(close).toHaveBeenCalled();
        expect(desktopGrants()).toEqual([GRANT]);
    });

    describe("in its pane", () => {
        const cardOf = () => paneRequests.of("b1").find((c) => c.source === "desktop-control");
        beforeEach(async () => {
            onEvent!({ requests: [], grants: [] });
            onEvent!({ requests: [REQUEST], grants: [] });
            await Promise.resolve();
        });

        it("is one card in the asking agent's pane, not a window prompt, and the same request twice is still one", async () => {
            const before = opened.length;
            onEvent!({ requests: [REQUEST], grants: [] });
            await Promise.resolve();
            expect(opened).toHaveLength(before);
            expect(paneRequests.of("b1").filter((c) => c.source === "desktop-control")).toHaveLength(1);
            expect(cardOf()?.presentation).toBe("card");
            expect(cardOf()?.summary).toBe("QTest: control notepad");
        });

        it("says who wants what and what it can do, and answers to srv: a while, the session, no", async () => {
            const card = cardOf()!;
            const close = vi.fn();
            const Card = card.Component;
            render(() => <Card {...card.props} close={close} />);
            expect(screen.getByRole("alertdialog", { name: "QTest wants to control notepad" })).toBeTruthy();
            expect(screen.getByText("Window: Untitled - Notepad")).toBeTruthy();
            expect(screen.getByText(/be able to click and type in it/)).toBeTruthy();
            answer.mockResolvedValue(undefined);
            fireEvent.click(screen.getByRole("button", { name: "Allow for 30 min" }));
            await vi.waitFor(() => expect(answer).toHaveBeenLastCalledWith("r1", "timed"));
            await vi.waitFor(() => expect(close).toHaveBeenCalledTimes(1));
            fireEvent.click(screen.getByRole("button", { name: "Allow for this session" }));
            await vi.waitFor(() => expect(answer).toHaveBeenLastCalledWith("r1", "session"));
            fireEvent.click(screen.getByRole("button", { name: "Don't allow" }));
            await vi.waitFor(() => expect(answer).toHaveBeenLastCalledWith("r1", "deny"));
        });

        it("shows a failed answer and stays", async () => {
            const card = cardOf()!;
            const close = vi.fn();
            const Card = card.Component;
            render(() => <Card {...card.props} close={close} />);
            answer.mockRejectedValue(new Error("host said no"));
            fireEvent.click(screen.getByRole("button", { name: "Don't allow" }));
            expect(await screen.findByText("host said no")).toBeTruthy();
            expect(close).not.toHaveBeenCalled();
        });

        it("goes when srv's list no longer has it (answered elsewhere)", async () => {
            onEvent!({ requests: [], grants: [] });
            await Promise.resolve();
            expect(cardOf()).toBeUndefined();
        });

        it("closing the pane with the question open says no", async () => {
            const gone = { ...REQUEST, id: "r9", block_id: "b9" };
            onEvent!({ requests: [gone], grants: [] });
            await Promise.resolve();
            expect(paneRequests.of("b9")).toHaveLength(1);
            answer.mockResolvedValue(undefined);
            pane("b9")[1](undefined);
            await vi.waitFor(() => expect(answer).toHaveBeenCalledWith("r9", "deny"));
        });
    });
});
