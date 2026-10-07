// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentDefinition } from "@/app/store/rpc-api";
import { describe, expect, it, vi } from "vitest";
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
