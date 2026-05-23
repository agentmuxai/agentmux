// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Banner-visibility tests for AgentDisconnectedBanner (PR F of the
 * turn-phase migration). The banner must:
 *   - render iff `phase().kind === "Disconnected"`
 *   - surface `lastKind` + a relative age on the message line
 *   - fire `onReconnect` when the Reconnect button is clicked
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §6.4.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentDisconnectedBanner } from "./AgentDisconnectedBanner";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";

afterEach(() => {
    // @solidjs/testing-library doesn't auto-cleanup by default. Each
    // `render` mounts into a fresh container, but the previous
    // container is left in the DOM — `screen.queryBy*` searches the
    // whole document body so multiple renders within one file
    // accumulate. Explicit cleanup keeps tests independent.
    cleanup();
});

describe("AgentDisconnectedBanner", () => {
    it("renders nothing when phase.kind !== 'Disconnected'", () => {
        const [phase] = createSignal<TurnPhase>({ kind: "Idle" });
        render(() => (
            <AgentDisconnectedBanner phase={phase} onReconnect={() => {}} />
        ));
        expect(screen.queryByText(/Disconnected from stream/)).toBeNull();
        expect(screen.queryByRole("button", { name: /Reconnect/ })).toBeNull();
    });

    it("renders the banner + Reconnect button when phase.kind === 'Disconnected'", () => {
        const [phase] = createSignal<TurnPhase>({
            kind: "Disconnected",
            lastKind: "Streaming",
            lastConnectedAt: Date.now(),
            reason: "stream-unsubscribed",
        });
        render(() => (
            <AgentDisconnectedBanner phase={phase} onReconnect={() => {}} />
        ));
        expect(
            screen.getByText(/Disconnected from stream/),
        ).toBeInTheDocument();
        // The detail line mentions the lost kind, lowercased.
        expect(screen.getByText(/was streaming/i)).toBeInTheDocument();
        // Manual reconnect affordance present.
        expect(
            screen.getByRole("button", { name: /Reconnect/ }),
        ).toBeInTheDocument();
    });

    it("invokes onReconnect when the Reconnect button is clicked", async () => {
        const onReconnect = vi.fn();
        const [phase] = createSignal<TurnPhase>({
            kind: "Disconnected",
            lastKind: "Submitting",
            lastConnectedAt: Date.now(),
            reason: "stream-unsubscribed",
        });
        render(() => (
            <AgentDisconnectedBanner phase={phase} onReconnect={onReconnect} />
        ));
        const user = userEvent.setup();
        await user.click(screen.getByRole("button", { name: /Reconnect/ }));
        expect(onReconnect).toHaveBeenCalledTimes(1);
    });

    it("toggles visibility when the phase signal flips", () => {
        const [phase, setPhase] = createSignal<TurnPhase>({ kind: "Idle" });
        render(() => (
            <AgentDisconnectedBanner phase={phase} onReconnect={() => {}} />
        ));
        // Initially absent.
        expect(screen.queryByText(/Disconnected from stream/)).toBeNull();

        // Flip to Disconnected — banner appears.
        setPhase({
            kind: "Disconnected",
            lastKind: "Interrupting",
            lastConnectedAt: Date.now(),
            reason: "stream-unsubscribed",
        });
        expect(
            screen.getByText(/Disconnected from stream/),
        ).toBeInTheDocument();

        // Flip back to Idle (e.g. user pressed Reconnect, dispatcher
        // landed StreamSubscribe → Idle). Banner disappears.
        setPhase({ kind: "Idle" });
        expect(screen.queryByText(/Disconnected from stream/)).toBeNull();
    });
});
