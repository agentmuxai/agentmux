// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression test for reagent P1 / codex P2 on PR #3196: a fleet-action
 * audit entry's caller-supplied `reason` (e.g. ClosePane's cross-pane
 * `reason` field) never rendered, because the reason `<Show>` required
 * `entry.outcome != null` — a condition written for Supervisor-nudge
 * entries specifically. Fleet actions always have `outcome: undefined`, so
 * their reason silently fell through to the error-message branch instead.
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AuditEntry } from "./warden-audit-shared";

const mockFetchWardenAudit = vi.fn<() => Promise<AuditEntry[]>>();

vi.mock("./warden-audit-shared", () => ({
    WARDEN_AUDIT_LIMIT: 50,
    fetchWardenAudit: () => mockFetchWardenAudit(),
}));

import { WardenAuditManager } from "./warden-audit-manager";

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

function entry(overrides: Partial<AuditEntry>): AuditEntry {
    return {
        timestamp: Date.now(),
        target_agent: "loap2",
        block_id: "block-1",
        message_hash: "abc123",
        message_length: 10,
        success: true,
        request_id: "req-1",
        ...overrides,
    };
}

describe("WardenAuditManager — fleet-action reason display", () => {
    it("shows a successful fleet action's reason even though outcome is unset", async () => {
        mockFetchWardenAudit.mockResolvedValue([
            entry({
                source_agent: "agent5",
                target_agent: "loap2",
                success: true,
                reason: "closing a pane stuck in a broken render state",
            }),
        ]);
        render(() => <WardenAuditManager />);

        await waitFor(() => {
            expect(screen.getByText("closing a pane stuck in a broken render state")).toBeInTheDocument();
        });
    });

    it("still shows error_message, not reason, on a failed entry with no reason", async () => {
        mockFetchWardenAudit.mockResolvedValue([
            entry({
                success: false,
                error_message: "block not found",
            }),
        ]);
        render(() => <WardenAuditManager />);

        await waitFor(() => {
            expect(screen.getByText("block not found")).toBeInTheDocument();
        });
    });

    it("still prioritizes outcome-gated Supervisor rendering when outcome IS set", async () => {
        mockFetchWardenAudit.mockResolvedValue([
            entry({
                success: true,
                outcome: "nudge_sent",
                reason: "target looked stalled",
            }),
        ]);
        render(() => <WardenAuditManager />);

        await waitFor(() => {
            expect(screen.getByText("target looked stalled")).toBeInTheDocument();
            expect(screen.getByText("nudged")).toBeInTheDocument();
        });
    });
});
