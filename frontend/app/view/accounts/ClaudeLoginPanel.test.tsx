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
    listAgentIdentities: vi.fn(),
    ensureAuthDir: vi.fn(),
    refreshAccountCache: vi.fn(),
    cancelCliLogin: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    getApi: () => ({
        ensureAuthDir: (...args: unknown[]) => hub.ensureAuthDir(...args),
        cancelCliLogin: (...args: unknown[]) => hub.cancelCliLogin(...args),
    }),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ResolveCliCommand: (...args: unknown[]) => hub.resolveCliCommand(...args),
        UnlinkAgentIdentityCommand: (...args: unknown[]) => hub.unlinkAgentIdentity(...args),
        ListAgentIdentitiesCommand: (...args: unknown[]) => hub.listAgentIdentities(...args),
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
    // Confirms the link by default — matches onAccountRegistered("acct-new", …)
    // used throughout these tests. Individual tests override to simulate a
    // missing/failed link.
    hub.listAgentIdentities.mockReset().mockResolvedValue([
        { agent_id: "agent-1", account_id: "acct-new", provider: "claude" },
    ]);
    hub.ensureAuthDir.mockReset().mockResolvedValue("/tmp/claude-auth");
    hub.refreshAccountCache.mockReset();
    hub.cancelCliLogin.mockReset().mockResolvedValue(undefined);
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
                // silent: true (codex P2 on PR #2414) — this is an alias
                // migration, not a real unbind; must not trigger the
                // user-facing "Credentials revoked" broadcast.
                { agent_id: "agent-1", provider: "claude-code", silent: true },
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

describe("ClaudeLoginPanel — Stash link verification (codex P1 on PR #2414)", () => {
    it("does NOT show success for a Stash flow (linkTarget set) when the account was persisted but the agent link itself never landed", async () => {
        // finalizeAccount (run-provider-login.ts) catches and only logs a
        // LinkAgentIdentityCommand failure — onAccountRegistered still
        // fires. This is exactly that: the account exists, but no link row
        // for this agent shows up when we check.
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });
        hub.listAgentIdentities.mockResolvedValue([]); // no link for this agent

        const { findByText } = render(() => (
            <ClaudeLoginPanel onClose={() => {}} linkTarget={{ agentDefinitionId: "agent-1" }} />
        ));

        await findByText(/couldn't confirm the account was linked/i);
        expect(hub.refreshAccountCache).not.toHaveBeenCalled();
        expect(hub.unlinkAgentIdentity).not.toHaveBeenCalled();
    });

    it("shows success when the link IS confirmed present for this agent", async () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });

        const { findByText } = render(() => (
            <ClaudeLoginPanel onClose={() => {}} linkTarget={{ agentDefinitionId: "agent-1" }} />
        ));

        await findByText(/signed in to claude/i);
        expect(hub.refreshAccountCache).toHaveBeenCalled();
    });

    it("skips link verification entirely for Armory's bare Connect (no linkTarget) — nothing to verify", async () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });

        const { findByText } = render(() => <ClaudeLoginPanel onClose={() => {}} />);

        await findByText(/signed in to claude/i);
        expect(hub.listAgentIdentities).not.toHaveBeenCalled();
    });
});

describe("ClaudeLoginPanel — unmount cleanup (codex P2 on PR #2414)", () => {
    it("does NOT cancel the host's login when unmounting after already reaching success (inFlight is false)", async () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });

        const { findByText, unmount } = render(() => (
            <ClaudeLoginPanel onClose={() => {}} linkTarget={{ agentDefinitionId: "agent-1" }} />
        ));
        await findByText(/signed in to claude/i);
        hub.cancelCliLogin.mockClear(); // clear whatever the login flow itself called

        unmount();

        // The whole point: a lingering "✓ Signed in" panel that finally
        // closes must not kill some OTHER, newer login the host's single
        // global slot might hold by then.
        expect(hub.cancelCliLogin).not.toHaveBeenCalled();
    });

    it("DOES cancel the host's login when unmounting while still genuinely in flight", async () => {
        hub.runProviderLogin.mockImplementation(() => new Promise(() => {})); // never resolves

        const { unmount } = render(() => <ClaudeLoginPanel onClose={() => {}} />);
        await Promise.resolve();

        unmount();

        expect(hub.cancelCliLogin).toHaveBeenCalled();
    });
});

describe("ClaudeLoginPanel — login-runner rejection (codex P2 on PR #2414)", () => {
    it("surfaces a retryable error instead of leaving the panel stuck when runProviderLogin rejects outright", async () => {
        hub.runProviderLogin.mockRejectedValue(new Error("PTY spawn failed"));

        const { findByText } = render(() => <ClaudeLoginPanel onClose={() => {}} />);

        await findByText(/PTY spawn failed/i);
        expect(hub.cancelCliLogin).toHaveBeenCalled();
    });
});
