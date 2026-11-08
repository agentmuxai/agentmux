// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/global", () => ({
    MOS: { makeORef: (t: string, id: string) => `${t}:${id}` },
    atoms: { fullConfigAtom: () => ({ widgets: {} }) },
}));
const renameCommand = vi.fn(async (_client: unknown, _args: unknown) => ({}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { RenameAgentDefinitionTitleCommand: (c: unknown, a: unknown) => renameCommand(c, a) } }));
const setBlockMeta = vi.fn(async (_blockId: string, _meta: Record<string, unknown>) => {});
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: (b: string, m: Record<string, unknown>) => setBlockMeta(b, m) }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { describePaneTab, PaneTabIconView } from "@/element/pane-tab-model";
import { registerPaneTab } from "@/app/store/pane-tab-registry";
import { stubPaneTab } from "@/app/block/pane-tab-test-utils";
import { agentPaneTab, agentTabIcon } from "./agent-pane-tab";

// As in block-registry.ts's agent manifest.
registerPaneTab(stubPaneTab("agent", { label: "Agent", icon: "sparkles", tab: agentPaneTab }));

function describeAgent(meta: Record<string, unknown>) {
    return describePaneTab({ blockId: "b1", view: "agent", meta: { view: "agent", ...meta }, ordinal: 1, liveViewModel: null });
}

afterEach(() => cleanup());

describe("agent pane tab", () => {
    it("shows the provider's brand logo, resolving aliases", () => {
        expect(agentTabIcon({ agentProvider: "claude" })).toEqual({ kind: "provider", provider: "claude", vendor: "anthropic" });
    });

    it("falls back to a quick-launch agentId that names a provider", () => {
        expect(agentTabIcon({ agentId: "claude" })).toEqual({ kind: "provider", provider: "claude", vendor: "anthropic" });
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
        expect(tab.icon).toEqual({ kind: "provider", provider: "claude", vendor: "anthropic" });
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

    it("a rename renames the definition and clears an earlier pane title, so the new name shows", async () => {
        const tab = describeAgent({ agentId: "def-1", agentName: "Korp", "frame:title": "old pane title" });
        expect(tab.rename).toBeDefined();
        await tab.rename!("BrowserLeader");
        expect(renameCommand).toHaveBeenCalledWith(expect.anything(), { id: "def-1", title: "BrowserLeader" });
        expect(setBlockMeta).toHaveBeenCalledWith("b1", { agentName: "BrowserLeader", "frame:title": null });
    });

    const ROUTED = {
        route: "openrouter",
        route_label: "OpenRouter",
        model: "deepseek/deepseek-v4.1-flash-1017",
        label: "DeepSeek V4.1 Flash",
    };

    it("the pill's icon is the harness with the model answering as its badge, and follows a switch", () => {
        const meta = { agentProvider: "claude", "agent:route_model": ROUTED };
        expect(agentTabIcon(meta)).toEqual({ kind: "provider", provider: "claude", vendor: "deepseek" });
        const switched = { ...meta, "agent:route_model": { ...ROUTED, model: "moonshotai/kimi-k3", label: "Kimi K3" } };
        expect(agentTabIcon(switched)).toEqual({ kind: "provider", provider: "claude", vendor: "moonshot" });
        expect(agentTabIcon({ ...meta, "agent:route_model": null })).toEqual({
            kind: "provider",
            provider: "claude",
            vendor: "anthropic",
        });
    });

    it("the pill's tooltip names the harness and the model", () => {
        const routed = describeAgent({ agentProvider: "claude", "agent:route_model": ROUTED });
        expect(routed.modelLine).toBe("Claude Code harness running on DeepSeek V4.1 Flash via OpenRouter");
        const plain = describeAgent({ agentProvider: "claude" });
        expect(plain.modelLine).toBe("Claude Code harness running on Anthropic");
        expect(describeAgent({}).modelLine).toBeUndefined();
    });

    it("draws the badge on the pill's 14 px icon, and none when the harness answers itself", () => {
        const badged = render(() => <PaneTabIconView icon={() => agentTabIcon({ agentProvider: "claude", "agent:route_model": ROUTED })} />);
        expect(badged.container.querySelector(".dual-provider-logo-badge")).not.toBeNull();
        cleanup();
        const bare = render(() => <PaneTabIconView icon={() => ({ kind: "provider", provider: "codex" })} />);
        expect(bare.container.querySelector(".dual-provider-logo-badge")).toBeNull();
        expect(bare.container.querySelector(".provider-logo")).not.toBeNull();
    });
});
