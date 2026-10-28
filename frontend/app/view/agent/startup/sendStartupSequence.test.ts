// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentDefinition } from "@/app/store/rpc-api";
import { afterEach, describe, expect, it, vi } from "vitest";
import { noteOpenedByPerson, OPENING_SUMMARY_PROMPT, OPEN_NOTE_TTL_MS, takeOpenedByPerson } from "./opening-summary";
import { sendStartupSequence, type StartupSequenceDeps } from "./sendStartupSequence";

const agent = { id: "a1", name: "Ada", slug: "ada" } as unknown as AgentDefinition;

function deps(over: Partial<StartupSequenceDeps> = {}): StartupSequenceDeps & { sent: string[]; logs: string[] } {
    const sent: string[] = [];
    const logs: string[] = [];
    return {
        sessionId: () => undefined,
        agent: () => agent,
        providerDisplayName: () => "Claude",
        workDir: () => "/w",
        version: () => "1.2.3",
        peerAgents: () => [],
        getAgentContent: async (contentType) => (contentType === "startup" ? { content: "freeform" } : null),
        listIdentities: async () => [],
        loadAccounts: () => [],
        send: async (payload) => {
            sent.push(payload);
        },
        log: (tag, text) => logs.push(`${tag}: ${text}`),
        ...over,
        sent,
        logs,
    };
}

describe("sendStartupSequence", () => {
    it("sends one startup payload built from the agent and its startup content", async () => {
        const d = deps();
        await sendStartupSequence(d);
        expect(d.sent).toHaveLength(1);
        expect(d.sent[0]).toContain("Ada");
        expect(d.sent[0]).toContain("freeform");
        expect(d.logs).toContain("agent: sending startup sequence");
    });

    it("does nothing for a resumed session", async () => {
        const getAgentContent = vi.fn();
        const d = deps({ sessionId: () => "s-1", getAgentContent });
        await sendStartupSequence(d);
        expect(d.sent).toEqual([]);
        expect(getAgentContent).not.toHaveBeenCalled();
    });

    it("does nothing without an agent definition", async () => {
        const d = deps({ agent: () => undefined });
        await sendStartupSequence(d);
        expect(d.sent).toEqual([]);
    });

    // Bundles reach the agent through its startup file now
    // (SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md §3.6), not this message.
    it("reads only the freeform startup content, never a startup bundle", async () => {
        const getAgentContent = vi.fn(async (t: string) => (t === "startup" ? { content: "freeform" } : null));
        const d = deps({ getAgentContent });
        await sendStartupSequence(d);
        expect(getAgentContent.mock.calls.map((c) => c[0])).toEqual(["startup"]);
        expect(d.sent[0]).toContain("freeform");
    });

    it("sends nothing when the startup content is the skip sentinel", async () => {
        const d = deps({ getAgentContent: async (t) => (t === "startup" ? { content: "__SKIP__" } : null) });
        await sendStartupSequence(d);
        expect(d.sent).toEqual([]);
    });

    it("survives failing lookups: a failed RPC reads as absent", async () => {
        const d = deps({
            getAgentContent: async () => {
                throw new Error("down");
            },
            listIdentities: async () => {
                throw new Error("down");
            },
        });
        await sendStartupSequence(d);
        expect(d.sent).toHaveLength(1);
    });

    it("logs a warning, not a throw, when sending fails", async () => {
        const d = deps({
            send: async () => {
                throw new Error("no controller");
            },
        });
        await expect(sendStartupSequence(d)).resolves.toBeUndefined();
        expect(d.logs.some((l) => l.startsWith("warn: startup sequence failed"))).toBe(true);
    });
});

describe("the opening update", () => {
    // Nothing leaks from one test into the next: a note is used up by whoever asks.
    afterEach(() => void takeOpenedByPerson("a1"));

    it("goes to a resumed session whose pane the person opened, once", async () => {
        noteOpenedByPerson("a1");
        const d = deps({ sessionId: () => "s-1" });
        await sendStartupSequence(d);
        expect(d.sent).toEqual([OPENING_SUMMARY_PROMPT]);
        // The pane connects again (a login, a restart of its process): no second update.
        await sendStartupSequence(d);
        expect(d.sent).toHaveLength(1);
    });

    it("does not go to a pane that only mounted: a layout restore, a tab switch or a background resume", async () => {
        // None of them goes through the explicit open, so no note exists.
        const d = deps({ sessionId: () => "s-1" });
        await sendStartupSequence(d);
        await sendStartupSequence(d);
        expect(d.sent).toEqual([]);
    });

    it("does not go to a pane opened for another agent", async () => {
        noteOpenedByPerson("someone-else");
        const d = deps({ sessionId: () => "s-1" });
        await sendStartupSequence(d);
        expect(d.sent).toEqual([]);
        expect(takeOpenedByPerson("someone-else")).toBe(true);
    });

    it("is not sent for a new agent: it has nothing to report and gets its usual startup payload", async () => {
        noteOpenedByPerson("a1");
        const d = deps();
        await sendStartupSequence(d);
        expect(d.sent).toHaveLength(1);
        expect(d.sent[0]).not.toBe(OPENING_SUMMARY_PROMPT);
        expect(d.sent[0]).toContain("# Session Context");
        expect(d.sent[0]).not.toContain("Opening update");
    });

    it("a failed send is logged, not thrown", async () => {
        noteOpenedByPerson("a1");
        const d = deps({
            sessionId: () => "s-1",
            send: async () => {
                throw new Error("offline");
            },
        });
        await expect(sendStartupSequence(d)).resolves.toBeUndefined();
        expect(d.logs.some((l) => l.includes("opening update failed"))).toBe(true);
    });
});

describe("the note of an explicit open", () => {
    it("is used up by the first pane that asks", () => {
        noteOpenedByPerson("a2", 1000);
        expect(takeOpenedByPerson("a2", 1500)).toBe(true);
        expect(takeOpenedByPerson("a2", 1600)).toBe(false);
    });

    it("expires: an open that mounted no pane does not fire for a later mount of the same agent", () => {
        noteOpenedByPerson("a3", 1000);
        expect(takeOpenedByPerson("a3", 1000 + OPEN_NOTE_TTL_MS + 1)).toBe(false);
    });

    it("is not there for an agent nobody opened, or without an id", () => {
        expect(takeOpenedByPerson("nobody")).toBe(false);
        expect(takeOpenedByPerson(undefined)).toBe(false);
    });

    it("the summary asks for at most three lines and the usual startup heading", () => {
        expect(OPENING_SUMMARY_PROMPT.startsWith("# Session Context")).toBe(true);
        expect(OPENING_SUMMARY_PROMPT).toContain("at most three short lines");
        expect(OPENING_SUMMARY_PROMPT).toContain("New agent, nothing yet.");
    });
});
