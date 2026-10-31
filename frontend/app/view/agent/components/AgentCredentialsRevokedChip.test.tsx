// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The credentials-revoked row says the agent still holds the old tokens until restarted, so it
 * goes away when the agent's launch flow runs again (a rebind restarts it), and not before.
 */
import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

let revoke: ((data: { provider: string }) => void) | undefined;
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_event: string, cb: (data: { provider: string }) => void) => {
        revoke = cb;
        return () => {};
    },
}));

import { AgentCredentialsRevokedChip } from "./AgentCredentialsRevokedChip";

const ROW = /Credentials revoked/;

describe("AgentCredentialsRevokedChip", () => {
    afterEach(() => {
        cleanup();
        revoke = undefined;
    });

    it("shows on a revoke and clears when the agent's launch flow starts again", async () => {
        const [running, setRunning] = createSignal(false);
        render(() => <AgentCredentialsRevokedChip agentId="a1" flowRunning={running} />);
        expect(screen.queryByText(ROW)).toBeNull();
        revoke!({ provider: "claude" });
        expect(await screen.findByText(ROW)).toBeTruthy();
        setRunning(true);
        await Promise.resolve();
        expect(screen.queryByText(ROW)).toBeNull();
    });

    it("stays while the flow was already running, and across a flow that ends", async () => {
        const [running, setRunning] = createSignal(true);
        render(() => <AgentCredentialsRevokedChip agentId="a1" flowRunning={running} />);
        revoke!({ provider: "claude" });
        expect(await screen.findByText(ROW)).toBeTruthy();
        setRunning(false);
        await Promise.resolve();
        expect(screen.queryByText(ROW)).toBeTruthy();
    });
});
