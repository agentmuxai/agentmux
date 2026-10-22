// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** An agent's request to drive a native app opens a prompt; the user's answer goes to srv. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import type { Component } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const answer = vi.fn();
const list = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { AppDesktopCommand: (...a: unknown[]) => list(...a) },
}));
vi.mock("@/app/store/app-api", () => ({
    getApi: () => ({ approvals: { decideDesktop: (...a: unknown[]) => answer(...a) } }),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
let onEvent: ((data: unknown) => void) | null = null;
vi.mock("@/app/store/mps", () => ({ onMuxEvent: (_: string, f: (d: unknown) => void) => (onEvent = f) }));
const opened: { comp: Component<any>; props: any }[] = [];
vi.mock("@/app/store/modalmodel", () => ({
    modalsModel: { openModal: (comp: Component<any>, props: any) => opened.push({ comp, props }) },
}));

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

afterEach(() => cleanup());

describe("agents' desktop requests", () => {
    it("open one prompt per request, and the answer goes to srv through the host", async () => {
        list.mockResolvedValue({ requests: [], grants: [] });
        startDesktopRequests();
        onEvent!({ requests: [REQUEST], grants: [] });
        onEvent!({ requests: [REQUEST], grants: [] });
        await Promise.resolve();
        expect(opened).toHaveLength(1);
        expect(desktopRequests()).toEqual([REQUEST]);

        const close = vi.fn();
        const { comp: Prompt, props } = opened[0];
        render(() => <Prompt {...props} close={close} />);
        expect(screen.getByText("QTest wants to control notepad")).toBeTruthy();
        expect(screen.getByText("Window: Untitled - Notepad")).toBeTruthy();

        answer.mockResolvedValue(undefined);
        fireEvent.click(screen.getByRole("button", { name: "Allow for 30 min" }));
        await Promise.resolve();
        expect(answer).toHaveBeenCalledWith("r1", "timed");
    });

    it("has three answers: a while, the session, and no", async () => {
        onEvent!({ requests: [{ ...REQUEST, id: "r2" }], grants: [] });
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
        onEvent!({ requests: [{ ...REQUEST, id: "r3" }], grants: [] });
        await Promise.resolve();
        const { comp: Prompt, props } = opened[opened.length - 1];
        const close = vi.fn();
        render(() => <Prompt {...props} close={close} />);
        onEvent!({ requests: [], grants: [GRANT] });
        await Promise.resolve();
        expect(close).toHaveBeenCalled();
        expect(desktopGrants()).toEqual([GRANT]);
    });
});
