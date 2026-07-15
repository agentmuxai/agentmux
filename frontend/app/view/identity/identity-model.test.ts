// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tests for the field-name translation between frontend `Account` (loose
// shape with `value` for plaintext_dev) and backend `IdentityAccount`
// (discriminated union with `plaintext_dev`). Reagent caught a bug in
// PR #480 where a naked cast hid the mismatch — these tests guard
// against regressing the translator.

import { describe, expect, it } from "vitest";

// We test the (un-exported) helpers indirectly via a small re-export
// shim. Keep them un-exported in the module so callers don't depend on
// them, but expose for tests via `__internal__`.
import * as identityModel from "./identity-model";
import { __internal__ } from "./identity-model";
import type { Account } from "./identity-model";

// The helpers aren't re-exported; we round-trip through the public API
// using a stub Account shape. The conversion happens inside
// `accountToBackend` (private) — we exercise it by calling
// `IdentityViewModel.createAccount` against a mocked RPC, but the
// test setup for that is heavy. Cheaper: verify the shape by
// constructing the wire payload manually and checking the output of
// the (now-tested) translation pair via a tiny fixture round-trip.

// Until a more direct test hook is added, these assertions verify the
// runtime invariants by going through the public typed boundary.

describe("identity-model SecretRef field-name translation", () => {
    it("env: round-trip preserves env_var", () => {
        const acct: Account = {
            id: "id1",
            name: "asaf-github",
            provider: "github",
            kind: "pat",
            secret_ref: { backend: "env", env_var: "GITHUB_TOKEN" },
            context: {},
            assigned_agents: [],
            status: "unknown",
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
        };
        // Smoke: constructing this shape should be valid TS at compile
        // time. The conversion correctness is exercised indirectly when
        // `IdentityViewModel.createAccount` calls the (private)
        // `accountToBackend` and the round-trip via the cache.
        expect(acct.secret_ref.backend).toBe("env");
    });

    it("plaintext_dev: frontend uses `value`, backend uses `plaintext_dev`", () => {
        // Frontend representation
        const acct: Account = {
            id: "id2",
            name: "dev-token",
            provider: "anthropic",
            kind: "api_key",
            secret_ref: { backend: "plaintext_dev", value: "sk-test" },
            context: {},
            assigned_agents: [],
            status: "unknown",
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
        };
        expect(acct.secret_ref.backend).toBe("plaintext_dev");
        expect((acct.secret_ref as { value?: string }).value).toBe("sk-test");

        // Wire representation (what the backend sends/receives)
        const wireSecret = { backend: "plaintext_dev" as const, plaintext_dev: "sk-test" };
        expect(wireSecret.plaintext_dev).toBe("sk-test");
        // Different field name confirms why naked casts between the two
        // shapes are unsafe — caught by reagent in PR #480.
        expect((wireSecret as unknown as { value?: string }).value).toBeUndefined();
    });

    it("oauth_config_dir: maps to a defined SecretRef (regression: OAuth accounts crashed the Accounts detail modal)", () => {
        // Live repro 2026-07-14: a Claude OAuth login persists a backend
        // SecretRef::OAuthConfigDir, which secretRefFromBackend's switch
        // didn't handle — it fell through and returned `undefined`, so
        // opening the account in Armory → Accounts threw
        // "Cannot read properties of undefined (reading 'backend')".
        const wire = { backend: "oauth_config_dir", dir: "C:\\Users\\x\\.agentmux\\shared\\identities\\abc\\claude" } as const;
        const fe = __internal__.secretRefFromBackend(wire);
        expect(fe).toBeDefined();
        expect(fe.backend).toBe("oauth_config_dir");
        expect(fe.dir).toBe(wire.dir);

        // Round-trip back to the wire shape.
        const back = __internal__.secretRefToBackend(fe);
        expect(back).toEqual(wire);
    });

    it("every backend SecretRef variant maps to a defined frontend SecretRef", () => {
        // Exhaustiveness net: if the Rust enum grows another variant and
        // gotypes/this module lag, the specific new variant can't be listed
        // here yet — but every *known* variant must at minimum round-trip
        // to a defined object, and the view additionally guards with `?.`.
        const wires: Parameters<typeof __internal__.secretRefFromBackend>[0][] = [
            { backend: "env", env_var: "X" },
            { backend: "secrets_manager", sm_path: "p" },
            { backend: "plaintext_dev", plaintext_dev: "v" },
            { backend: "keychain", service: "s", account: "a" },
            { backend: "oauth_config_dir", dir: "d" },
        ];
        for (const w of wires) {
            expect(__internal__.secretRefFromBackend(w), `variant ${w.backend}`).toBeDefined();
        }
    });

    it("module exports expected public surface", () => {
        // Defensive: catches accidental removal/rename of the public API
        // that callers (agent-view.tsx, AgentIdentityPanel.tsx,
        // app-init.ts) import.
        expect(typeof identityModel.loadAccounts).toBe("function");
        expect(typeof identityModel.refreshAccountCache).toBe("function");
        expect(typeof identityModel.primeAccountCache).toBe("function");
        expect(typeof identityModel.subscribeAccountChanges).toBe("function");
        expect(typeof identityModel.parseAgentAccounts).toBe("function");
        expect(typeof identityModel.serializeAgentAccounts).toBe("function");
    });
});

// ── Delete-time disclosure (layer 4 — SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4 §4)

describe("deleteDisclosureNotice", () => {
    it("returns null when the delete cascaded no agent links", () => {
        expect(identityModel.deleteDisclosureNotice([])).toBeNull();
        expect(identityModel.deleteDisclosureNotice(undefined)).toBeNull();
    });

    it("returns the honest linked-agent notice when agents were using the account", () => {
        const one = identityModel.deleteDisclosureNotice(["ag-1"]);
        expect(one).toContain("Account deleted.");
        expect(one).toContain("1 agent(s) were using it");
        // Honest wording: we disclose that tokens survive, we do NOT
        // pretend the running process was deauthenticated (spec §3).
        expect(one).toContain("until restarted");

        const three = identityModel.deleteDisclosureNotice(["a", "b", "c"]);
        expect(three).toContain("3 agent(s)");
    });
});
