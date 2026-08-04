// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for ClaudeLoginPanel — specifically the staleAliasProvider
 * cleanup (reagent P0 on PR #2414): a successful login here always links
 * the CANONICAL "claude" provider (finalizeAccount's
 * ON CONFLICT(agent_id, provider) key), so opening this panel from a
 * legacy-aliased row ("claude-code") and succeeding must ALSO unlink the
 * old alias — otherwise the orphaned alias row lingers and the resolver's
 * inject.rs aborts every future spawn on it, even though a healthy
 * canonical "claude" link now exists right alongside it.
 */

import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    runProviderLogin: vi.fn(),
    resolveCliCommand: vi.fn(),
    unlinkAgentIdentity: vi.fn(),
    ensureAuthDir: vi.fn(),
    refreshAccountCache: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    getApi: () => ({
        ensureAuthDir: (...args: unknown[]) => hub.ensureAuthDir(...args),
        cancelCliLogin: () => Promise.resolve(),
    }),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ResolveCliCommand: (...args: unknown[]) => hub.resolveCliCommand(...args),
        UnlinkAgentIdentityCommand: (...args: unknown[]) => hub.unlinkAgentIdentity(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/errors/translate", () => ({
    translateError: (e: any) => ({ title: "Error", message: String(e?.message ?? e), retry: "" }),
}));
vi.mock("@/app/view/agent/flows/run-provider-login", () => ({
    runProviderLogin: (...args: unknown[]) => hub.runProviderLogin(...args),
}));
vi.mock("@/app/view/agent/components/InAppLoginPanel", () => ({
    InAppLoginPanel: () => null,
}));
vi.mock("@/app/view/identity/identity-model", () => ({
    refreshAccountCache: (...args: unknown[]) => hub.refreshAccountCache(...args),
}));

import { ClaudeLoginPanel } from "./ClaudeLoginPanel";

beforeEach(() => {
    hub.runProviderLogin.mockReset();
    hub.resolveCliCommand.mockReset().mockResolvedValue({ cli_path: "/usr/bin/claude" });
    hub.unlinkAgentIdentity.mockReset().mockResolvedValue({ unlinked: true });
    hub.ensureAuthDir.mockReset().mockResolvedValue("/tmp/claude-auth");
    hub.refreshAccountCache.mockReset();
});

afterEach(() => {
    cleanup();
});

describe("ClaudeLoginPanel — staleAliasProvider cleanup (reagent P0 on PR #2414)", () => {
    it("unlinks the stale alias provider after a successful login, using linkTarget's agentDefinitionId", async () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });

        render(() => (
            <ClaudeLoginPanel
                onClose={() => {}}
                existingAccountId="acc-claude-1"
                linkTarget={{ agentDefinitionId: "agent-1" }}
                staleAliasProvider="claude-code"
            />
        ));

        await waitFor(() => {
            expect(hub.unlinkAgentIdentity).toHaveBeenCalledWith(
                {},
                { agent_id: "agent-1", provider: "claude-code" },
            );
        });
        expect(hub.refreshAccountCache).toHaveBeenCalled();
    });

    it("does NOT call UnlinkAgentIdentityCommand when staleAliasProvider is unset (canonical row, or Armory's bare Connect)", async () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });

        render(() => <ClaudeLoginPanel onClose={() => {}} />);

        await waitFor(() => {
            expect(hub.refreshAccountCache).toHaveBeenCalled();
        });
        expect(hub.unlinkAgentIdentity).not.toHaveBeenCalled();
    });

    it("does NOT unlink on a failed/unregistered login — nothing to clean up if the new link never landed", async () => {
        hub.runProviderLogin.mockResolvedValue("inapp-timeout");

        render(() => (
            <ClaudeLoginPanel
                onClose={() => {}}
                linkTarget={{ agentDefinitionId: "agent-1" }}
                staleAliasProvider="claude-code"
            />
        ));

        await waitFor(() => {
            expect(hub.runProviderLogin).toHaveBeenCalled();
        });
        expect(hub.unlinkAgentIdentity).not.toHaveBeenCalled();
    });
});
