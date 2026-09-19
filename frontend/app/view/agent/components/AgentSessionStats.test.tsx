// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSessionStats — the context reading as a button, and the popover it
 * opens (SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §3.1).
 *
 * The behaviors worth guarding are the ones the old drawer rows got for free
 * and this popover does not: it must not close when the user clicks one of
 * its own action buttons, and it must not claim a $0 cost for providers that
 * report no cost at all.
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi, beforeEach } from "vitest";

import { AgentSessionStats } from "./AgentSessionStats";

const archiveSession = vi.hoisted(() => vi.fn(async () => {}));
const exportSession = vi.hoisted(() => vi.fn(async () => {}));
const restoreSession = vi.hoisted(() => vi.fn(async () => {}));

vi.mock("../session-actions", async (importOriginal) => {
    const actual = await importOriginal<typeof import("../session-actions")>();
    return { ...actual, archiveSession, exportSession, restoreSession };
});

const block = (meta: Record<string, unknown>) => () => ({ meta }) as never;

const baseProps = {
    blockId: "block-1",
    blockAtom: block({ "session:line_count": 12_483 }),
    providerId: "claude",
    ctxClass: "",
    label: "104k / 200k",
};

const openPanel = async () => {
    await userEvent.click(screen.getByRole("button", { name: /104k \/ 200k/ }));
    return await screen.findByRole("dialog", { name: /session stats/i });
};

describe("AgentSessionStats", () => {
    // This suite queries `screen` (document-wide) and the panel renders through
    // a Portal into body — without an explicit unmount, the previous test's
    // trigger and panel are still in the DOM and every query matches twice.
    afterEach(cleanup);

    beforeEach(() => {
        archiveSession.mockClear();
        exportSession.mockClear();
        restoreSession.mockClear();
    });

    it("renders the reading as a button and opens the popover on click", async () => {
        render(() => <AgentSessionStats {...baseProps} contextTokens={104_000} contextWindow={200_000} />);
        expect(screen.queryByRole("dialog")).toBeNull();
        const panel = await openPanel();
        expect(panel).toHaveTextContent("104k / 200k (52%)");
    });

    it("shows cumulative cost, turns and token totals with the cache share", async () => {
        render(() => (
            <AgentSessionStats
                {...baseProps}
                contextTokens={104_000}
                contextWindow={200_000}
                sessionTotals={{
                    cost_usd: 2.41,
                    num_turns: 37,
                    duration_ms: 18 * 60_000,
                    input_tokens: 1_000_000,
                    output_tokens: 46_000,
                    cache_read_input_tokens: 890_000,
                }}
            />
        ));
        const panel = await openPanel();
        expect(panel).toHaveTextContent("$2.41");
        expect(panel).toHaveTextContent("37 turns");
        expect(panel).toHaveTextContent("18m");
        expect(panel).toHaveTextContent("89% cached");
        expect(panel).toHaveTextContent("46k out");
    });

    it("renders an em dash, not $0, when the provider reports no cost", async () => {
        render(() => (
            <AgentSessionStats
                {...baseProps}
                providerId="codex"
                contextTokens={104_000}
                contextWindow={200_000}
                sessionTotals={{ num_turns: 4, input_tokens: 1_000, output_tokens: 500 }}
            />
        ));
        const panel = await openPanel();
        expect(panel).toHaveTextContent("—");
        expect(panel).not.toHaveTextContent("$0");
    });

    it("stays open when an action button inside it is clicked", async () => {
        render(() => <AgentSessionStats {...baseProps} contextTokens={104_000} contextWindow={200_000} />);
        await openPanel();
        await userEvent.click(screen.getByRole("button", { name: /Archive/ }));
        await waitFor(() => expect(archiveSession).toHaveBeenCalledWith("block-1"));
        expect(screen.queryByRole("dialog", { name: /session stats/i })).not.toBeNull();
    });

    it("offers Restore instead of Archive once the session is archived", async () => {
        render(() => (
            <AgentSessionStats
                {...baseProps}
                blockAtom={block({ "session:line_count": 10, "session:archived_at": 1_700_000_000_000 })}
                contextTokens={104_000}
                contextWindow={200_000}
            />
        ));
        await openPanel();
        expect(screen.queryByRole("button", { name: /Archive/ })).toBeNull();
        expect(screen.getByRole("button", { name: /Restore/ })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /Export/ })).toBeInTheDocument();
    });

    it("hides session management for non-Claude providers", async () => {
        render(() => (
            <AgentSessionStats {...baseProps} providerId="gemini" contextTokens={104_000} contextWindow={200_000} />
        ));
        const panel = await openPanel();
        // The stats themselves still render — only the actions are gated.
        expect(panel).toHaveTextContent("104k / 200k");
        expect(screen.queryByRole("button", { name: /Archive/ })).toBeNull();
        expect(screen.queryByRole("button", { name: /Export/ })).toBeNull();
    });

    it("closes on Escape", async () => {
        render(() => <AgentSessionStats {...baseProps} contextTokens={104_000} contextWindow={200_000} />);
        await openPanel();
        await userEvent.keyboard("{Escape}");
        await waitFor(() => expect(screen.queryByRole("dialog", { name: /session stats/i })).toBeNull());
    });
});
