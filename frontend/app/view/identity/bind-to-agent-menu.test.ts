// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tests for the "Bind to Agent" menu's pure candidate computation —
// SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md §3/§5.

import { beforeEach, describe, expect, it, vi } from "vitest";

const rpcCalls: Array<{ cmd: string; data: unknown }> = [];
const listAgentIdentitiesMock = vi.fn(async (): Promise<AgentDefinitionIdentity[]> => []);
const getMemoryMock = vi.fn(async (_c: unknown, data: { id: string }) => undefined as any);
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListAgentIdentitiesCommand: (_c: unknown, data: { agent_id: string }) => {
            rpcCalls.push({ cmd: "list", data });
            return listAgentIdentitiesMock();
        },
        UnlinkAgentIdentityCommand: async (_c: unknown, data: unknown) => {
            rpcCalls.push({ cmd: "unlink", data });
        },
        LinkAgentIdentityCommand: async (_c: unknown, data: unknown) => {
            rpcCalls.push({ cmd: "link", data });
        },
        ListAllAgentIdentitiesCommand: vi.fn(async (): Promise<AgentDefinitionIdentity[]> => []),
        // Backs `resolveEffectiveLaunchProvider`'s bound-bundle resolution
        // (#2594) — resolves to `undefined` by default so agents without
        // `memory_id` never even trigger a fetch; the drift regression
        // tests below set their own `.mockResolvedValue`.
        GetBundleCommand: (c: unknown, data: { id: string }) => getMemoryMock(c, data),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ MOS: {}, workspace: () => null }));
vi.mock("@/app/store/agent-pane-state-store", () => ({ getOpenDefinitionMap: () => new Map() }));

import { bindAccountToAgent, buildAccountRowMenu } from "./bind-to-agent-menu";
import { RpcApi } from "@/app/store/rpc-api";
import { MENU_ICON } from "@/app/util/menu-icons";
import type { Account } from "./identity-model";
import type { AgentDefinition, AgentDefinitionIdentity } from "@/app/store/rpc-api";

function mkAccount(over: Partial<Account> = {}): Account {
    return {
        id: "acct-1",
        name: "work-claude",
        provider: "claude" as Account["provider"],
        kind: "oauth",
        secret_ref: { backend: "oauth_config_dir", dir: "/tmp/acct-1" },
        context: {},
        assigned_agents: [],
        status: "valid",
        created_at: "0",
        updated_at: "0",
        ...over,
    };
}

function mkAgent(over: Partial<AgentDefinition> = {}): AgentDefinition {
    return {
        id: "agent-1",
        name: "AgentA",
        provider: "claude",
        is_seeded: 0,
        ...over,
    } as AgentDefinition;
}

const NO_LINKS: AgentDefinitionIdentity[] = [];
const NO_OPEN = new Map<string, string>();
const NO_NAMES = new Map<string, string>();

describe("bindAccountToAgent — alias-row cleanup (reagentx P1 on #2485, round 2)", () => {
    const CANDIDATE = {
        agentId: "agent-1",
        agentName: "AgentA",
        runningBlockId: null,
        boundHere: false,
        boundElsewhereName: null,
    };

    beforeEach(() => {
        rpcCalls.length = 0;
        listAgentIdentitiesMock.mockReset();
        listAgentIdentitiesMock.mockResolvedValue([]);
    });

    it("unlinks an alias-stored link for the same canonical provider BEFORE linking", async () => {
        // Existing link under legacy alias "claude-code" — the backend's
        // upsert (keyed on the raw provider string) would NOT replace it,
        // and injection's ORDER BY provider would apply it last, silently
        // keeping the old account.
        listAgentIdentitiesMock.mockResolvedValue([
            { agent_id: "agent-1", account_id: "acct-OLD", provider: "claude-code" },
        ]);
        await bindAccountToAgent(mkAccount(), CANDIDATE);

        const unlinks = rpcCalls.filter((c) => c.cmd === "unlink");
        expect(unlinks).toHaveLength(1);
        expect(unlinks[0].data).toEqual({ agent_id: "agent-1", provider: "claude-code", silent: true });
        // Cleanup precedes the link.
        expect(rpcCalls.map((c) => c.cmd)).toEqual(["list", "unlink", "link"]);
        expect(rpcCalls[2].data).toMatchObject({ account_id: "acct-1", provider: "claude" });
    });

    it("does not unlink a canonical-provider link (the upsert replaces it) or other providers' links", async () => {
        listAgentIdentitiesMock.mockResolvedValue([
            { agent_id: "agent-1", account_id: "acct-OLD", provider: "claude" },
            { agent_id: "agent-1", account_id: "acct-gh", provider: "github" },
        ]);
        await bindAccountToAgent(mkAccount(), CANDIDATE);
        expect(rpcCalls.filter((c) => c.cmd === "unlink")).toHaveLength(0);
        expect(rpcCalls.filter((c) => c.cmd === "link")).toHaveLength(1);
    });
});

describe("buildAccountRowMenu — Copy account ID and Delete account… (SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04 §3.1)", () => {
    it("has no Bind to Agent: an agent's account is changed from its composer", () => {
        const menu = buildAccountRowMenu(mkAccount(), vi.fn());
        expect(menu.some((m) => m.label === "Bind to Agent")).toBe(false);
        expect(menu.map((m) => m.label)).toEqual(["Copy account ID", undefined, "Delete account…"]);
    });

    it("Delete is the last item, after a separator, and opens the confirmation", () => {
        const onDelete = vi.fn();
        const menu = buildAccountRowMenu(mkAccount(), onDelete);
        const last = menu[menu.length - 1];
        expect(last.label).toBe("Delete account…");
        expect(menu[menu.length - 2].type).toBe("separator");
        last.click?.();
        expect(onDelete).toHaveBeenCalledTimes(1);
    });

    it("gives Copy account ID and Delete account their icons", () => {
        const menu = buildAccountRowMenu(mkAccount(), vi.fn());
        const icon = (label: string) => menu.find((m) => m.label === label)?.icon;
        expect(icon("Copy account ID")).toBe(MENU_ICON.copy);
        expect(icon("Delete account…")).toBe(MENU_ICON.delete);
    });

    it("is absent when the caller offers no delete", () => {
        const menu = buildAccountRowMenu(mkAccount());
        expect(menu.some((m) => m.label === "Delete account…")).toBe(false);
    });
});
