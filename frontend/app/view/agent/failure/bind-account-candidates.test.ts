// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { computeAccountBindCandidates } from "./bind-account-candidates";
import type { Account } from "@/app/view/identity/identity-model";

const mkAccount = (overrides: Partial<Account> = {}): Account => ({
    id: "acct-1",
    name: "work-claude",
    // CLI-OAuth accounts (claude/codex/gemini/…) store the canonical AGENT
    // provider-catalog id here, not one of `AccountProvider`'s narrower
    // service-account literals — see `ClaudeLoginPanel.tsx`'s "successful
    // login here links the CANONICAL 'claude' provider" and
    // `bind-to-agent-menu.ts`'s identical `resolveProviderAlias(account.provider)`
    // usage, both of which only make sense against real CLI ids. Cast past
    // the (stale, service-account-oriented) `AccountProvider` union.
    provider: "claude" as Account["provider"],
    kind: "oauth",
    secret_ref: { backend: "oauth_config_dir", dir: "/tmp/acct-1" },
    context: {},
    assigned_agents: [],
    status: "valid",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
});

describe("computeAccountBindCandidates", () => {
    it("returns an oauth-class account matching the agent's provider", () => {
        const accounts = [mkAccount()];
        expect(computeAccountBindCandidates("claude", accounts)).toEqual(accounts);
    });

    it("canonicalizes provider aliases on both sides", () => {
        // "claude-code" is a real legacy alias of "claude" in this codebase's
        // provider catalog (PROVIDER_ALIASES) — an account stored under the
        // alias must still match an agent declared under the canonical id,
        // and vice versa.
        const accounts = [mkAccount({ provider: "claude-code" as Account["provider"] })];
        expect(computeAccountBindCandidates("claude", accounts)).toHaveLength(1);
    });

    it("excludes accounts for a different provider", () => {
        const accounts = [mkAccount({ provider: "codex" as Account["provider"] })];
        expect(computeAccountBindCandidates("claude", accounts)).toEqual([]);
    });

    it("excludes non-OAuth (api-key/service) accounts", () => {
        const accounts = [mkAccount({ secret_ref: { backend: "env", env_var: "GH_TOKEN" } })];
        expect(computeAccountBindCandidates("claude", accounts)).toEqual([]);
    });

    it("excludes the account already linked to this agent", () => {
        const accounts = [mkAccount({ id: "acct-1" }), mkAccount({ id: "acct-2", name: "personal-claude" })];
        const result = computeAccountBindCandidates("claude", accounts, "acct-1");
        expect(result.map((a) => a.id)).toEqual(["acct-2"]);
    });

    it("excludes expired/invalid/unknown/checking accounts entirely, not just deprioritizes them", () => {
        // Amended 2026-09-05 (reagentx P1): offering a KNOWN-bad account as a
        // one-click "fix" interacts badly with recheckAuthAfterBind's trust
        // in CheckCliAuthCommand's expired-but-present false positive — see
        // this module's own doc comment. An expired candidate must not
        // appear at all, not just sort after valid ones.
        const accounts = (["expired", "invalid", "unknown", "checking"] as const).map((status) =>
            mkAccount({ id: status, status }),
        );
        expect(computeAccountBindCandidates("claude", accounts)).toEqual([]);
    });

    it("sorts most-recently-updated first among valid accounts", () => {
        const accounts = [
            mkAccount({ id: "older", updated_at: "2026-01-01T00:00:00Z" }),
            mkAccount({ id: "newer", updated_at: "2026-02-01T00:00:00Z" }),
        ];
        const result = computeAccountBindCandidates("claude", accounts);
        expect(result.map((a) => a.id)).toEqual(["newer", "older"]);
    });
});
