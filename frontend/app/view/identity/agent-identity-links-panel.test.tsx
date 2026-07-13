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

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
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
        { id: "agent-1", name: "Agent One" },
        { id: "agent-2", name: "Agent Two" },
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
    ],
    subscribeAccountChanges: () => () => {},
    PROVIDER_LABELS: { github: "GitHub", claude: "Claude" },
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
});
