// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Render tests for `RecentSessionsList` — the AgentPicker's "Recent
 * sessions" surface (cascade follow-up 2026-05-23). Covers the three
 * UX states the parent relies on:
 *   1. empty (no filter) → generic copy
 *   2. empty (identity filter) → "for this identity" copy
 *   3. populated → row count + preview + provider icon + click handler
 *
 * Plus a unit test for `formatRelative` to pin the "Xm ago" wording.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
    EMPTY_FILTERED,
    EMPTY_GLOBAL,
    formatRelative,
    RecentSessionsList,
} from "./RecentSessionsList";

vi.mock("@/app/store/rpc-api", () => {
    const RpcApi = {
        ListRecentSessionsCommand: vi.fn(),
    };
    return { RpcApi };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

// ProviderLogo just renders something — we don't care what, just that
// rows mount. The component module imports SVG assets via raw imports
// which vitest can't resolve without a transformer.
vi.mock("@/element/ProviderLogo", () => ({
    ProviderLogo: () => null,
}));

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

beforeEach(async () => {
    vi.clearAllMocks();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
});

afterEach(() => {
    cleanup();
});

const makeRow = (overrides: Partial<RecentSessionRow> = {}): RecentSessionRow => ({
    instance_id: "inst-1",
    instance_name: "Maks",
    definition_id: "def-claude",
    definition_name: "Claude Code",
    provider: "claude",
    working_directory: "/tmp/maks",
    identity_id: "id-work",
    identity_name: "Work",
    memory_id: "mem-notes",
    memory_name: "Notes",
    block_id_hint: "blk-1",
    preview: "fix the live-feed hover delay",
    node_count: 12,
    last_active_at: Date.now() - 60_000,
    has_snapshot: true,
    ...overrides,
});

describe("RecentSessionsList — empty state", () => {
    it("renders the generic empty copy when no filter is applied", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue([]);
        render(() => <RecentSessionsList onReattach={() => {}} />);
        const empty = await screen.findByTestId("agent-recent-sessions-empty");
        expect(empty).toHaveTextContent(EMPTY_GLOBAL);
        // No row entries rendered.
        expect(screen.queryByTestId("agent-recent-sessions-entry")).toBeNull();
    });

    it("renders the identity-specific empty copy when filtered", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue([]);
        const [identityId] = createSignal<string>("id-work");
        render(() => (
            <RecentSessionsList
                identityId={identityId}
                onReattach={() => {}}
            />
        ));
        const empty = await screen.findByTestId("agent-recent-sessions-empty");
        expect(empty).toHaveTextContent(EMPTY_FILTERED);
    });
});

describe("RecentSessionsList — populated", () => {
    it("renders one row per session with preview + node count", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue([
            makeRow({ instance_id: "a", instance_name: "Maks", preview: "fix the live-feed" }),
            makeRow({
                instance_id: "b",
                instance_name: "Other",
                preview: "earlier conversation",
                node_count: 1,
            }),
        ]);
        render(() => <RecentSessionsList onReattach={() => {}} />);
        // Wait for resource to settle.
        const entries = await screen.findAllByTestId("agent-recent-sessions-entry");
        expect(entries).toHaveLength(2);

        // Previews render verbatim.
        expect(screen.getByText("fix the live-feed")).toBeInTheDocument();
        expect(screen.getByText("earlier conversation")).toBeInTheDocument();

        // Node-count pluralization (12 messages vs 1 message).
        expect(screen.getByText(/12 messages/)).toBeInTheDocument();
        expect(screen.getByText(/^1 message$/)).toBeInTheDocument();
    });

    it("renders an italic empty-preview hint when has_snapshot but no user message", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue([
            makeRow({ preview: "", has_snapshot: true, node_count: 0 }),
        ]);
        render(() => <RecentSessionsList onReattach={() => {}} />);
        await screen.findByTestId("agent-recent-sessions-entry");
        expect(
            screen.getByText("(no user message yet)"),
        ).toBeInTheDocument();
    });

    it("renders a 'no snapshot' hint when the block has no snapshot file", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue([
            makeRow({ preview: "", has_snapshot: false, node_count: 0 }),
        ]);
        render(() => <RecentSessionsList onReattach={() => {}} />);
        await screen.findByTestId("agent-recent-sessions-entry");
        expect(
            screen.getByText("(no conversation snapshot)"),
        ).toBeInTheDocument();
    });

    it("fires onReattach with the row when an entry is clicked", async () => {
        const onReattach = vi.fn();
        const row = makeRow({ instance_id: "click-me" });
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue([row]);
        render(() => <RecentSessionsList onReattach={onReattach} />);
        const entry = await screen.findByTestId("agent-recent-sessions-entry");
        await userEvent.click(entry);
        expect(onReattach).toHaveBeenCalledTimes(1);
        expect(onReattach.mock.calls[0][0].instance_id).toBe("click-me");
    });

    it("passes identity_id to the RPC when filter is set", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue([]);
        const [identityId] = createSignal<string>("id-work");
        render(() => (
            <RecentSessionsList
                identityId={identityId}
                onReattach={() => {}}
            />
        ));
        await screen.findByTestId("agent-recent-sessions-empty");
        expect(RpcApi.ListRecentSessionsCommand).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ identity_id: "id-work" }),
        );
    });

    it("treats null / undefined identityId as no-filter (empty string sent)", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue([]);
        render(() => <RecentSessionsList onReattach={() => {}} />);
        await screen.findByTestId("agent-recent-sessions-empty");
        expect(RpcApi.ListRecentSessionsCommand).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ identity_id: "" }),
        );
    });
});

describe("formatRelative", () => {
    it("returns 'just now' for sub-minute deltas", () => {
        expect(formatRelative(1_000_000, 1_000_000 - 30_000)).toBe("just now");
    });
    it("returns 'Xm ago' for sub-hour deltas", () => {
        expect(formatRelative(10_000_000, 10_000_000 - 5 * 60_000)).toBe("5m ago");
    });
    it("returns 'Xh ago' for sub-day deltas", () => {
        expect(formatRelative(100_000_000, 100_000_000 - 3 * 3_600_000)).toBe("3h ago");
    });
    it("returns 'Xd ago' for multi-day deltas", () => {
        expect(formatRelative(1_000_000_000, 1_000_000_000 - 2 * 86_400_000)).toBe("2d ago");
    });
    it("returns '' for zero", () => {
        expect(formatRelative(Date.now(), 0)).toBe("");
    });
});
