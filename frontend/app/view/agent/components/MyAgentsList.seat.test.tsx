// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The My Agents tile's seat (SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §4): a
 * role badge after the runtime badge, "reports to … · area" under the
 * account line, and "Seat…" in the row menu, which opens the Seat editor.
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ModalLayerContext, type ModalLayerApi } from "@/app/element/modal-layer";
import type { RecentSessionRow } from "@/types/rpc/RecentSessionRow";

const listRecent = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ListRecentSessionsCommand: (...args: unknown[]) => listRecent(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/element/ProviderLogo", () => ({ ProviderLogo: () => null }));

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

const fakeLayer = (): ModalLayerApi => ({ open: vi.fn(), replace: vi.fn(), close: vi.fn(), current: () => null });

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

    it("opens the Seat editor from the row menu", async () => {
        listRecent.mockResolvedValue({ rows: [row()], degraded: [] });
        const layer = fakeLayer();
        render(() => (
            <ModalLayerContext.Provider value={layer}>
                <MyAgentsList onReattach={() => {}} />
            </ModalLayerContext.Provider>
        ));
        const user = userEvent.setup();
        await user.click(await screen.findByTestId("agent-my-agents-menu-toggle"));
        await user.click(await screen.findByTestId("agent-my-agents-seat"));
        expect(layer.open).toHaveBeenCalledWith({ kind: "agent-seat", agentId: "uid-x", agentName: "AgentX" });
        await waitFor(() => expect(screen.queryByTestId("agent-row-menu")).toBeNull());
    });

    it("opens the Environment editor from the row menu", async () => {
        listRecent.mockResolvedValue({ rows: [row()], degraded: [] });
        const layer = fakeLayer();
        render(() => (
            <ModalLayerContext.Provider value={layer}>
                <MyAgentsList onReattach={() => {}} />
            </ModalLayerContext.Provider>
        ));
        const user = userEvent.setup();
        await user.click(await screen.findByTestId("agent-my-agents-menu-toggle"));
        await user.click(await screen.findByTestId("agent-my-agents-env"));
        expect(layer.open).toHaveBeenCalledWith({ kind: "agent-env", agentId: "uid-x", agentName: "AgentX" });
        await waitFor(() => expect(screen.queryByTestId("agent-row-menu")).toBeNull());
    });

    it("has no Seat item where there is no modal layer to open it in", async () => {
        listRecent.mockResolvedValue({ rows: [row()], degraded: [] });
        render(() => <MyAgentsList onReattach={() => {}} />);
        const user = userEvent.setup();
        await user.click(await screen.findByTestId("agent-my-agents-menu-toggle"));
        await screen.findByTestId("agent-row-menu");
        expect(screen.queryByTestId("agent-my-agents-seat")).toBeNull();
    });
});
