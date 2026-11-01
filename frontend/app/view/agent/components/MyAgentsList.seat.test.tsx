// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The My Agents tile's seat (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4): a
 * role badge after the runtime badge, "reports to … · area" under the
 * account line, and "Seat…" in the row's panel, which opens the Seat editor in it.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";

const listRecent = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ListRecentSessionsCommand: (...args: unknown[]) => listRecent(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/element/ProviderLogo", () => ({ ProviderLogo: () => null }));

vi.mock("./AgentSeatPanel", () => ({
    AgentSeatPanel: (p: { agentName: string; onClose?: () => void }) => (
        <div data-testid="seat-editor">
            {p.agentName}
            <button data-testid="seat-editor-close" onClick={() => p.onClose?.()} />
        </div>
    ),
}));
vi.mock("./AgentBundlesTab", () => ({ AgentBundlesTab: () => <div data-testid="bundles-editor" /> }));
vi.mock("./AgentEnvPanel", () => ({ AgentEnvPanel: () => <div data-testid="env-editor" /> }));

import { MyAgentsList } from "./MyAgentsList";

const row = (overrides: Partial<RecentSessionRow> = {}): RecentSessionRow => ({
    instance_id: "inst-1",
    instance_name: "AgentX",
    definition_id: "uid-x",
    definition_name: "AgentX",
    provider: "claude",
    working_directory: "/tmp/x",
    identity_id: "id-work",
    identity_name: "Work",
    memory_id: "",
    memory_name: "",
    block_id_hint: "",
    preview: "",
    node_count: 0,
    last_active_at: 0,
    has_snapshot: false,
    agent_created_at: 0,
    started_at: 0,
    model_vendor_base_url: "",
    session_id: "",
    snapshot_check_failed: false,
    agent_type: "host",
    ...overrides,
});

describe("MyAgentsList — seats", () => {
    beforeEach(() => listRecent.mockReset());
    afterEach(() => cleanup());

    it("shows the role badge after the runtime badge, and the reports-to line under the account", async () => {
        listRecent.mockResolvedValue({
            rows: [
                row({
                    seat: {
                        role: "worker",
                        qualifier: null,
                        lead: { id: "uid-c", name: "Camper" },
                        area: "panes, layout, tabs",
                    },
                }),
            ],
            degraded: [],
        });
        const { container } = render(() => <MyAgentsList onReattach={() => {}} />);
        const line = await screen.findByTestId("agent-my-agents-seat-line");
        expect(line).toHaveTextContent("reports to Camper · panes, layout, tabs");
        const line1 = container.querySelector(".agent-recent-sessions-line1")!;
        const badge = line1.querySelector(".agent-seat-badge")!;
        expect(badge).toHaveTextContent("Worker");
        // After the runtime badge, on the name's line.
        const children = [...line1.children];
        expect(children.indexOf(badge)).toBeGreaterThan(children.findIndex((c) => /NATIVE/i.test(c.textContent ?? "")));
        // Under the account line.
        const account = container.querySelector(".agent-recent-sessions-account")!;
        expect(account.compareDocumentPosition(line) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it("names the operator as a leader's lead, with the qualifier on the badge", async () => {
        listRecent.mockResolvedValue({
            rows: [row({ seat: { role: "leader", qualifier: "product", lead: null, area: null } })],
            degraded: [],
        });
        const { container } = render(() => <MyAgentsList onReattach={() => {}} />);
        expect(await screen.findByTestId("agent-my-agents-seat-line")).toHaveTextContent("reports to the operator");
        expect(container.querySelector(".agent-seat-badge")).toHaveTextContent("Leader · product");
    });

    it("shows nothing for an agent with no seat", async () => {
        listRecent.mockResolvedValue({ rows: [row()], degraded: [] });
        const { container } = render(() => <MyAgentsList onReattach={() => {}} />);
        await screen.findByText("AgentX");
        expect(container.querySelector(".agent-seat-badge")).toBeNull();
        expect(screen.queryByTestId("agent-my-agents-seat-line")).toBeNull();
    });

    // The three editors are the panel's own views now (they were modals); their bodies are the same
    // components, which load their own data, so they are stood in for here.
    const openActions = async () => {
        listRecent.mockResolvedValue({ rows: [row()], degraded: [] });
        render(() => <MyAgentsList onReattach={() => {}} />);
        const user = userEvent.setup();
        await user.click(await screen.findByTestId("agent-my-agents-menu-toggle"));
        await screen.findByTestId("agent-row-actions");
        return user;
    };

    it("opens the Seat editor in the row's panel, and Close goes back to the actions", async () => {
        const user = await openActions();
        await user.click(screen.getByTestId("agent-my-agents-seat"));
        const body = await screen.findByTestId("seat-editor");
        expect(body.textContent).toBe("AgentX");
        const panel = screen.getByTestId("agent-row-panel");
        expect(panel.dataset.view).toBe("seat");
        expect(panel.contains(body)).toBe(true);
        expect(screen.queryByTestId("agent-row-actions")).toBeNull();
        await user.click(screen.getByTestId("seat-editor-close"));
        expect(screen.getByTestId("agent-row-panel").dataset.view).toBe("actions");
    });

    it("opens the Bundles editor in the panel, beside Seat in the actions", async () => {
        const user = await openActions();
        const bundles = screen.getByTestId("agent-my-agents-bundles");
        expect(screen.getByTestId("agent-my-agents-seat").nextElementSibling).toBe(bundles);
        await user.click(bundles);
        expect(await screen.findByTestId("bundles-editor")).toBeInTheDocument();
        expect(screen.getByTestId("agent-row-panel").dataset.view).toBe("bundles");
    });

    it("opens the Environment editor in the panel, and Back returns to the actions", async () => {
        const user = await openActions();
        await user.click(screen.getByTestId("agent-my-agents-env"));
        expect(await screen.findByTestId("env-editor")).toBeInTheDocument();
        expect(screen.getByTestId("agent-row-panel").dataset.view).toBe("env");
        await user.click(screen.getByTestId("agent-row-panel-back"));
        expect(screen.getByTestId("agent-row-panel").dataset.view).toBe("actions");
    });

    it("needs no modal layer: the editors are in the panel", async () => {
        const user = await openActions();
        expect(screen.getByTestId("agent-my-agents-seat")).toBeInTheDocument();
        await user.click(screen.getByTestId("agent-my-agents-seat"));
        expect(await screen.findByTestId("seat-editor")).toBeInTheDocument();
    });
});
