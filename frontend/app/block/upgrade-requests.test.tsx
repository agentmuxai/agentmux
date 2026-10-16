// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** An agent's request to upgrade AgentMux opens a prompt; the user's answer goes to srv. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import type { Component } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const answer = vi.fn();
const list = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AppUpgradeRequestAnswerCommand: (...a: unknown[]) => answer(...a),
        AppUpgradeRequestsCommand: (...a: unknown[]) => list(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
let onEvent: ((data: unknown) => void) | null = null;
vi.mock("@/app/store/mps", () => ({ onMuxEvent: (_: string, f: (d: unknown) => void) => (onEvent = f) }));
const opened: { comp: Component<any>; props: any }[] = [];
vi.mock("@/app/store/modalmodel", () => ({
    modalsModel: { openModal: (comp: Component<any>, props: any) => opened.push({ comp, props }) },
}));

import { startUpgradeRequests, upgradeRequests } from "./upgrade-requests";

const REQUEST = { id: "r1", agent: "QTest", reason: "0.59.27 fixes the send bug", version: "0.59.27", asked_ms: 1 };

afterEach(() => cleanup());

describe("agents' upgrade requests", () => {
    it("open one prompt per request, and the answer goes to srv", async () => {
        list.mockResolvedValue({ requests: [] });
        startUpgradeRequests();
        onEvent!({ requests: [REQUEST] });
        onEvent!({ requests: [REQUEST] });
        await Promise.resolve();
        expect(opened).toHaveLength(1);
        expect(upgradeRequests()).toEqual([REQUEST]);

        const close = vi.fn();
        const { comp: Prompt, props } = opened[0];
        render(() => <Prompt {...props} close={close} />);
        expect(screen.getByText("QTest asks to upgrade AgentMux to 0.59.27")).toBeTruthy();
        expect(screen.getByText("0.59.27 fixes the send bug")).toBeTruthy();

        answer.mockResolvedValue({ requests: [] });
        fireEvent.click(screen.getByRole("button", { name: "Later" }));
        await Promise.resolve();
        expect(answer).toHaveBeenCalledWith(expect.anything(), { id: "r1", accept: false });
    });

    it("Upgrade now accepts, and a request answered elsewhere closes its prompt", async () => {
        onEvent!({ requests: [{ ...REQUEST, id: "r2" }] });
        await Promise.resolve();
        const { comp: Prompt, props } = opened[opened.length - 1];
        const close = vi.fn();
        render(() => <Prompt {...props} close={close} />);
        answer.mockResolvedValue({ requests: [] });
        fireEvent.click(screen.getByRole("button", { name: "Upgrade now" }));
        await Promise.resolve();
        expect(answer).toHaveBeenLastCalledWith(expect.anything(), { id: "r2", accept: true });

        onEvent!({ requests: [] });
        await Promise.resolve();
        expect(close).toHaveBeenCalled();
    });
});
