// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const { history, append, draft, handlers } = vi.hoisted(() => ({
    history: vi.fn(),
    append: vi.fn(),
    draft: vi.fn(),
    handlers: new Map<string, (data: any) => void>(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentInputHistoryCommand: history,
        AgentInputHistoryAppendCommand: append,
        AgentInputDraftCommand: draft,
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (type: string, handler: (data: any) => void) => {
        handlers.set(type, handler);
        return () => handlers.delete(type);
    },
}));

import {
    agentDraft,
    agentHistory,
    historyAgentId,
    loadOlderAgentHistory,
    rememberSent,
    resetAgentInputHistoryForTests,
    saveAgentDraft,
    sendKind,
    watchAgentHistory,
} from "./agent-input-history";

/** srv's answer: entries newest first, ids counting down from `top`. */
const page = (texts: string[], top: number) => ({
    entries: texts.map((text, i) => ({ id: top - i, agent_id: "a1", sent_at_ms: 0, text, kind: "message", source: "composer", sends: 1 })),
});
const flush = () => new Promise((r) => setTimeout(r, 0));

describe("agent input history", () => {
    beforeEach(() => {
        resetAgentInputHistoryForTests();
        history.mockReset();
        append.mockReset();
        draft.mockReset();
        handlers.clear();
        append.mockResolvedValue({ entry: { id: 99 } });
    });

    it("belongs to a defined agent, not to a quick-launch provider pane", () => {
        expect(historyAgentId({ agentId: "6f1c2d3e-def" })).toBe("6f1c2d3e-def");
        expect(historyAgentId({ agentId: "claude" })).toBeNull();
        expect(historyAgentId({})).toBeNull();
        expect(historyAgentId(undefined)).toBeNull();
        expect(sendKind("/compact")).toBe("slash");
        expect(sendKind("fix it")).toBe("message");
    });

    it("loads the agent's newest entries oldest first, and the cursor moves with them", async () => {
        history.mockResolvedValueOnce(page(["third", "second", "first"], 3));
        const entries = agentHistory("a1");
        let pos = entries.length; // the composer's live position
        watchAgentHistory("a1", (where, n) => {
            if (where === "older") pos += n;
        });
        await flush();
        expect(entries).toEqual(["first", "second", "third"]);
        expect(pos, "still on the live draft").toBe(3);
        expect(history).toHaveBeenCalledWith({}, { agent_id: "a1", before_id: undefined, limit: 200 });
        expect(agentHistory("a1"), "one cache, shared by every pane").toBe(entries);
    });

    it("pages back from the oldest entry until srv has no more", async () => {
        history.mockResolvedValueOnce(page(Array.from({ length: 200 }, (_, i) => `m${200 - i}`), 400));
        const entries = agentHistory("a1");
        await flush();
        history.mockResolvedValueOnce(page(["m0"], 200));
        await loadOlderAgentHistory("a1");
        expect(entries[0]).toBe("m0");
        expect(history).toHaveBeenLastCalledWith({}, { agent_id: "a1", before_id: 201, limit: 200 });
        await loadOlderAgentHistory("a1");
        expect(history, "a short page means nothing older is left").toHaveBeenCalledTimes(2);
    });

    it("a send goes at the end at once and srv keeps it; a repeat isn't added twice", async () => {
        history.mockResolvedValueOnce(page(["older"], 1));
        const entries = agentHistory("a1");
        await flush();
        // Another composer of the same agent, on its live draft.
        let otherPos = entries.length;
        watchAgentHistory("a1", (where, n) => {
            if (where === "newer" && otherPos === entries.length - n) otherPos += n;
        });
        rememberSent("a1", "run the tests");
        rememberSent("a1", "run the tests");
        rememberSent("a1", "   ");
        expect(entries).toEqual(["older", "run the tests"]);
        expect(otherPos, "the other composer stays on its live draft").toBe(2);
        expect(append).toHaveBeenCalledTimes(2);
        expect(append).toHaveBeenCalledWith({}, { agent_id: "a1", text: "run the tests", kind: "message" });
    });

    it("takes what another pane sent, after the newest shown", async () => {
        history.mockResolvedValueOnce(page(["mine"], 5));
        const entries = agentHistory("a1");
        let newer = 0;
        watchAgentHistory("a1", (where, n) => {
            if (where === "newer") newer += n;
        });
        await flush();
        history.mockResolvedValueOnce(page(["from the other pane", "mine"], 6));
        handlers.get("agent:inputhistory")!({ agent_id: "a1", what: "history" });
        await flush();
        expect(entries).toEqual(["mine", "from the other pane"]);
        expect(newer).toBe(1);
    });

    it("takes the entries srv filled from past conversations, before the oldest shown", async () => {
        history.mockResolvedValueOnce(page(["typed today"], 7));
        const entries = agentHistory("a1");
        let pos = entries.length;
        watchAgentHistory("a1", (where, n) => {
            if (where === "older") pos += n;
        });
        await flush();
        // Filled entries have ids below the oldest kept, down past zero.
        history.mockResolvedValueOnce(page(["last week", "last month"], 0));
        handlers.get("agent:inputhistory")!({ agent_id: "a1", what: "backfill" });
        await flush();
        await flush();
        expect(history).toHaveBeenLastCalledWith({}, { agent_id: "a1", before_id: 7, limit: 200 });
        expect(entries).toEqual(["last month", "last week", "typed today"]);
        expect(pos, "still on the live draft").toBe(3);
    });

    it("keeps the draft a moment after the last change, and reads it back", async () => {
        vi.useFakeTimers();
        draft.mockResolvedValue({});
        saveAgentDraft("a1", "half");
        saveAgentDraft("a1", "half a thought");
        expect(draft).not.toHaveBeenCalled();
        vi.advanceTimersByTime(600);
        expect(draft).toHaveBeenCalledTimes(1);
        expect(draft).toHaveBeenCalledWith({}, { agent_id: "a1", text: "half a thought" });
        vi.useRealTimers();
        draft.mockResolvedValueOnce({ text: "half a thought" });
        expect(await agentDraft("a1")).toBe("half a thought");
    });
});
