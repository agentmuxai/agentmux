// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The status bar's cloud indicator: quiet, amber after two minutes, red with a sign-in. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const connect = vi.fn(async () => {});
const refresh = vi.fn(async () => {});
const [loading, setLoading] = createSignal(false);
vi.mock("@/app/view/accounts/AgentMuxConnectPanel", () => ({
    useMuxBusStatus: () => ({ connect, refresh, loading }),
}));

import { acceptMuxbusDelivery, RECONNECTING_QUIET_MS } from "@/store/muxbus-delivery";
import { MuxBusIndicator } from "./MuxBusIndicator";

describe("MuxBusIndicator", () => {
    beforeEach(() => {
        connect.mockClear();
        refresh.mockClear();
        setLoading(false);
    });
    afterEach(() => cleanup());

    it("shows nothing while connected or signed out", () => {
        acceptMuxbusDelivery({ state: "connected", since_ms: Date.now() });
        const { container } = render(() => <MuxBusIndicator />);
        expect(container.textContent).toBe("");
        acceptMuxbusDelivery({ state: "signed_out", since_ms: Date.now() });
        expect(container.textContent).toBe("");
    });

    it("stays quiet through a short reconnect, then shows amber", () => {
        acceptMuxbusDelivery({ state: "reconnecting", since_ms: Date.now() - 10_000 });
        const { container } = render(() => <MuxBusIndicator />);
        expect(container.textContent).toBe("");
        acceptMuxbusDelivery({ state: "reconnecting", since_ms: Date.now() - RECONNECTING_QUIET_MS - 1_000 });
        const item = screen.getByRole("status");
        expect(item.textContent).toContain("MuxBus reconnecting…");
        expect(item.classList.contains("status-muxbus--warn")).toBe(true);
        fireEvent.click(item);
        expect(connect).not.toHaveBeenCalled();
    });

    it("is red when the sign-in stopped working, and clicking starts the sign-in", () => {
        acceptMuxbusDelivery({ state: "needs_sign_in", since_ms: Date.now() });
        render(() => <MuxBusIndicator />);
        const item = screen.getByRole("button");
        expect(item.textContent).toContain("MuxBus: sign in");
        expect(item.classList.contains("status-muxbus--error")).toBe(true);
        fireEvent.click(item);
        expect(connect).toHaveBeenCalledTimes(1);
        fireEvent.keyDown(item, { key: "Enter" });
        expect(connect).toHaveBeenCalledTimes(2);
    });

    it("does not start a second sign-in while one is running", () => {
        acceptMuxbusDelivery({ state: "needs_sign_in", since_ms: Date.now() });
        setLoading(true);
        render(() => <MuxBusIndicator />);
        const item = screen.getByRole("button");
        expect(item.textContent).toContain("Signing in…");
        fireEvent.click(item);
        expect(connect).not.toHaveBeenCalled();
    });
});
