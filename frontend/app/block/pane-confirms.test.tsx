// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A question srv asks about the person's own click in a pane (add a host, remove the helper, end a
 * session, install a tool) is a modal over that pane only, answered through `pane.confirm.decide`
 * (docs/specs/SPEC_AGENT_SPAWN_NOTICE_IN_THE_PANE_2026_11_08.md §6 rows 3 and 6).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const decide = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { PaneConfirmDecideCommand: (...a: unknown[]) => decide(...a) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
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

import { ModalLayer } from "@/app/element/ModalLayer";
import { __resetPaneRequests, paneRequests } from "@/app/store/pane-requests";
import { PaneRequestModals } from "@/app/view/agent/pane-requests/PaneRequestDock";
import type { PaneConfirmView } from "@/types/rpc/PaneConfirmView";
import { __resetPaneConfirms, startPaneConfirms } from "./pane-confirms";

const Q: PaneConfirmView = {
    id: "q1",
    block_id: "remotes",
    title: "End a session on db1",
    message: "End the durable session s1 on db1?\n\nIts shell, and everything running in it, stops.",
    ok_label: "End Session",
    cancel_label: "Keep It",
    destructive: true,
    checkbox: "",
    deadline_ms: 1_700_000_120_000,
    status: "pending",
};

/** Two panes and a tab bar button, each pane with the block frame's modal host. */
function Window() {
    return (
        <div>
            <button data-testid="tab-bar">tab bar</button>
            {["remotes", "other"].map((id) => (
                <div data-testid={`pane-${id}`}>
                    <ModalLayer scope="pane" fill>
                        <PaneRequestModals blockId={id} />
                        <button data-testid={`btn-${id}`}>content {id}</button>
                    </ModalLayer>
                </div>
            ))}
        </div>
    );
}

const requests = (blockId = "remotes") => paneRequests.of(blockId).filter((r) => r.source === "pane-confirm");

beforeEach(() => {
    __resetPaneConfirms();
    __resetPaneRequests();
    panes.clear();
    decide.mockReset();
    decide.mockResolvedValue({ outcome: "answered" });
    startPaneConfirms();
});

afterEach(() => cleanup());

describe("a question srv asks about a pane", () => {
    it("is one modal in that pane's layer, with srv's words, and the same event twice is still one", () => {
        const { getByTestId } = render(() => <Window />);
        onEvent!(Q);
        onEvent!(Q);
        expect(requests()).toHaveLength(1);
        expect(requests()[0].presentation).toBe("modal");
        expect(requests()[0].deadlineMs).toBe(Q.deadline_ms);
        const title = screen.getByText("End a session on db1");
        expect(getByTestId("pane-remotes").contains(title)).toBe(true);
        expect(getByTestId("pane-other").contains(title)).toBe(false);
        expect(screen.getByText(/Its shell, and everything running in it, stops\./)).toBeTruthy();
        expect(screen.getByRole("button", { name: "End Session" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "Keep It" })).toBeTruthy();
        expect(screen.queryByRole("switch")).toBeNull();
    });

    it.each([
        ["End Session", true],
        ["Keep It", false],
    ])("%s goes to srv as approve=%s, and the modal closes", async (label, approve) => {
        render(() => <Window />);
        onEvent!(Q);
        fireEvent.click(screen.getByRole("button", { name: label }));
        await vi.waitFor(() => expect(decide).toHaveBeenCalledWith({}, { request_id: "q1", approve, checkbox: false }));
        await vi.waitFor(() => expect(requests()).toHaveLength(0));
    });

    it("sends the tick box with a yes, and never with a no", async () => {
        render(() => <Window />);
        onEvent!({
            ...Q,
            id: "q2",
            destructive: false,
            checkbox: "Don't ask again for db1",
            ok_label: "Install",
            cancel_label: "Not now",
        });
        fireEvent.click(screen.getByRole("switch"));
        fireEvent.click(screen.getByRole("button", { name: "Install" }));
        await vi.waitFor(() =>
            expect(decide).toHaveBeenCalledWith({}, { request_id: "q2", approve: true, checkbox: true })
        );

        onEvent!({ ...Q, id: "q3", checkbox: "Don't ask again for db1" });
        await vi.waitFor(() => expect(screen.getByRole("switch")).toBeTruthy());
        fireEvent.click(screen.getByRole("switch"));
        fireEvent.click(screen.getByRole("button", { name: "Keep It" }));
        await vi.waitFor(() =>
            expect(decide).toHaveBeenLastCalledWith({}, { request_id: "q3", approve: false, checkbox: false })
        );
    });

    it("shows a failed answer and stays, so the person can answer again", async () => {
        render(() => <Window />);
        onEvent!(Q);
        decide.mockRejectedValueOnce(new Error("FORBIDDEN: not the window"));
        fireEvent.click(screen.getByRole("button", { name: "End Session" }));
        expect(await screen.findByText("FORBIDDEN: not the window")).toBeTruthy();
        expect(requests()).toHaveLength(1);
    });

    it("goes when srv says it ended (its time ran out, or another window answered)", () => {
        render(() => <Window />);
        onEvent!(Q);
        expect(requests()).toHaveLength(1);
        onEvent!({ ...Q, status: "cleared" });
        expect(requests()).toHaveLength(0);
        expect(decide).not.toHaveBeenCalled();
    });

    it("is a no when the pane is closed with it open", async () => {
        render(() => <Window />);
        pane("remotes");
        onEvent!(Q);
        pane("remotes")[1](undefined);
        await vi.waitFor(() =>
            expect(decide).toHaveBeenCalledWith({}, { request_id: "q1", approve: false, checkbox: false })
        );
        expect(requests()).toHaveLength(0);
    });

    it("asks nothing for a question that names no pane", () => {
        onEvent!({ ...Q, id: "q9", block_id: "" });
        expect(paneRequests.of("")).toHaveLength(0);
    });
});
