// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** MuxBus Sign out asks first, in place, only while MuxBus can't be reached. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { acceptMuxbusDelivery } from "@/store/muxbus-delivery";
import type { MuxBusController } from "./AgentMuxConnectPanel";
import { MuxBusSignOutButton } from "./muxbus-sign-out";

afterEach(() => cleanup());

function controller(): MuxBusController & { disconnect: ReturnType<typeof vi.fn> } {
    return {
        status: () => null,
        loading: () => false,
        error: () => null,
        refresh: async () => {},
        connect: async () => {},
        cancel: async () => {},
        disconnect: vi.fn(async () => {}),
        isConfigured: () => true,
        identityProviders: () => [],
    };
}

describe("MuxBusSignOutButton", () => {
    it("signs out at once while connected", async () => {
        acceptMuxbusDelivery({ state: "connected", since_ms: Date.now() });
        const muxbus = controller();
        render(() => <MuxBusSignOutButton muxbus={muxbus} />);
        fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
        expect(muxbus.disconnect).toHaveBeenCalledTimes(1);
    });

    it("asks first while MuxBus can't be reached, and keeps the sign-in on Keep", async () => {
        acceptMuxbusDelivery({ state: "reconnecting", since_ms: Date.now() });
        const muxbus = controller();
        render(() => <MuxBusSignOutButton muxbus={muxbus} label="Disconnect" />);
        fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
        expect(muxbus.disconnect).not.toHaveBeenCalled();
        expect(screen.getByRole("alert").textContent).toContain("can't sign in again until MuxBus is reachable");
        fireEvent.click(screen.getByRole("button", { name: "Keep my sign-in" }));
        expect(muxbus.disconnect).not.toHaveBeenCalled();
        expect(screen.getByRole("button", { name: "Disconnect" })).toBeTruthy();
    });

    it("signs out on Sign out anyway", async () => {
        acceptMuxbusDelivery({ state: "reconnecting", since_ms: Date.now() });
        const muxbus = controller();
        render(() => <MuxBusSignOutButton muxbus={muxbus} />);
        fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
        fireEvent.click(screen.getByRole("button", { name: "Sign out anyway" }));
        expect(muxbus.disconnect).toHaveBeenCalledTimes(1);
    });

    it("drops the question when MuxBus comes back", async () => {
        acceptMuxbusDelivery({ state: "reconnecting", since_ms: Date.now() });
        const muxbus = controller();
        render(() => <MuxBusSignOutButton muxbus={muxbus} />);
        fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
        expect(screen.queryByRole("alert")).not.toBeNull();
        acceptMuxbusDelivery({ state: "connected", since_ms: Date.now() });
        expect(screen.queryByRole("alert")).toBeNull();
        expect(screen.getByRole("button", { name: "Sign out" })).toBeTruthy();
    });
});
