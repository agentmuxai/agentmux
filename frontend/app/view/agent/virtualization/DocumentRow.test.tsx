// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DocumentRow inline auth-error CTA tests (P2.3 of
 * SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20 §7).
 *
 * An `agent_error` node whose `code` is an auth status (401/403) renders a
 * "Login Again" button that drives the same re-auth flow as the failure
 * banner. Any other code (or code 0 = non-HTTP) renders no button — those
 * errors have no in-place fix.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DocumentRow } from "./DocumentRow";
import type { AgentErrorNode, CompactionStartedNode, ContextCompactedNode, DocumentNode, DocumentState } from "../types";

afterEach(() => cleanup());

const emptyState = (): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    expandedTools: new Set(),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

const errorNode = (code: number, message = "boom"): AgentErrorNode => ({
    type: "agent_error",
    id: "err-1",
    code,
    message,
});

const renderRow = (node: DocumentNode, onAgentErrorLogin?: () => void) => {
    const [n] = createSignal<DocumentNode>(node);
    const [state] = createSignal<DocumentState>(emptyState());
    return render(() => (
        <DocumentRow
            node={n}
            documentState={state}
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
            onAgentErrorLogin={onAgentErrorLogin}
        />
    ));
};

describe("DocumentRow — inline auth-error CTA", () => {
    it("renders a Login Again button for a 401 error and fires onAgentErrorLogin on click", async () => {
        const onLogin = vi.fn();
        renderRow(errorNode(401, "Invalid authentication credentials"), onLogin);

        const btn = screen.getByRole("button", { name: /Login Again/i });
        expect(btn).toBeInTheDocument();
        expect(screen.getByText("HTTP 401")).toBeInTheDocument();

        await userEvent.click(btn);
        expect(onLogin).toHaveBeenCalledTimes(1);
    });

    it("renders the CTA for a 403 error too", () => {
        renderRow(errorNode(403, "Forbidden"), vi.fn());
        expect(screen.getByRole("button", { name: /Login Again/i })).toBeInTheDocument();
    });

    it("renders NO CTA for a non-auth error code (500)", () => {
        renderRow(errorNode(500, "Internal error"), vi.fn());
        expect(screen.queryByRole("button", { name: /Login Again/i })).toBeNull();
        expect(screen.getByText("HTTP 500")).toBeInTheDocument();
    });

    it("renders NO CTA for a non-HTTP error (code 0) and shows 'Error' not 'HTTP 0'", () => {
        renderRow(errorNode(0, "Network connection lost"), vi.fn());
        expect(screen.queryByRole("button", { name: /Login Again/i })).toBeNull();
        expect(screen.getByText("Error")).toBeInTheDocument();
        expect(screen.queryByText(/HTTP 0/)).toBeNull();
    });

    it("renders NO CTA when onAgentErrorLogin is not provided, even for a 401", () => {
        renderRow(errorNode(401), undefined);
        expect(screen.queryByRole("button", { name: /Login Again/i })).toBeNull();
    });
});

/**
 * DocumentRow — compaction nodes (SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md).
 *
 * Two DISTINCT node types render two DISTINCT rows: `compaction_started`
 * (in-progress announcement, no outcome data yet) and `context_compacted`
 * (the completed record — real backend data or the heuristic fallback).
 * They must never be visually confusable — that's the exact bug this
 * split guards against (an in-progress compaction reading as finished).
 */
describe("DocumentRow — compaction nodes", () => {
    const realCompactedNode = (): ContextCompactedNode => ({
        type: "context_compacted",
        id: "cc-1",
        tokensBefore: 100_000,
        tokensAfter: 5_000,
        timestamp: Date.now(),
        source: "real",
        trigger: "manual",
        durationMs: 12_345,
    });

    const heuristicCompactedNode = (): ContextCompactedNode => ({
        type: "context_compacted",
        id: "cc-2",
        tokensBefore: 60_000,
        tokensAfter: 4_000,
        timestamp: Date.now(),
        source: "heuristic",
    });

    const startedNode = (trigger: "manual" | "auto"): CompactionStartedNode => ({
        type: "compaction_started",
        id: "cs-1",
        trigger,
        startedAt: Date.now(),
    });

    it("real context_compacted shows the trigger label and real duration", () => {
        renderRow(realCompactedNode());
        expect(screen.getByText(/context compacted/i)).toBeInTheDocument();
        expect(screen.getByText(/you ran \/compact/i)).toBeInTheDocument();
        expect(screen.getByText(/100k → 5k tokens/i)).toBeInTheDocument();
        expect(screen.getByText(/took 12\.3s/i)).toBeInTheDocument();
    });

    it("real context_compacted with auto trigger shows the auto-compacted label", () => {
        renderRow({ ...realCompactedNode(), trigger: "auto" });
        expect(screen.getByText(/auto-compacted/i)).toBeInTheDocument();
    });

    it("heuristic context_compacted renders WITHOUT a trigger label or duration", () => {
        renderRow(heuristicCompactedNode());
        expect(screen.getByText(/context compacted/i)).toBeInTheDocument();
        expect(screen.queryByText(/you ran \/compact/i)).toBeNull();
        expect(screen.queryByText(/auto-compacted/i)).toBeNull();
        expect(screen.queryByText(/took/i)).toBeNull();
        expect(screen.getByText(/60k → 4k tokens/i)).toBeInTheDocument();
    });

    it("compaction_started renders the in-progress announcement, distinct from context_compacted", () => {
        renderRow(startedNode("manual"));
        expect(screen.getByText(/Compacting conversation/i)).toBeInTheDocument();
        expect(screen.getByText(/you ran \/compact/i)).toBeInTheDocument();
        // Must NOT render anything from the completed-record copy — an
        // in-progress compaction must never look like a finished one.
        expect(screen.queryByText(/context compacted/i)).toBeNull();
    });

    it("compaction_started with auto trigger shows a distinct reason label", () => {
        renderRow(startedNode("auto"));
        expect(screen.getByText(/Compacting conversation/i)).toBeInTheDocument();
        expect(screen.getByText(/context filled up/i)).toBeInTheDocument();
    });
});
