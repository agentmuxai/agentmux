// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { joinAgentIdentityRows } from "./agent-identities-model";
import type { Account } from "./identity-model";

function mkAccount(overrides: Partial<Account> & Pick<Account, "id">): Account {
    return {
        name: "test-account",
        provider: "github",
        kind: "pat",
        secret_ref: { backend: "keychain" },
        context: {},
        assigned_agents: [],
        status: "valid",
        created_at: "",
        updated_at: "",
        ...overrides,
    };
}

function mkLink(overrides: Partial<AgentDefinitionIdentity>): AgentDefinitionIdentity {
    return {
        agent_id: "agent-1",
        account_id: "acc-1",
        provider: "github",
        ...overrides,
    };
}

describe("joinAgentIdentityRows", () => {
    it("returns an empty array when the agent has no links", () => {
        const rows = joinAgentIdentityRows("agent-1", [], new Map());
        expect(rows).toEqual([]);
    });

    it("filters links to only the requested agent", () => {
        const links = [
            mkLink({ agent_id: "agent-1", provider: "github", account_id: "acc-1" }),
            mkLink({ agent_id: "agent-2", provider: "claude", account_id: "acc-2" }),
        ];
        const rows = joinAgentIdentityRows("agent-1", links, new Map());
        expect(rows).toHaveLength(1);
        expect(rows[0].provider).toBe("github");
    });

    it("joins each row against the account cache by account id", () => {
        const acc = mkAccount({ id: "acc-1", name: "work-gh" });
        const links = [mkLink({ agent_id: "agent-1", provider: "github", account_id: "acc-1" })];
        const rows = joinAgentIdentityRows("agent-1", links, new Map([["acc-1", acc]]));
        expect(rows[0].account).toBe(acc);
        expect(rows[0].accountId).toBe("acc-1");
    });

    it("returns account: null for a link whose account was deleted (orphan), without throwing", () => {
        const links = [mkLink({ agent_id: "agent-1", provider: "github", account_id: "acc-deleted" })];
        const rows = joinAgentIdentityRows("agent-1", links, new Map());
        expect(rows).toHaveLength(1);
        expect(rows[0].account).toBeNull();
        expect(rows[0].accountId).toBe("acc-deleted");
    });

    it("handles multiple providers for the same agent", () => {
        const links = [
            mkLink({ agent_id: "agent-1", provider: "github", account_id: "acc-1" }),
            mkLink({ agent_id: "agent-1", provider: "claude", account_id: "acc-2" }),
        ];
        const rows = joinAgentIdentityRows("agent-1", links, new Map());
        expect(rows.map((r) => r.provider).sort()).toEqual(["claude", "github"]);
    });
});
