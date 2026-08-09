// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tests for the "Bind to Agent" menu's pure candidate computation —
// SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md §3/§5.

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ WOS: {} }));
vi.mock("@/app/store/agent-pane-state-store", () => ({ getOpenDefinitionMap: () => new Map() }));

import { computeBindCandidates, candidateSublabel } from "./bind-to-agent-menu";
import type { Account } from "./identity-model";

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

describe("computeBindCandidates", () => {
    it("CLI-OAuth account only offers same-provider agents", () => {
        const out = computeBindCandidates(
            mkAccount(),
            [mkAgent({ id: "a", name: "Claude1", provider: "claude" }), mkAgent({ id: "b", name: "Codex1", provider: "codex" })],
            NO_LINKS,
            NO_OPEN,
            NO_NAMES,
        );
        expect(out.map((c) => c.agentName)).toEqual(["Claude1"]);
    });

    it("canonicalizes provider aliases on BOTH sides (codex P1 on #2377 class)", () => {
        // Account stored under a legacy alias; agent under the canonical id.
        const out = computeBindCandidates(
            mkAccount({ provider: "claude-code" as Account["provider"] }),
            [mkAgent({ provider: "claude" })],
            NO_LINKS,
            NO_OPEN,
            NO_NAMES,
        );
        expect(out).toHaveLength(1);
    });

    it("service (api-key) account offers every agent regardless of provider", () => {
        const gh = mkAccount({
            provider: "github" as Account["provider"],
            kind: "pat",
            secret_ref: { backend: "keychain", service: "s", account: "a" },
        });
        const out = computeBindCandidates(
            gh,
            [mkAgent({ id: "a", name: "Claude1", provider: "claude" }), mkAgent({ id: "b", name: "Codex1", provider: "codex" })],
            NO_LINKS,
            NO_OPEN,
            NO_NAMES,
        );
        expect(out).toHaveLength(2);
    });

    it("excludes seeded templates", () => {
        const out = computeBindCandidates(
            mkAccount(),
            [mkAgent({ is_seeded: 1 })],
            NO_LINKS,
            NO_OPEN,
            NO_NAMES,
        );
        expect(out).toHaveLength(0);
    });

    it("marks boundHere for the agent linked to this account, even via a legacy alias link", () => {
        const links: AgentDefinitionIdentity[] = [
            { agent_id: "agent-1", account_id: "acct-1", provider: "claude-code" },
        ];
        const out = computeBindCandidates(mkAccount(), [mkAgent()], links, NO_OPEN, NO_NAMES);
        expect(out[0].boundHere).toBe(true);
        expect(out[0].boundElsewhereName).toBeNull();
    });

    it("names the other account when bound elsewhere", () => {
        const links: AgentDefinitionIdentity[] = [
            { agent_id: "agent-1", account_id: "acct-OTHER", provider: "claude" },
        ];
        const names = new Map([["acct-OTHER", "personal-claude"]]);
        const out = computeBindCandidates(mkAccount(), [mkAgent()], links, NO_OPEN, names);
        expect(out[0].boundHere).toBe(false);
        expect(out[0].boundElsewhereName).toBe("personal-claude");
    });

    it("sorts running agents first, then by name", () => {
        const agents = [
            mkAgent({ id: "z", name: "Zeta" }),
            mkAgent({ id: "a", name: "Alpha" }),
            mkAgent({ id: "r", name: "RunningOne" }),
        ];
        const open = new Map([["r", "block-r"]]);
        const out = computeBindCandidates(mkAccount(), agents, NO_LINKS, open, NO_NAMES);
        expect(out.map((c) => c.agentName)).toEqual(["RunningOne", "Alpha", "Zeta"]);
        expect(out[0].runningBlockId).toBe("block-r");
    });
});

describe("candidateSublabel", () => {
    const base = { agentId: "a", agentName: "A", runningBlockId: null, boundHere: false, boundElsewhereName: null };

    it("unbound + not running → 'no account bound'", () => {
        expect(candidateSublabel({ ...base })).toBe("no account bound");
    });

    it("bound elsewhere shows the other account's name", () => {
        expect(candidateSublabel({ ...base, boundElsewhereName: "personal" })).toBe("bound: personal");
    });

    it("boundHere carries no binding text (checkmark covers it) but keeps the running marker", () => {
        expect(candidateSublabel({ ...base, boundHere: true, runningBlockId: "b" })).toBe("● running");
    });

    it("running + bound elsewhere combines both", () => {
        expect(candidateSublabel({ ...base, runningBlockId: "b", boundElsewhereName: "x" })).toBe(
            "● running  ·  bound: x",
        );
    });
});
