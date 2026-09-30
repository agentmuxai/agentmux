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
        getBundle: async () => null,
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

    it("prefers a selected bundle's instructions over the freeform startup content", async () => {
        const d = deps({
            getAgentContent: async (t) => (t === "startup" ? { content: "freeform" } : { content: " b-7 " }),
            getBundle: async (id) => (id === "b-7" ? { instructions: "from the bundle" } : null),
        });
        await sendStartupSequence(d);
        expect(d.sent[0]).toContain("from the bundle");
        expect(d.sent[0]).not.toContain("freeform");
    });

    it("falls back to the freeform content when the bundle no longer resolves", async () => {
        const d = deps({
            getAgentContent: async (t) => (t === "startup" ? { content: "freeform" } : { content: "gone" }),
            getBundle: async () => null,
        });
        await sendStartupSequence(d);
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
