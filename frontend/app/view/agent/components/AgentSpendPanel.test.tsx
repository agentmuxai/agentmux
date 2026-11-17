// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Spend cap view of My Agents' row panel: reads the agent's cap by its id, and lets a person set a number of
 * dollars, no cap, or the default, each as one `agent.spend` call.
 */
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const spend = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { AgentSpendCommand: (...args: unknown[]) => spend(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
let changed: (() => void) | undefined;
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_event: string, cb: () => void) => {
        changed = cb;
        return () => {};
    },
}));

import { AgentSpendPanel, capLine } from "./AgentSpendPanel";

const answer = (over: Record<string, unknown> = {}) => ({
    cap_usd: 50,
    spent_usd: 4.5,
    applies: true,
    own_cap: false,
    default_cap_usd: 50,
    ...over,
});
const box = () => screen.getByTestId("agent-spend-input") as HTMLInputElement;
const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;
const mount = () => render(() => <AgentSpendPanel agentId="uid-x" agentName="AgentY" />);
const calls = (action: string) => spend.mock.calls.filter((c) => (c[1] as { action: string }).action === action);

describe("AgentSpendPanel", () => {
    beforeEach(() => {
        spend.mockReset();
        changed = undefined;
    });
    afterEach(() => cleanup());

    it("reads the agent's cap by its id and says whose it is", async () => {
        spend.mockResolvedValue(answer());
        mount();
        await waitFor(() => expect(box().value).toBe("50"));
        expect(spend).toHaveBeenCalledWith({}, { blockid: "", agent: "uid-x", action: "get", cap_usd: null });
        expect(screen.getByTestId("agent-spend-line").textContent).toBe("Used today: $4.5. Its cap is $50 a day, the default every agent has until one is set for it.");
        expect(button("Use the default ($50)").disabled).toBe(true);
    });

    it("says in one line each of the four ways a cap can stand", () => {
        const line = (r: Parameters<typeof capLine>[0]) => capLine(r);
        expect(line(answer({ own_cap: true, cap_usd: 100 }))).toBe("Used today: $4.5. Its cap is $100 a day, set for it.");
        expect(line(answer({ own_cap: true, cap_usd: null }))).toBe("Used today: $4.5. It has no cap.");
        expect(line(answer({ cap_usd: null, default_cap_usd: null }))).toBe("Used today: $4.5. It has no cap: the default is no cap.");
    });

    it("sets the dollars typed, with or without a $, and shows it as the agent's own", async () => {
        spend.mockResolvedValueOnce(answer());
        mount();
        await waitFor(() => expect(box().value).toBe("50"));
        spend.mockResolvedValueOnce(answer({ cap_usd: 100, own_cap: true }));
        fireEvent.input(box(), { target: { value: "$100" } });
        fireEvent.click(button("Set cap"));
        await waitFor(() => expect(calls("set")).toHaveLength(1));
        expect(calls("set")[0][1]).toEqual({ blockid: "", agent: "uid-x", action: "set", cap_usd: 100 });
        await screen.findByTestId("agent-spend-saved");
        expect(screen.getByTestId("agent-spend-line").textContent).toContain("Its cap is $100 a day, set for it.");
        expect(button("Use the default ($50)").disabled).toBe(false);
    });

    it("refuses text that is not a cap before asking srv, and says why", async () => {
        spend.mockResolvedValue(answer());
        mount();
        await waitFor(() => expect(box().value).toBe("50"));
        for (const [text, said] of [
            ["", "Type a number of dollars."],
            ["lots", "That is not a number of dollars."],
            ["-5", "That is not a number of dollars."],
            ["0.001", "A cap is at least $0.01. Use No cap to remove it."],
        ] as const) {
            fireEvent.input(box(), { target: { value: text } });
            fireEvent.click(button("Set cap"));
            expect((await screen.findByRole("alert")).textContent).toBe(said);
        }
        expect(calls("set")).toHaveLength(0);
    });

    it("takes the cap away, and gives the agent the default again", async () => {
        spend.mockResolvedValueOnce(answer({ cap_usd: 100, own_cap: true }));
        mount();
        await waitFor(() => expect(box().value).toBe("100"));
        spend.mockResolvedValueOnce(answer({ cap_usd: null, own_cap: true }));
        fireEvent.click(button("No cap"));
        await waitFor(() => expect(calls("set")).toHaveLength(1));
        expect(calls("set")[0][1]).toEqual({ blockid: "", agent: "uid-x", action: "set", cap_usd: null });
        await waitFor(() => expect(box().value).toBe(""));
        expect(button("No cap").disabled).toBe(true);
        spend.mockResolvedValueOnce(answer());
        fireEvent.click(button("Use the default ($50)"));
        await waitFor(() => expect(calls("default")).toHaveLength(1));
        await waitFor(() => expect(box().value).toBe("50"));
    });

    it("offers nothing for an agent on a login, which has plan limits", async () => {
        spend.mockResolvedValue(answer({ applies: false, cap_usd: null }));
        mount();
        expect((await screen.findByTestId("agent-spend-none")).textContent).toContain("it is on a login");
        expect(screen.queryByTestId("agent-spend-input")).toBeNull();
        expect(screen.queryByRole("button", { name: "Set cap" })).toBeNull();
    });

    it("shows why srv refused, and keeps what was typed", async () => {
        spend.mockResolvedValueOnce(answer());
        mount();
        await waitFor(() => expect(box().value).toBe("50"));
        spend.mockRejectedValueOnce(new Error("only a person changes an agent's spend cap in the window"));
        fireEvent.input(box(), { target: { value: "75" } });
        fireEvent.click(button("Set cap"));
        expect((await screen.findByRole("alert")).textContent).toContain("only a person changes");
        expect(box().value).toBe("75");
    });

    it("follows a change made elsewhere, unless there is an edit in the box", async () => {
        spend.mockResolvedValue(answer());
        mount();
        await waitFor(() => expect(box().value).toBe("50"));
        spend.mockResolvedValue(answer({ cap_usd: 80, own_cap: true }));
        changed?.();
        await waitFor(() => expect(box().value).toBe("80"));
        fireEvent.input(box(), { target: { value: "120" } });
        spend.mockResolvedValue(answer({ cap_usd: 90, own_cap: true }));
        changed?.();
        await waitFor(() => expect(screen.getByTestId("agent-spend-line").textContent).toContain("$90"));
        expect(box().value).toBe("120");
    });
});
