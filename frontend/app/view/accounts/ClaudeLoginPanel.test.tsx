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
 *
 * Uses `screen.findByText` (document-wide), not `render()`'s own bound
 * `findByText` — ClaudeLoginPanel now renders through the canonical `Modal`
 * (ANALYSIS_ARMORY_STASH_CREDENTIAL_VISIBILITY_GAP_2026_08_04.md's Fix 3),
 * which `<Portal>`s outside the test's render container; container-scoped
 * queries can't see it.
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
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
import { Modal } from "@/element/modal";
import { createSignal } from "solid-js";

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

        render(() => (
            <ClaudeLoginPanel onClose={() => {}} linkTarget={{ agentDefinitionId: "agent-1" }} />
        ));

        await screen.findByText(/couldn't confirm the account was linked/i);
        expect(hub.refreshAccountCache).not.toHaveBeenCalled();
        expect(hub.unlinkAgentIdentity).not.toHaveBeenCalled();
    });

    it("reagent P2 on PR #2414 (round 4): Retry after a link-verification failure reuses the account THIS panel already minted, instead of re-minting a second one", async () => {
        // First attempt: Armory's bare-Connect shape (no existingAccountId
        // prop) mints a NEW account and persists it, but the agent-link
        // check fails — mirrors the test above, just with a Stash
        // linkTarget added so there's an agent to (fail to) link to.
        hub.runProviderLogin.mockImplementationOnce(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });
        hub.listAgentIdentities.mockResolvedValueOnce([]); // link never landed

        render(() => (
            <ClaudeLoginPanel onClose={() => {}} linkTarget={{ agentDefinitionId: "agent-1" }} />
        ));
        const retryButton = await screen.findByText("Retry");

        // Second attempt (Retry): this time the link verification succeeds.
        hub.runProviderLogin.mockImplementationOnce(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });
        hub.listAgentIdentities.mockResolvedValueOnce([
            { agent_id: "agent-1", account_id: "acct-new", provider: "claude" },
        ]);
        retryButton.click();

        await screen.findByText(/signed in to anthropic/i);
        // The whole point: Retry must refresh the SAME account
        // ("acct-new") the first attempt already minted and persisted, not
        // mint a brand-new one under the still-undefined original prop —
        // that would orphan "acct-new" as a real, credentialed, unlinked
        // Claude account.
        expect(hub.runProviderLogin).toHaveBeenLastCalledWith(
            expect.objectContaining({ existingAccountId: "acct-new" }),
        );
    });

    it("shows success when the link IS confirmed present for this agent", async () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });

        render(() => (
            <ClaudeLoginPanel onClose={() => {}} linkTarget={{ agentDefinitionId: "agent-1" }} />
        ));

        await screen.findByText(/signed in to anthropic/i);
        expect(hub.refreshAccountCache).toHaveBeenCalled();
    });

    it("reagent P0 on PR #2414 (round 3): does NOT show success when the only matching row is the STALE ALIAS link, not the canonical 'claude' one — a Stash re-login refreshes the SAME account_id, so an account_id-only check is vacuously satisfied by the pre-existing alias row even when the new canonical link insert silently failed", async () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            // A re-login: onAccountRegistered fires with the SAME account
            // id the alias row already points at (registeredAccountId ===
            // existingAccountId), exactly as run-provider-login.ts does for
            // a refresh rather than a fresh mint.
            opts.onAccountRegistered?.("acct-existing", "/tmp/acct-existing");
            return "inapp-success";
        });
        // Only the OLD alias row is present — the canonical "claude" link
        // insert silently failed (finalizeAccount swallows that error).
        hub.listAgentIdentities.mockResolvedValue([
            { agent_id: "agent-1", account_id: "acct-existing", provider: "claude-code" },
        ]);

        render(() => (
            <ClaudeLoginPanel
                onClose={() => {}}
                existingAccountId="acct-existing"
                linkTarget={{ agentDefinitionId: "agent-1" }}
                staleAliasProvider="claude-code"
            />
        ));

        await screen.findByText(/couldn't confirm the account was linked/i);
        // Must NOT proceed to the stale-alias cleanup unlink — that would
        // delete the one link that actually works, leaving zero links.
        expect(hub.unlinkAgentIdentity).not.toHaveBeenCalled();
        expect(hub.refreshAccountCache).not.toHaveBeenCalled();
    });

    it("skips link verification entirely for Armory's bare Connect (no linkTarget) — nothing to verify", async () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });

        render(() => <ClaudeLoginPanel onClose={() => {}} />);

        await screen.findByText(/signed in to anthropic/i);
        expect(hub.listAgentIdentities).not.toHaveBeenCalled();
    });
});

describe("ClaudeLoginPanel — unmount cleanup (codex P2 on PR #2414)", () => {
    it("does NOT cancel the host's login when unmounting after already reaching success (inFlight is false)", async () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.onAccountRegistered?.("acct-new", "/tmp/acct-new");
            return "inapp-success";
        });

        const { unmount } = render(() => (
            <ClaudeLoginPanel onClose={() => {}} linkTarget={{ agentDefinitionId: "agent-1" }} />
        ));
        await screen.findByText(/signed in to anthropic/i);
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

        render(() => <ClaudeLoginPanel onClose={() => {}} />);

        await screen.findByText(/PTY spawn failed/i);
        expect(hub.cancelCliLogin).toHaveBeenCalled();
    });
});

describe("ClaudeLoginPanel — nested modal stacking (Fix 3 of ANALYSIS_ARMORY_STASH_CREDENTIAL_VISIBILITY_GAP_2026_08_04.md)", () => {
    // ClaudeLoginPanel's own `Modal` is opened from INSIDE the Agent Stash's
    // Accounts tab, which is itself already inside a canonical `Modal`
    // (AgentStashModal, dispatched via modal-dispatch.tsx/ModalLayer.tsx).
    // The real risk this guards against: if ClaudeLoginPanel were wired
    // through `useModalLayer().open(...)` instead of a plain `<Modal>`
    // (a mistake that's easy to make — that's the established pattern for
    // most of this app's other dialogs), it would hit ModalLayer's
    // single-`current`-signal "replace" semantics and CLOSE the Stash
    // modal it's nested in, instead of stacking on top of it. Standing up
    // the full `AgentStashModal` (many unrelated tabs/RPC deps) isn't
    // needed to exercise this — the behavior lives entirely in the shared
    // `Modal`/`modal-stack.ts` primitive, so a minimal outer `<Modal>`
    // standing in for AgentStashModal is a faithful, much cheaper test of
    // the same mechanism.
    it("opening ClaudeLoginPanel from inside another open Modal does not close that outer Modal", async () => {
        const [outerOpen, setOuterOpen] = createSignal(true);
        const [innerOpen, setInnerOpen] = createSignal(false);

        render(() => (
            <Modal open={outerOpen()} onClose={() => setOuterOpen(false)} ariaLabel="Agent Stash">
                <div>Stash content marker</div>
                <button onClick={() => setInnerOpen(true)}>Connect</button>
                {innerOpen() && <ClaudeLoginPanel onClose={() => setInnerOpen(false)} />}
            </Modal>
        ));

        expect(screen.getByRole("dialog", { name: "Agent Stash" })).toBeInTheDocument();

        screen.getByText("Connect").click();

        // Both dialogs must be simultaneously present — the outer one must
        // NOT have been closed/replaced by the inner one opening.
        await waitFor(() => {
            expect(screen.getByRole("dialog", { name: "Connect Anthropic" })).toBeInTheDocument();
        });
        expect(screen.getByRole("dialog", { name: "Agent Stash" })).toBeInTheDocument();
        expect(screen.getByText("Stash content marker")).toBeInTheDocument();
    });
});
