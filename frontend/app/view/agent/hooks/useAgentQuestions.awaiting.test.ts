// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The `term:awaiting_user` block-meta flag the Swarm row reads to say "Waiting
 * for you" (store/swarm-line.ts). ReAgent P1 on #4234: the flag must survive the
 * pane unmounting while a question is still pending (a tab switch), and a stale
 * `true` must not live on once nothing is pending.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentNode, ToolNode } from "../types";

const hub = vi.hoisted(() => ({
    updates: [] as Array<Record<string, unknown>>,
    blockMeta: {} as Record<string, unknown>,
}));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/agent-document-store", () => ({ dispatch: vi.fn() }));
vi.mock("@/app/store/agent-pane-state-store", () => ({ fireEvent: vi.fn() }));
vi.mock("@/app/store/services", () => ({
    ObjectService: {
        UpdateObjectMeta: (_oref: string, meta: Record<string, unknown>) => {
            hub.updates.push(meta);
            return Promise.resolve();
        },
    },
}));
vi.mock("@/app/store/global", () => ({
    MOS: { getMuxObjectAtom: () => () => ({ meta: hub.blockMeta }) },
}));

import { META_AWAITING_USER } from "@/app/store/swarm-line";
import { useAgentQuestions } from "./useAgentQuestions";
import { __resetWaitingForYou } from "@/app/notification/waiting-for-you";

function question(): ToolNode {
    return {
        type: "tool",
        id: "n1",
        name: "AskUserQuestion",
        status: "awaiting_answer",
        question: { tool_use_id: "tu-1", questions: [{ question: "Pick one", options: ["a", "b"] }] },
    } as unknown as ToolNode;
}

const awaitingWrites = () => hub.updates.filter((u) => META_AWAITING_USER in u).map((u) => u[META_AWAITING_USER]);

function mount(initial: DocumentNode[]) {
    const [doc, setDoc] = createSignal<DocumentNode[]>(initial);
    let dispose = () => {};
    createRoot((d) => {
        dispose = d;
        useAgentQuestions({ blockId: "b1", getDocument: doc, sendMessage: async () => {}, log: () => {} });
    });
    return { setDoc, dispose };
}

beforeEach(() => {
    __resetWaitingForYou();
    hub.updates.length = 0;
    hub.blockMeta = {};
});
afterEach(() => vi.clearAllMocks());

describe("term:awaiting_user", () => {
    it("is set when a question appears and removed when it is answered", async () => {
        const { setDoc, dispose } = mount([]);
        expect(awaitingWrites()).toEqual([]);

        setDoc([question()]);
        await Promise.resolve();
        expect(awaitingWrites()).toEqual([true]);

        setDoc([]);
        await Promise.resolve();
        expect(awaitingWrites()).toEqual([true, null]);
        dispose();
    });

    it("is NOT removed when the pane unmounts with the question still pending", async () => {
        const { dispose } = mount([question()]);
        await Promise.resolve();
        expect(awaitingWrites()).toEqual([true]);

        dispose(); // a tab switch: the pane goes away, the question does not
        await Promise.resolve();
        expect(awaitingWrites()).toEqual([true]);
    });

    it("clears a stale flag at mount when nothing is pending", async () => {
        hub.blockMeta = { [META_AWAITING_USER]: true };
        const { dispose } = mount([]);
        await Promise.resolve();
        expect(awaitingWrites()).toEqual([null]);
        dispose();
    });

    it("writes nothing at mount when there is no flag and nothing pending", async () => {
        const { dispose } = mount([]);
        await Promise.resolve();
        expect(hub.updates).toEqual([]);
        dispose();
    });
});
