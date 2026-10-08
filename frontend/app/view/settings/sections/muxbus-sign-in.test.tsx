// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › MuxBus: the status bar's state as a line, with Sign in or Sign out. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const connect = vi.fn(async () => {});
const disconnect = vi.fn(async () => {});
const cancel = vi.fn(async () => {});
const refresh = vi.fn(async () => {});
const [loading, setLoading] = createSignal(false);
vi.mock("@/app/view/accounts/AgentMuxConnectPanel", () => ({
    useMuxBusStatus: () => ({
        connect,
        disconnect,
        cancel,
        refresh,
        loading,
        error: () => null,
        isConfigured: () => true,
    }),
}));

import { acceptMuxbusDelivery, muxbusDeliveryLine } from "@/store/muxbus-delivery";
import { MuxBusSignIn } from "./muxbus-sign-in";

describe("MuxBusSignIn", () => {
    beforeEach(() => {
        for (const f of [connect, disconnect, cancel, refresh]) f.mockClear();
        setLoading(false);
    });
    afterEach(() => cleanup());

    it("reads the status on open", () => {
        render(() => <MuxBusSignIn />);
        expect(refresh).toHaveBeenCalledTimes(1);
    });

    it("shows a lost sign-in with Sign in", () => {
        const status = { state: "needs_sign_in" as const, since_ms: Date.now(), account_email: "a@b.c" };
        acceptMuxbusDelivery(status);
        render(() => <MuxBusSignIn />);
        expect(screen.getByRole("status").textContent).toBe(muxbusDeliveryLine(status));
        fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
        expect(connect).toHaveBeenCalledTimes(1);
    });

    it("shows a working sign-in with Sign out, reconnecting included", () => {
        acceptMuxbusDelivery({ state: "connected", since_ms: Date.now(), account_email: "a@b.c" });
        render(() => <MuxBusSignIn />);
        expect(screen.getByRole("status").textContent).toContain("Connected as a@b.c");
        fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
        expect(disconnect).toHaveBeenCalledTimes(1);
        acceptMuxbusDelivery({ state: "reconnecting", since_ms: Date.now() });
        expect(screen.getByRole("button", { name: "Sign out" })).toBeInTheDocument();
    });

    it("lets a running sign-in be cancelled", () => {
        acceptMuxbusDelivery({ state: "signed_out", since_ms: Date.now() });
        setLoading(true);
        render(() => <MuxBusSignIn />);
        fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
        expect(cancel).toHaveBeenCalledTimes(1);
        expect(connect).not.toHaveBeenCalled();
    });
});
