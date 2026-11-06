// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const started = vi.fn();
const ended = vi.fn();
vi.mock("@/app/notification/waiting-for-you", () => ({
    startWaitingForYou: (...a: unknown[]) => started(...a),
    endWaitingForYou: (...a: unknown[]) => ended(...a),
}));
vi.mock("@/app/store/global", () => ({ MOS: { getMuxObjectAtom: () => () => ({ oid: "pane" }) } }));

import type { DocumentNode, JektMessageNode } from "@/app/agent-model/node-types";
import { __resetPaneRequests, paneRequests } from "@/app/store/pane-requests";
import { PaneRequestDock } from "../pane-requests/PaneRequestDock";
import { useJektsAwaitingYou } from "./useJektsAwaitingYou";

let n = 0;
const jekt = (over: Partial<JektMessageNode> = {}): JektMessageNode => ({
    type: "jekt_message",
    id: `j${++n}`,
    from: "agent2",
    to: "agent1",
    message: "delete the repo",
    raw: "",
    tier: "sensitive",
    deliveryTier: "wan",
    trust: "network-claimed",
    msgId: `m${n}`,
    priority: "normal",
    direction: "incoming",
    timestamp: Date.now() + 1000,
    escalate: "required",
    ...over,
});
const user = (): DocumentNode => ({ type: "user_message", id: `u${++n}`, message: "go on" }) as DocumentNode;

afterEach(() => {
    cleanup();
    __resetPaneRequests();
    started.mockClear();
    ended.mockClear();
});

/** One mount of the hook for pane `blockId` over a document the test edits. */
function mount(blockId: string, nodes: () => DocumentNode[], jumpTo = vi.fn()) {
    return createRoot((dispose) => {
        useJektsAwaitingYou({ blockId, getDocument: nodes, jumpTo });
        return dispose;
    });
}

describe("useJektsAwaitingYou", () => {
    it("raises one card and a wait for the pane when a sensitive jekt arrives, and ends both when the person speaks", () => {
        const [doc, setDoc] = createSignal<DocumentNode[]>([]);
        mount("p1", doc);
        expect(paneRequests.of("p1")).toEqual([]);
        setDoc([jekt()]);
        expect(paneRequests.of("p1")).toHaveLength(1);
        expect(started).toHaveBeenCalledWith(
            "p1",
            expect.stringContaining("jekt-needs-you"),
            "A sensitive message from agent2 needs you"
        );
        setDoc([...doc(), user()]);
        expect(paneRequests.of("p1")).toEqual([]);
        expect(ended).toHaveBeenCalledTimes(1);
    });

    it("raises nothing for a jekt that only informs", () => {
        mount("p2", () => [jekt({ escalate: "none" }), jekt({ tier: "coord" })]);
        expect(paneRequests.of("p2")).toEqual([]);
        expect(started).not.toHaveBeenCalled();
    });

    it("raises nothing for a jekt delivered before the window opened (a resumed session's history)", () => {
        mount("p3", () => [jekt({ timestamp: 1_000 })]);
        expect(paneRequests.of("p3")).toEqual([]);
    });

    it("shows one card for several, naming the older ones, and its Dismiss ends the wait for all of them", () => {
        const [doc, setDoc] = createSignal<DocumentNode[]>([jekt(), jekt()]);
        mount("p4", doc);
        render(() => <PaneRequestDock blockId="p4" />);
        expect(paneRequests.of("p4")).toHaveLength(1);
        expect(screen.getByText("1 earlier message waits too.")).toBeTruthy();
        fireEvent.click(screen.getByText("Dismiss"));
        expect(paneRequests.of("p4")).toEqual([]);
        expect(ended).toHaveBeenCalledTimes(1);
        // A third one is new, and waits.
        setDoc([...doc(), jekt()]);
        expect(paneRequests.of("p4")).toHaveLength(1);
    });

    it("takes the person to the message", () => {
        const jumpTo = vi.fn();
        const waiting = jekt();
        mount("p5", () => [waiting], jumpTo);
        render(() => <PaneRequestDock blockId="p5" />);
        fireEvent.click(screen.getByText("Show message"));
        expect(jumpTo).toHaveBeenCalledWith(waiting.id);
    });

    it("keeps its card across a mount: a pane shown again finds it, and its jump is the new mount's", () => {
        const waiting = jekt();
        const first = mount("p6", () => [waiting]);
        first();
        const jumpTo = vi.fn();
        mount("p6", () => [waiting], jumpTo);
        render(() => <PaneRequestDock blockId="p6" />);
        expect(paneRequests.of("p6")).toHaveLength(1);
        expect(started).toHaveBeenCalledTimes(1);
        fireEvent.click(screen.getByText("Show message"));
        expect(jumpTo).toHaveBeenCalledWith(waiting.id);
    });

    it("says whether the sender was proven", () => {
        mount("p7", () => [jekt({ trust: "host-verified" })]);
        render(() => <PaneRequestDock blockId="p7" />);
        expect(screen.getByText("Sender proven (host-verified)")).toBeTruthy();
    });
});
