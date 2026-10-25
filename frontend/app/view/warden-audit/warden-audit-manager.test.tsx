// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression test for reagent P1 / codex P2 on PR #3196: a fleet-action
 * audit entry's caller-supplied `reason` (e.g. ClosePane's cross-pane
 * `reason` field) never rendered, because the reason `<Show>` required
 * `entry.outcome != null`. Fleet actions always have `outcome: undefined`, so
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

    it("shows a delivery's size and status whatever its outcome", async () => {
        mockFetchWardenAudit.mockResolvedValue([
            entry({
                success: true,
                outcome: "held_delivered",
                message_length: 42,
            }),
        ]);
        render(() => <WardenAuditManager />);

        await waitFor(() => {
            expect(screen.getByText("42b")).toBeInTheDocument();
            expect(screen.getByText("ok")).toBeInTheDocument();
        });
    });

    it("shows a registration as a registration, not a blank delivery", async () => {
        mockFetchWardenAudit.mockResolvedValue([
            entry({
                event_kind: "register",
                target_agent: "loap2",
                block_id: "abcdef12-0000",
                evicted_block: "99887766-1111",
                evicted_agent: "someone",
                message_length: 0,
            }),
            entry({ event_kind: "unregister", target_agent: "agent5", block_id: "12345678-0000", message_length: 0 }),
            entry({ event_kind: "swept", target_agent: "agent6", block_id: "55667788-0000", message_length: 0 }),
        ]);
        render(() => <WardenAuditManager />);

        const registered = (await screen.findByText("loap2")).closest("li")!;
        expect(registered).toHaveTextContent("loap2 registered");
        expect(registered).toHaveTextContent("block abcdef12, replaced block 99887766, block was someone");
        expect(registered).not.toHaveTextContent("0b");
        const unregistered = screen.getByText("agent5").closest("li")!;
        expect(unregistered).toHaveTextContent("agent5 unregistered");
        expect(unregistered).toHaveTextContent("block 12345678");
        const swept = screen.getByText("agent6").closest("li")!;
        expect(swept).toHaveTextContent("agent6 swept");
        expect(swept).not.toHaveTextContent("0b");
    });
});
