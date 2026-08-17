// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Render tests for `MyAgentsList` — the top tier of the two-tier
 * AgentPicker (Phase 1 of SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
 * Formerly RecentSessionsList; tests carry over to lock the row UX
 * after the rename.
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
    EMPTY_FILTERED,
    EMPTY_GLOBAL,
    FETCH_ERROR,
    noMatchText,
    MyAgentsList,
} from "./MyAgentsList";

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

/** Wraps rows in the `listrecentsessions` response envelope
 * (`{ rows, degraded }`) introduced alongside the per-source
 * degradation hardening — see `ListRecentSessionsCommand`'s return
 * type and `session.rs`'s `ListRecentSessionsResult`. */
const okResult = (rows: RecentSessionRow[] = []) => ({ rows, degraded: [] as string[] });

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
    agent_created_at: Date.now() - 7_776_000_000,
    started_at: Date.now() - 3_600_000,
    ...overrides,
});

describe("MyAgentsList — empty state", () => {
    it("renders the generic empty copy when no filter is applied", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult());
        render(() => <MyAgentsList onReattach={() => {}} />);
        const empty = await screen.findByTestId("agent-my-agents-empty");
        expect(empty).toHaveTextContent(EMPTY_GLOBAL);
        // No row entries rendered.
        expect(screen.queryByTestId("agent-my-agents-entry")).toBeNull();
    });

    it("renders the identity-specific empty copy when filtered", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult());
        const [identityId] = createSignal<string>("id-work");
        render(() => (
            <MyAgentsList
                identityId={identityId}
                onReattach={() => {}}
            />
        ));
        const empty = await screen.findByTestId("agent-my-agents-empty");
        expect(empty).toHaveTextContent(EMPTY_FILTERED);
    });
});

describe("MyAgentsList — populated", () => {
    it("renders one row per agent with preview + node count", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([
            makeRow({ instance_id: "a", instance_name: "Maks", preview: "fix the live-feed" }),
            makeRow({
                instance_id: "b",
                instance_name: "Other",
                preview: "earlier conversation",
                node_count: 1,
            }),
        ]));
        render(() => <MyAgentsList onReattach={() => {}} />);
        // Wait for resource to settle.
        const entries = await screen.findAllByTestId("agent-my-agents-entry");
        expect(entries).toHaveLength(2);

        // Previews render verbatim.
        expect(screen.getByText("fix the live-feed")).toBeInTheDocument();
        expect(screen.getByText("earlier conversation")).toBeInTheDocument();

        // Node-count pluralization (12 messages vs 1 message).
        expect(screen.getByText(/12 messages/)).toBeInTheDocument();
        expect(screen.getByText(/^1 message$/)).toBeInTheDocument();
    });

    it("renders an italic empty-preview hint when has_snapshot but no user message", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([
            makeRow({ preview: "", has_snapshot: true, node_count: 0 }),
        ]));
        render(() => <MyAgentsList onReattach={() => {}} />);
        await screen.findByTestId("agent-my-agents-entry");
        expect(
            screen.getByText("(no user message yet)"),
        ).toBeInTheDocument();
    });

    it("renders a 'no snapshot' hint when the block has no snapshot file", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([
            makeRow({ preview: "", has_snapshot: false, node_count: 0 }),
        ]));
        render(() => <MyAgentsList onReattach={() => {}} />);
        await screen.findByTestId("agent-my-agents-entry");
        expect(
            screen.getByText("(no conversation snapshot)"),
        ).toBeInTheDocument();
    });

    it("fires onReattach with the row when an entry is clicked", async () => {
        const onReattach = vi.fn();
        const row = makeRow({ instance_id: "click-me" });
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([row]));
        render(() => <MyAgentsList onReattach={onReattach} />);
        const entry = await screen.findByTestId("agent-my-agents-entry");
        await userEvent.click(entry);
        expect(onReattach).toHaveBeenCalledTimes(1);
        expect(onReattach.mock.calls[0][0].instance_id).toBe("click-me");
    });

    it("passes identity_id to the RPC when filter is set", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult());
        const [identityId] = createSignal<string>("id-work");
        render(() => (
            <MyAgentsList
                identityId={identityId}
                onReattach={() => {}}
            />
        ));
        await screen.findByTestId("agent-my-agents-empty");
        expect(RpcApi.ListRecentSessionsCommand).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ identity_id: "id-work" }),
        );
    });

    it("treats null / undefined identityId as no-filter (empty string sent)", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult());
        render(() => <MyAgentsList onReattach={() => {}} />);
        await screen.findByTestId("agent-my-agents-empty");
        expect(RpcApi.ListRecentSessionsCommand).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ identity_id: "" }),
        );
    });

    it("renders the 'My Agents' section title", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([makeRow()]));
        render(() => <MyAgentsList onReattach={() => {}} />);
        await screen.findByTestId("agent-my-agents-entry");
        expect(screen.getByText("My Agents")).toBeInTheDocument();
    });
});

describe("MyAgentsList — nameFilter (SPEC_AGENT_PICKER_FILTER_SEARCH_2026_08_17.md)", () => {
    it("narrows rows by case-insensitive substring match on instance_name", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([
            makeRow({ instance_id: "a", instance_name: "Maks" }),
            makeRow({ instance_id: "b", instance_name: "Other Agent" }),
        ]));
        const [nameFilter] = createSignal("mak");
        render(() => <MyAgentsList nameFilter={nameFilter} onReattach={() => {}} />);
        const entries = await screen.findAllByTestId("agent-my-agents-entry");
        expect(entries).toHaveLength(1);
        expect(screen.getByText("Maks")).toBeInTheDocument();
        expect(screen.queryByText("Other Agent")).toBeNull();
    });

    it("falls back to definition_name when instance_name doesn't match but definition_name does", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([
            makeRow({ instance_id: "a", instance_name: "", definition_name: "Claude Code" }),
        ]));
        const [nameFilter] = createSignal("claude");
        render(() => <MyAgentsList nameFilter={nameFilter} onReattach={() => {}} />);
        const entries = await screen.findAllByTestId("agent-my-agents-entry");
        expect(entries).toHaveLength(1);
    });

    it("restores the full list when the filter is cleared", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([
            makeRow({ instance_id: "a", instance_name: "Maks" }),
            makeRow({ instance_id: "b", instance_name: "Other Agent" }),
        ]));
        const [nameFilter, setNameFilter] = createSignal("mak");
        render(() => <MyAgentsList nameFilter={nameFilter} onReattach={() => {}} />);
        await screen.findAllByTestId("agent-my-agents-entry");
        setNameFilter("");
        const entries = await screen.findAllByTestId("agent-my-agents-entry");
        expect(entries).toHaveLength(2);
    });

    it("renders a distinct no-match empty state when the filter matches nothing, not the generic/identity empty copy", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([
            makeRow({ instance_id: "a", instance_name: "Maks" }),
        ]));
        const [nameFilter] = createSignal("zzz-no-such-agent");
        render(() => <MyAgentsList nameFilter={nameFilter} onReattach={() => {}} />);
        const noMatch = await screen.findByTestId("agent-my-agents-no-match");
        expect(noMatch).toHaveTextContent(noMatchText("zzz-no-such-agent"));
        expect(screen.queryByTestId("agent-my-agents-empty")).toBeNull();
        expect(screen.queryByText(EMPTY_GLOBAL)).toBeNull();
    });

    it("does not filter when nameFilter is absent (existing callers unaffected)", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([
            makeRow({ instance_id: "a", instance_name: "Maks" }),
            makeRow({ instance_id: "b", instance_name: "Other Agent" }),
        ]));
        render(() => <MyAgentsList onReattach={() => {}} />);
        const entries = await screen.findAllByTestId("agent-my-agents-entry");
        expect(entries).toHaveLength(2);
    });

    it("bumps the fetch limit to 100 once the filter becomes non-empty, and reverts it when cleared", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult());
        const [nameFilter, setNameFilter] = createSignal("");
        render(() => <MyAgentsList nameFilter={nameFilter} onReattach={() => {}} />);
        await screen.findByTestId("agent-my-agents-empty");
        expect(RpcApi.ListRecentSessionsCommand).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({ limit: 20 }),
        );

        vi.mocked(RpcApi.ListRecentSessionsCommand).mockClear();
        setNameFilter("m");
        await waitFor(() =>
            expect(RpcApi.ListRecentSessionsCommand).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ limit: 100 }),
            ),
        );

        vi.mocked(RpcApi.ListRecentSessionsCommand).mockClear();
        setNameFilter("");
        await waitFor(() =>
            expect(RpcApi.ListRecentSessionsCommand).toHaveBeenCalledWith(
                expect.anything(),
                expect.objectContaining({ limit: 20 }),
            ),
        );
    });

    it("does not refetch on every keystroke once already searching (limit stays bumped, same call count)", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue(okResult([
            makeRow({ instance_id: "a", instance_name: "Maks" }),
        ]));
        const [nameFilter, setNameFilter] = createSignal("m");
        render(() => <MyAgentsList nameFilter={nameFilter} onReattach={() => {}} />);
        await screen.findAllByTestId("agent-my-agents-entry");
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockClear();

        setNameFilter("ma");
        setNameFilter("mak");
        setNameFilter("maks");
        await Promise.resolve();
        await Promise.resolve();

        expect(RpcApi.ListRecentSessionsCommand).not.toHaveBeenCalled();
    });
});

describe("MyAgentsList — fetch error (retro-my-agents-fresh-channel-regression-2026-07-27.md)", () => {
    it("renders a distinct error state, not the empty-agents copy, when the RPC rejects", async () => {
        // Suppress the expected console.error from Logger.error inside the
        // component's own catch block — this test is asserting that catch
        // path fires correctly, not treating its logging as a test failure.
        const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockRejectedValue(new Error("backend unreachable"));
        render(() => <MyAgentsList onReattach={() => {}} />);

        const error = await screen.findByTestId("agent-my-agents-error");
        expect(error).toHaveTextContent(FETCH_ERROR);
        // Must NOT also render (or ever have rendered) the "genuinely empty"
        // copy — that ambiguity is the exact bug this hardening fixes.
        expect(screen.queryByTestId("agent-my-agents-empty")).toBeNull();
        expect(screen.queryByText(EMPTY_GLOBAL)).toBeNull();

        consoleErrorSpy.mockRestore();
    });

    it("retries the RPC when the Retry button is clicked, and shows the list on success", async () => {
        const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        vi.mocked(RpcApi.ListRecentSessionsCommand)
            .mockRejectedValueOnce(new Error("backend unreachable"))
            .mockResolvedValueOnce(okResult([makeRow({ instance_id: "recovered" })]));
        render(() => <MyAgentsList onReattach={() => {}} />);

        const retryBtn = await screen.findByTestId("agent-my-agents-retry");
        await userEvent.click(retryBtn);

        const entries = await screen.findAllByTestId("agent-my-agents-entry");
        expect(entries).toHaveLength(1);
        expect(screen.queryByTestId("agent-my-agents-error")).toBeNull();
        expect(RpcApi.ListRecentSessionsCommand).toHaveBeenCalledTimes(2);

        consoleErrorSpy.mockRestore();
    });

    // codex P2 on PR #2327's post-merge re-review: Solid's createResource
    // keeps the previous resolved value ([]) visible while a refetch is in
    // flight (stale-while-revalidate) — `fetchError` is cleared synchronously
    // when Retry starts, so checking `rows() === undefined` for "loading"
    // only catches the very first fetch. A retry would fall straight
    // through to the empty-state branch for its ENTIRE in-flight duration,
    // flashing "No agents yet" and recreating the exact error/empty
    // ambiguity this whole fix exists to remove.
    it("does not flash the empty-agents message while a retry is in flight", async () => {
        const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

        let resolveRetry!: (result: { rows: RecentSessionRow[]; degraded: string[] }) => void;
        const retryPromise = new Promise<{ rows: RecentSessionRow[]; degraded: string[] }>((resolve) => {
            resolveRetry = resolve;
        });
        vi.mocked(RpcApi.ListRecentSessionsCommand)
            .mockRejectedValueOnce(new Error("backend unreachable"))
            .mockImplementationOnce(() => retryPromise);

        render(() => <MyAgentsList onReattach={() => {}} />);

        const retryBtn = await screen.findByTestId("agent-my-agents-retry");
        await userEvent.click(retryBtn);

        // The retry is now in flight (retryPromise hasn't resolved yet) —
        // neither the error banner nor the "genuinely empty" message must
        // show during this window.
        expect(screen.queryByTestId("agent-my-agents-empty")).toBeNull();
        expect(screen.queryByTestId("agent-my-agents-error")).toBeNull();

        resolveRetry(okResult([makeRow({ instance_id: "recovered" })]));
        const entries = await screen.findAllByTestId("agent-my-agents-entry");
        expect(entries).toHaveLength(1);

        consoleErrorSpy.mockRestore();
    });

    // reagent P2 on PR #2328's re-review: switching `isLoading` to
    // `rows.loading` (to fix the retry-flash bug above) is true for ANY
    // in-flight fetch, including background refetches the old
    // `rows() === undefined` check never touched (visibility regain,
    // agents:changed events). The count badge's guard must not use the
    // same `isLoading`, or it flickers away on every background refetch
    // instead of just the very first load.
    it("keeps the count badge visible during a background refetch, not just the initial load", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand)
            .mockResolvedValueOnce(okResult([makeRow({ instance_id: "a", node_count: 3 })]))
            // Never resolves — simulates a still-in-flight background
            // refetch (e.g. an identity switch, or an agents:changed
            // re-poll) so the count badge's behavior DURING that window
            // is directly observable.
            .mockImplementationOnce(() => new Promise(() => {}));

        const [identityId, setIdentityId] = createSignal<string>("id-1");
        render(() => (
            <MyAgentsList identityId={identityId} onReattach={() => {}} />
        ));

        await screen.findAllByTestId("agent-my-agents-entry");
        expect(screen.getByTestId("agent-my-agents-count")).toHaveTextContent("1");

        // Trigger a refetch while the previous row is still showing.
        setIdentityId("id-2");
        await Promise.resolve();

        // The stale row + its count badge must still be visible while the
        // new (deliberately never-resolving, for this test) fetch is
        // in flight — this is exactly what flickered away under the bug.
        expect(screen.getByTestId("agent-my-agents-entry")).toBeInTheDocument();
        expect(screen.getByTestId("agent-my-agents-count")).toHaveTextContent("1");
    });

    // reagent P1 on PR #2327: session.rs's hardening (each of 6 data
    // sources degrades to empty on its own failure instead of aborting
    // the whole RPC) means the RPC itself basically never throws anymore
    // — so a resolved call with zero rows AND a non-empty `degraded` is
    // now the ONLY signal that something failed rather than "you
    // genuinely have no agents." Without checking `degraded`, this exact
    // scenario would silently render EMPTY_GLOBAL — the very ambiguity
    // this whole fix exists to close, just one layer deeper than the
    // reject-based path the two tests above cover.
    it("renders the error state (not empty) when the RPC resolves with zero rows but reports a degraded source", async () => {
        const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue({
            rows: [],
            degraded: ["registry", "local_instances"],
        });
        render(() => <MyAgentsList onReattach={() => {}} />);

        const error = await screen.findByTestId("agent-my-agents-error");
        expect(error).toHaveTextContent(FETCH_ERROR);
        expect(screen.queryByTestId("agent-my-agents-empty")).toBeNull();
        expect(screen.queryByText(EMPTY_GLOBAL)).toBeNull();

        consoleErrorSpy.mockRestore();
    });

    it("still renders real rows when a source degrades but rows are non-empty — partial degradation is not an error state", async () => {
        vi.mocked(RpcApi.ListRecentSessionsCommand).mockResolvedValue({
            rows: [makeRow({ instance_id: "still-here" })],
            // e.g. identity_list failed — rows still come back with
            // fallback display text, per session.rs's own design intent.
            degraded: ["accounts"],
        });
        render(() => <MyAgentsList onReattach={() => {}} />);

        const entries = await screen.findAllByTestId("agent-my-agents-entry");
        expect(entries).toHaveLength(1);
        expect(screen.queryByTestId("agent-my-agents-error")).toBeNull();
    });

    // reagent P1 on PR #2327's re-review: `fetchError` was a single shared
    // signal with no per-request guard — a stale (superseded) fetch's late
    // rejection could overwrite the state a NEWER, already-succeeded fetch
    // just set, painting the error panel over valid loaded data.
    it("does not let a stale fetch's late rejection overwrite a newer fetch's successful data", async () => {
        const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

        let rejectStale!: (err: unknown) => void;
        const stalePromise = new Promise<never>((_, reject) => { rejectStale = reject; });
        vi.mocked(RpcApi.ListRecentSessionsCommand)
            .mockImplementationOnce(() => stalePromise)
            .mockResolvedValueOnce(okResult([makeRow({ instance_id: "fresh" })]));

        const [identityId, setIdentityId] = createSignal<string>("id-1");
        render(() => (
            <MyAgentsList identityId={identityId} onReattach={() => {}} />
        ));

        // Supersede the still-pending first fetch before it ever resolves.
        setIdentityId("id-2");
        const entries = await screen.findAllByTestId("agent-my-agents-entry");
        expect(entries).toHaveLength(1);

        // Now let the stale first fetch fail, late.
        rejectStale(new Error("stale failure"));
        await Promise.resolve();
        await Promise.resolve();

        // The already-loaded, correct data must still be showing — not the
        // error panel from a fetch that no longer matters.
        expect(screen.queryByTestId("agent-my-agents-error")).toBeNull();
        expect(screen.getAllByTestId("agent-my-agents-entry")).toHaveLength(1);

        consoleErrorSpy.mockRestore();
    });
});

// formatRelative was migrated to frontend/util/format-time.ts's formatTimeAgo
// (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md §2.2) — its tests moved with
// it, see format-time.test.ts.
