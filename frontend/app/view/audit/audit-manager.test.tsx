// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AuditEntry } from "@/types/rpc/AuditEntry";
import type { AuditQueryReq } from "@/types/rpc/AuditQueryReq";
import type { AuditQueryResult } from "@/types/rpc/AuditQueryResult";

const query = vi.fn<(req: AuditQueryReq) => Promise<AuditQueryResult>>();
const archives = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AuditQueryCommand: (_c: unknown, req: AuditQueryReq) => query(req),
        HistoryAgentsCommand: () => Promise.resolve({ agents: [{ definition_id: "uid-korp", provider: "codex" }] }),
        AgentSessionListArchivesCommand: () => archives(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/vault/vault-secrets", () => ({ VaultSecrets: () => null }));

const revealBlock = vi.fn();
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: (...a: unknown[]) => revealBlock(...a) }));
const openOrFocusHistoryTab = vi.fn();
vi.mock("@/app/view/agent/open-history-tab", () => ({ openOrFocusHistoryTab: (...a: unknown[]) => openOrFocusHistoryTab(...a) }));

import { AuditManager } from "./audit-manager";

const flush = () => new Promise((r) => setTimeout(r, 0));

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

function entry(over: Partial<AuditEntry>): AuditEntry {
    return { id: "e1", ts_ms: Date.now(), kind: "jekt", summary: "korp → loap: delivered, 40 bytes", links: {}, ...over };
}

const page = (entries: AuditEntry[], over: Partial<AuditQueryResult> = {}): AuditQueryResult => ({ entries, not_recorded: [], ...over });

describe("AuditManager", () => {
    it("lists what audit.query answers with the totals, a fleet action's reason included", async () => {
        query.mockResolvedValue(
            page([entry({ kind: "action", summary: "agent5 close_pane on loap", reason: "closing a pane stuck in a broken render state" })], {
                totals: { entries: 1, input: 10, cache_read: 0, output: 0, estimated: true },
            })
        );
        render(() => <AuditManager paneBlockId="pane-1" />);

        await waitFor(() => expect(screen.getByText("agent5 close_pane on loap")).toBeInTheDocument());
        expect(screen.getByText(/closing a pane stuck in a broken render state/)).toBeInTheDocument();
        expect(screen.getByText("1 entry · ~10 tokens")).toBeInTheDocument();
        expect(query.mock.calls[0][0]).toMatchObject({ limit: 50, totals: true });
    });

    it("asks again with the filter when a kind is ticked, and says what is not recorded", async () => {
        query.mockResolvedValue(page([entry({})]));
        render(() => <AuditManager paneBlockId="pane-1" />);
        await waitFor(() => expect(screen.getByText(/korp → loap/)).toBeInTheDocument());

        query.mockResolvedValue(page([entry({})], { not_recorded: ["tool"] }));
        fireEvent.click(screen.getByRole("button", { name: "Tool calls" }));

        await waitFor(() => expect(screen.getByText("Not recorded yet: tool calls.")).toBeInTheDocument());
        expect(query.mock.calls.at(-1)![0].kinds).toContain("tool");
    });

    it("starts filtered to the agent the pane names", async () => {
        query.mockResolvedValue(page([]));
        render(() => <AuditManager paneBlockId="pane-1" agent="korp" />);
        await waitFor(() => expect(query).toHaveBeenCalled());
        expect(query.mock.calls[0][0].agent_id).toBe("korp");
    });

    it("pages with the cursor and adds the older entries below, without repeating one", async () => {
        query.mockResolvedValueOnce(page([entry({ id: "a", summary: "first one" })], { next_cursor: "c1" }));
        render(() => <AuditManager paneBlockId="pane-1" />);
        await waitFor(() => expect(screen.getByText("first one")).toBeInTheDocument());

        query.mockResolvedValueOnce(page([entry({ id: "a", summary: "first one" }), entry({ id: "b", summary: "second one" })]));
        fireEvent.click(screen.getByRole("button", { name: /Show older/ }));

        await waitFor(() => expect(screen.getByText("second one")).toBeInTheDocument());
        expect(screen.getAllByText("first one")).toHaveLength(1);
        expect(query.mock.calls.at(-1)![0].cursor).toBe("c1");
    });

    it("opens the sender's history at the entry's time and reveals the receiver's pane", async () => {
        query.mockResolvedValue(
            page([
                entry({
                    from: { agent_id: "korp", links: { history: { agent_id: "korp", definition_id: "uid-korp", at_ms: 1234 } } },
                    to: { agent_id: "loap", links: { pane: { block_id: "blk-loap" } } },
                }),
            ])
        );
        render(() => <AuditManager paneBlockId="pane-1" />);
        await waitFor(() => expect(screen.getByRole("button", { name: "Sender's history" })).toBeInTheDocument());

        // The agent's provider comes from the history list, and the time falls before this archive.
        archives.mockResolvedValue([{ archive_zoneid: "agent:uid-korp:archive:5000", archived_at_ms: 5000 }]);
        fireEvent.click(screen.getByRole("button", { name: "Sender's history" }));
        await waitFor(() => expect(openOrFocusHistoryTab).toHaveBeenCalled());
        expect(openOrFocusHistoryTab).toHaveBeenCalledWith(
            expect.objectContaining({
                currentBlockId: "pane-1",
                agentId: "uid-korp",
                provider: "codex",
                at: expect.objectContaining({ at_ms: 1234, zone: "agent:uid-korp:archive:5000" }),
            })
        );

        fireEvent.click(screen.getByRole("button", { name: "Receiver's pane" }));
        expect(revealBlock).toHaveBeenCalledWith("blk-loap", { source: "audit" });
    });

    it("reloads a filter changed while the pane was hidden once it is shown, instead of paging the old list", async () => {
        const [visible, setVisible] = createSignal(true);
        const [agent, setAgent] = createSignal<string | undefined>(undefined);
        query.mockResolvedValue(page([entry({ summary: "old agent's" })], { next_cursor: "old" }));
        render(() => <AuditManager paneBlockId="pane-1" visible={visible()} agent={agent()} />);
        await waitFor(() => expect(screen.getByText("old agent's")).toBeInTheDocument());

        setVisible(false);
        await flush();
        query.mockResolvedValue(page([entry({ summary: "new agent's" })]));
        setAgent("loap");
        await flush();
        expect(screen.queryByRole("button", { name: /Show older/ })).toBeNull();

        setVisible(true);
        await waitFor(() => expect(screen.getByText("new agent's")).toBeInTheDocument());
        expect(query.mock.calls.at(-1)![0]).toMatchObject({ agent_id: "loap" });
        expect(query.mock.calls.at(-1)![0].cursor).toBeUndefined();
    });

    it("shows an error from the query", async () => {
        query.mockRejectedValue(new Error("audit.query: unknown kind"));
        render(() => <AuditManager paneBlockId="pane-1" />);
        await waitFor(() => expect(screen.getByText(/unknown kind/)).toBeInTheDocument());
    });
});
