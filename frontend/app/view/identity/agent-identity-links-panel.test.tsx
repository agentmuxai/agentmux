// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for `AgentIdentityLinksPanel` — the panel that closes the
 * data gap documented in
 * docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md
 * §1.3: the agent-pane's own `view: "identity"` tab must render only its
 * OWN block's agent's linked accounts, not another agent's, and must
 * degrade gracefully when it has no agent context at all.
 */

import { cleanup, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const listAllAgentIdentities = vi.fn();

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListAllAgentIdentitiesCommand: (...args: unknown[]) => listAllAgentIdentities(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn(() => () => {}),
}));
vi.mock("@/app/view/agent/components/AgentPicker", () => ({
    useAgentDefinitions: () => () => [
        { id: "agent-1", name: "Agent One", provider: "claude" },
        { id: "agent-2", name: "Agent Two", provider: "claude" },
        { id: "agent-3", name: "Agent Three (codex)", provider: "codex" },
    ],
}));
vi.mock("./identity-model", () => ({
    loadAccounts: () => [
        {
            id: "acc-1",
            name: "work-gh",
            provider: "github",
            kind: "pat",
            display_name: "Work GitHub",
            status: "valid",
        },
        {
            id: "acc-2",
            name: "other-gh",
            provider: "github",
            kind: "pat",
            display_name: "Agent Two's GitHub",
            status: "valid",
        },
        {
            id: "acc-claude-1",
            name: "claude-work",
            provider: "claude",
            kind: "oauth",
            display_name: "Claude Work",
            status: "valid",
        },
    ],
    subscribeAccountChanges: () => () => {},
    PROVIDER_LABELS: { github: "GitHub", claude: "Claude" },
}));

const claudeLoginPanel = vi.fn();
vi.mock("@/app/view/accounts/ClaudeLoginPanel", () => ({
    // Stub: records the props it was opened with instead of driving a real
    // login session (that flow is ClaudeLoginPanel's own concern) — this
    // file only tests that AgentIdentityLinksPanel opens it with the right
    // existingAccountId/linkTarget from the right row.
    ClaudeLoginPanel: (props: any) => {
        claudeLoginPanel(props);
        return <div data-testid="claude-login-panel" />;
    },
}));

import { AgentIdentityLinksPanel } from "./agent-identity-links-panel";

function mkLink(overrides: Partial<AgentDefinitionIdentity>): AgentDefinitionIdentity {
    return {
        agent_id: "agent-1",
        account_id: "acc-1",
        provider: "github",
        ...overrides,
    };
}

describe("AgentIdentityLinksPanel", () => {
    beforeEach(() => {
        listAllAgentIdentities.mockReset();
        claudeLoginPanel.mockReset();
    });

    afterEach(() => {
        cleanup();
    });

    it("renders only the account(s) linked to its own agentId, not another agent's", async () => {
        listAllAgentIdentities.mockResolvedValue([
            mkLink({ agent_id: "agent-1", account_id: "acc-1", provider: "github" }),
            mkLink({ agent_id: "agent-2", account_id: "acc-2", provider: "github" }),
        ]);

        render(() => <AgentIdentityLinksPanel agentId="agent-1" />);

        await waitFor(() => {
            expect(screen.getByText("Work GitHub")).toBeInTheDocument();
        });

        expect(screen.getByText("Agent One")).toBeInTheDocument();
        expect(screen.queryByText("Agent Two's GitHub")).not.toBeInTheDocument();
    });

    it("renders a different agent's account(s) when given a different agentId", async () => {
        listAllAgentIdentities.mockResolvedValue([
            mkLink({ agent_id: "agent-1", account_id: "acc-1", provider: "github" }),
            mkLink({ agent_id: "agent-2", account_id: "acc-2", provider: "github" }),
        ]);

        render(() => <AgentIdentityLinksPanel agentId="agent-2" />);

        await waitFor(() => {
            expect(screen.getByText("Agent Two's GitHub")).toBeInTheDocument();
        });

        expect(screen.getByText("Agent Two")).toBeInTheDocument();
        expect(screen.queryByText("Work GitHub")).not.toBeInTheDocument();
    });

    it("shows a context-free empty state when there is no agentId (no block-level agent context)", async () => {
        listAllAgentIdentities.mockResolvedValue([]);

        render(() => <AgentIdentityLinksPanel agentId={undefined} />);

        expect(screen.getByText(/isn't attached to a specific agent/i)).toBeInTheDocument();
        expect(screen.queryByText("Linked accounts")).not.toBeInTheDocument();
    });

    describe("Claude Connect/Re-login (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.3 surface 3)", () => {
        it("shows a Re-login button on a bound claude row and opens ClaudeLoginPanel with that row's account + this agent as linkTarget", async () => {
            listAllAgentIdentities.mockResolvedValue([
                mkLink({ agent_id: "agent-1", account_id: "acc-claude-1", provider: "claude" }),
            ]);

            render(() => <AgentIdentityLinksPanel agentId="agent-1" />);

            const button = await screen.findByRole("button", { name: "Re-login" });
            expect(claudeLoginPanel).not.toHaveBeenCalled();

            button.click();

            expect(claudeLoginPanel).toHaveBeenCalledWith(
                expect.objectContaining({
                    existingAccountId: "acc-claude-1",
                    linkTarget: { agentDefinitionId: "agent-1" },
                }),
            );
        });

        it("reagent P2 on PR #2414 (round 5): passes the link row's own account id, not the (possibly cache-lagging) joined Account's, so a click while the local accounts cache hasn't caught up still refreshes the RIGHT account instead of minting a new one", async () => {
            // "acc-not-yet-cached" has no matching entry in the mocked
            // loadAccounts() list (acc-1/acc-2/acc-claude-1 only) — this is
            // exactly what a real cache-lag looks like: the link row exists,
            // but subscribeAccountChanges() hasn't delivered the matching
            // Account yet, so joinAgentIdentityRows's `account` field is
            // null even though `accountId` (from the link row itself) is
            // real. The button must still show "Connect" (row.account is
            // null) but must NOT drop the account id on the floor.
            listAllAgentIdentities.mockResolvedValue([
                mkLink({ agent_id: "agent-1", account_id: "acc-not-yet-cached", provider: "claude" }),
            ]);

            render(() => <AgentIdentityLinksPanel agentId="agent-1" />);

            const button = await screen.findByRole("button", { name: "Connect" });
            button.click();

            expect(claudeLoginPanel).toHaveBeenCalledWith(
                expect.objectContaining({ existingAccountId: "acc-not-yet-cached" }),
            );
        });

        it("does NOT show a PER-ROW Connect/Re-login button on a non-claude row (github) — only Claude has an in-app session today", async () => {
            listAllAgentIdentities.mockResolvedValue([
                mkLink({ agent_id: "agent-1", account_id: "acc-1", provider: "github" }),
            ]);

            render(() => <AgentIdentityLinksPanel agentId="agent-1" />);

            await waitFor(() => expect(screen.getByText("Work GitHub")).toBeInTheDocument());
            // The github row itself gets no button (only Claude rows do) —
            // agent-1 is mocked as a claude-provider agent, though, so it
            // DOES still get the standalone Connect affordance below the
            // table (reagent P1 on PR #2414, round 6) since it has no
            // claude/claude-code row of its own yet; that one action is
            // legitimate and covered by the next test.
            const table = screen.getByRole("table");
            expect(within(table).queryByRole("button", { name: /re-login|connect/i })).not.toBeInTheDocument();
        });

        it("reagent P1 on PR #2414 (round 6): STILL offers 'Connect Claude account' when the agent has other-provider links but no claude/claude-code row of its own — the per-row button can't cover this since no claude row exists to attach it to", async () => {
            listAllAgentIdentities.mockResolvedValue([
                mkLink({ agent_id: "agent-1", account_id: "acc-1", provider: "github" }),
            ]);

            render(() => <AgentIdentityLinksPanel agentId="agent-1" />);

            await waitFor(() => expect(screen.getByText("Work GitHub")).toBeInTheDocument());
            const button = await screen.findByRole("button", { name: "Connect Claude account" });
            button.click();

            expect(claudeLoginPanel).toHaveBeenCalledWith(
                expect.objectContaining({
                    existingAccountId: undefined,
                    linkTarget: { agentDefinitionId: "agent-1" },
                }),
            );
        });

        it("reagent P1 on PR #2414 (round 6): does NOT offer 'Connect Claude account' anywhere for a non-claude agent that has other-provider links but no claude row — that would link an unrelated Claude account to an agent whose actual provider is something else entirely", async () => {
            listAllAgentIdentities.mockResolvedValue([
                mkLink({ agent_id: "agent-3", account_id: "acc-1", provider: "github" }),
            ]);

            render(() => <AgentIdentityLinksPanel agentId="agent-3" />);

            await waitFor(() => expect(screen.getByText("Work GitHub")).toBeInTheDocument());
            expect(screen.queryByRole("button", { name: /connect|re-login/i })).not.toBeInTheDocument();
        });

        it("shows Re-login for a legacy-aliased claude link row (\"claude-code\"), not just the canonical \"claude\" provider string (reagent P1 on PR #2414)", async () => {
            listAllAgentIdentities.mockResolvedValue([
                mkLink({ agent_id: "agent-1", account_id: "acc-claude-1", provider: "claude-code" }),
            ]);

            render(() => <AgentIdentityLinksPanel agentId="agent-1" />);

            const button = await screen.findByRole("button", { name: "Re-login" });
            button.click();

            expect(claudeLoginPanel).toHaveBeenCalledWith(
                expect.objectContaining({
                    existingAccountId: "acc-claude-1",
                    // reagent P0 on PR #2414: opening from an aliased row
                    // must tell the panel which alias to clean up on
                    // success — otherwise the login links the canonical
                    // "claude" provider while this orphaned "claude-code"
                    // row lingers and aborts every future spawn.
                    staleAliasProvider: "claude-code",
                }),
            );
        });

        it("does NOT set staleAliasProvider when the row is already canonical \"claude\" — nothing to clean up", async () => {
            listAllAgentIdentities.mockResolvedValue([
                mkLink({ agent_id: "agent-1", account_id: "acc-claude-1", provider: "claude" }),
            ]);

            render(() => <AgentIdentityLinksPanel agentId="agent-1" />);

            const button = await screen.findByRole("button", { name: "Re-login" });
            button.click();

            expect(claudeLoginPanel).toHaveBeenCalledWith(
                expect.objectContaining({ staleAliasProvider: undefined }),
            );
        });

        it("empty state offers 'Connect Claude account' for an agent with NO links at all (the v0.54.9 stuck-instance case — no row exists to attach a per-row button to)", async () => {
            listAllAgentIdentities.mockResolvedValue([]);

            render(() => <AgentIdentityLinksPanel agentId="agent-1" />);

            const button = await screen.findByRole("button", { name: "Connect Claude account" });
            button.click();

            expect(claudeLoginPanel).toHaveBeenCalledWith(
                expect.objectContaining({
                    existingAccountId: undefined,
                    linkTarget: { agentDefinitionId: "agent-1" },
                }),
            );
        });

        it("reagent P2 on PR #2414 (round 3): does NOT offer 'Connect Claude account' for an agent whose own provider isn't claude — that would link an unrelated Claude account instead of the agent's actual provider", async () => {
            listAllAgentIdentities.mockResolvedValue([]);

            render(() => <AgentIdentityLinksPanel agentId="agent-3" />);

            await waitFor(() => {
                expect(screen.getByText(/no linked accounts yet/i)).toBeInTheDocument();
            });
            expect(screen.queryByRole("button", { name: "Connect Claude account" })).not.toBeInTheDocument();
        });

        it("reagent P2 on PR #2414 (round 3): does NOT offer 'Connect Claude account' before the initial load resolves or after it fails — only once genuinely confirmed empty", async () => {
            let resolveLoad!: (v: unknown[]) => void;
            listAllAgentIdentities.mockReturnValue(new Promise((res) => { resolveLoad = res; }));

            render(() => <AgentIdentityLinksPanel agentId="agent-1" />);

            expect(screen.queryByRole("button", { name: "Connect Claude account" })).not.toBeInTheDocument();

            resolveLoad([]);
            const button = await screen.findByRole("button", { name: "Connect Claude account" });
            expect(button).toBeInTheDocument();
        });
    });
});
