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
import type { AgentErrorNode, DocumentNode, DocumentState } from "../types";

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
