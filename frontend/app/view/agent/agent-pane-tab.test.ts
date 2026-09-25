// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    MOS: { makeORef: (t: string, id: string) => `${t}:${id}` },
    atoms: { fullConfigAtom: () => ({ widgets: {} }) },
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { describePaneTab } from "@/element/pane-tab-model";
import { agentTabIcon } from "./agent-pane-tab";

function describeAgent(meta: Record<string, unknown>) {
    return describePaneTab({ blockId: "b1", view: "agent", meta: { view: "agent", ...meta }, ordinal: 1, liveViewModel: null });
}

describe("agent pane tab", () => {
    it("shows the provider's brand logo, resolving aliases", () => {
        expect(agentTabIcon({ agentProvider: "claude" })).toEqual({ kind: "provider", provider: "claude" });
    });

    it("falls back to a quick-launch agentId that names a provider", () => {
        expect(agentTabIcon({ agentId: "claude" })).toEqual({ kind: "provider", provider: "claude" });
    });

    it("an unlaunched picker tab still has an icon and reads as 'Agent' (PR #3341)", () => {
        const tab = describeAgent({});
        expect(tab.icon).toEqual({ kind: "fa", name: "sparkles" });
        expect(tab.label).toBe("Agent");
        expect(tab.rename).toBeUndefined();
    });

    it("labels a launched agent by name and lets it be renamed", () => {
        const tab = describeAgent({ agentName: "Camper", agentId: "def-1", agentProvider: "claude" });
        expect(tab.label).toBe("Camper");
        expect(tab.icon).toEqual({ kind: "provider", provider: "claude" });
        expect(tab.rename).toBeTypeOf("function");
    });

    it("a history tab reads as \"<agent>'s History\" and can't rename the shared definition", () => {
        const tab = describeAgent({ agentName: "Camper", agentId: "def-1", "agent:historyTabFor": "b0" });
        expect(tab.label).toBe("Camper's History");
        expect(tab.rename).toBeUndefined();
    });

    it("a history tab with no agent name still says whose kind of pane it is", () => {
        const tab = describeAgent({ "agent:historyTabFor": "b0" });
        expect(tab.label).toBe("Agent's History");
    });
});
