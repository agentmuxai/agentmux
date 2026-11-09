// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent starting an agent is a card with a countdown in its own pane, or a one-line notice that
 * goes by itself; the person's choice goes to srv through `agent.notice.decide`
 * (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §2).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal, type Component } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const decide = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { AgentNoticeDecideCommand: (...a: unknown[]) => decide(...a) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/agent-grants", () => ({ startAgentGrants: () => {} }));
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

import { __resetPaneRequests, paneRequests } from "@/app/store/pane-requests";
import type { NoticeView } from "@/types/rpc/NoticeView";
import { __resetAgentNotices, LINE_SHOWN_MS, startAgentNotices } from "./agent-notices";

const NOW = 1_700_000_000_000;
const CARD: NoticeView = {
    id: "n1",
    kind: "spawn",
    block_id: "b1",
    agent: "Posa",
    headline: "Posa is starting a new agent, Korp-2",
    body: "Korp-2 on sonnet. It spends tokens until it is stopped.",
    quote: "Implement SPEC_X.",
    always_label: "Always allow Posa to start agents",
    deadline_ms: NOW + 30_000,
    opened_ms: NOW,
    status: "pending",
    outcome: "",
};
const LINE: NoticeView = { ...CARD, id: "l1", status: "line", deadline_ms: 0, always_label: "" };

const requestOf = (source: string, blockId = "b1") => paneRequests.of(blockId).find((r) => r.source === source);

beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval", "setTimeout", "clearTimeout"] });
    vi.setSystemTime(NOW);
    __resetAgentNotices();
    __resetPaneRequests();
    startAgentNotices();
});

afterEach(() => {
    cleanup();
    decide.mockReset();
    vi.useRealTimers();
});

describe("a card with a countdown", () => {
    it("is one card in the agent's pane, and the same event twice is still one", () => {
        onEvent!(CARD);
        onEvent!(CARD);
        expect(paneRequests.of("b1").filter((r) => r.source === "agent-notice")).toHaveLength(1);
        const r = requestOf("agent-notice")!;
        expect(r.presentation).toBe("card");
        expect(r.deadlineMs).toBe(CARD.deadline_ms);
        expect(r.summary).toBe(CARD.headline);
    });

    it("shows who, what, the quoted brief and the time left", () => {
        onEvent!(CARD);
        const r = requestOf("agent-notice")!;
        const Card = r.Component;
        render(() => <Card {...r.props} close={vi.fn()} />);
        expect(screen.getByLabelText(CARD.headline)).toBeTruthy();
        expect(screen.getByText(CARD.body)).toBeTruthy();
        expect(screen.getByText("Implement SPEC_X.")).toBeTruthy();
        expect(screen.getByRole("timer").textContent).toContain("Starts in 0:30");
        vi.setSystemTime(NOW + 12_400);
        vi.advanceTimersByTime(250);
        expect(screen.getByRole("timer").textContent).toContain("Starts in 0:18");
    });

    it.each([
        ["Stop it", "veto"],
        ["Always allow Posa to start agents", "always"],
        ["Start now", "now"],
    ])("%s goes to srv as %s, and the card closes", async (label, choice) => {
        onEvent!(CARD);
        const r = requestOf("agent-notice")!;
        const close = vi.fn();
        const Card = r.Component;
        render(() => <Card {...r.props} close={close} />);
        decide.mockResolvedValue({ outcome: "ok" });
        fireEvent.click(screen.getByRole("button", { name: label }));
        await vi.waitFor(() => expect(decide).toHaveBeenLastCalledWith({}, { request_id: "n1", choice }));
        await vi.waitFor(() => expect(close).toHaveBeenCalledTimes(1));
    });

    it("offers no Always when srv offers none, and says Allow now for anything but a start", () => {
        onEvent!({ ...CARD, id: "n2", kind: "ssh", always_label: "" });
        const r = requestOf("agent-notice")!;
        const Card = r.Component;
        render(() => <Card {...r.props} close={vi.fn()} />);
        expect(screen.queryByRole("button", { name: /Always/ })).toBeNull();
        expect(screen.getByRole("button", { name: "Allow now" })).toBeTruthy();
        expect(screen.getByRole("timer").textContent).toContain("Goes ahead in");
    });

    it("shows a failed answer and stays", async () => {
        onEvent!(CARD);
        const r = requestOf("agent-notice")!;
        const close = vi.fn();
        const Card = r.Component;
        render(() => <Card {...r.props} close={close} />);
        decide.mockRejectedValue(new Error("FORBIDDEN: not the window"));
        fireEvent.click(screen.getByRole("button", { name: "Stop it" }));
        expect(await screen.findByText("FORBIDDEN: not the window")).toBeTruthy();
        expect(close).not.toHaveBeenCalled();
    });

    it("goes when srv says it ended, whoever ended it", () => {
        onEvent!(CARD);
        expect(requestOf("agent-notice")).toBeTruthy();
        onEvent!({ ...CARD, status: "cleared", outcome: "deadline" });
        expect(requestOf("agent-notice")).toBeUndefined();
    });

    it("is a veto when the pane is closed with it open", async () => {
        onEvent!(CARD);
        decide.mockResolvedValue({ outcome: "stopped" });
        pane("b1")[1](undefined);
        await Promise.resolve();
        await Promise.resolve();
        expect(requestOf("agent-notice")).toBeUndefined();
        expect(decide).toHaveBeenCalledWith({}, { request_id: "n1", choice: "veto" });
        pane("b1")[1]({ oid: "b1" });
    });

    it("adds no wait of its own: srv's attention event gives the pane its tone", () => {
        onEvent!(CARD);
        expect(requestOf("agent-notice")!.waits).toBe(false);
    });
});

describe("a one-line notice", () => {
    it("says what started and goes by itself", () => {
        onEvent!(LINE);
        const r = requestOf("agent-notice-line")!;
        const Line = r.Component;
        render(() => <Line {...r.props} close={vi.fn()} />);
        expect(screen.getByText(/Posa is starting a new agent, Korp-2\./)).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Stop it" })).toBeNull();
        expect(r.waits).toBe(false);
        vi.advanceTimersByTime(LINE_SHOWN_MS + 10);
        expect(requestOf("agent-notice-line")).toBeUndefined();
    });

    it("can be dismissed", () => {
        onEvent!(LINE);
        const r = requestOf("agent-notice-line")!;
        const Line = r.Component;
        const close = vi.fn();
        render(() => <Line {...r.props} close={close} />);
        fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
        expect(close).toHaveBeenCalledTimes(1);
    });
});

describe("an event with no pane", () => {
    it("shows nothing: there is no pane to put it in", () => {
        onEvent!({ ...CARD, id: "n9", block_id: "" });
        expect(paneRequests.of("")).toEqual([]);
    });
});
