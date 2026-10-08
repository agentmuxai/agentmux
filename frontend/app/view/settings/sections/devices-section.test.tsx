// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › Paired devices: the list from `viewer.devices`, and Revoke. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const listDevices = vi.fn();
const revoke = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ViewerDevicesCommand: (...args: unknown[]) => listDevices(...args),
        ViewerRevokeCommand: (...args: unknown[]) => revoke(...args),
        PresenceStatusCommand: () => Promise.resolve({ state: "signed_out", since_ms: 0, record_version: 2 }),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ viewerPairedAtom: () => null }));

import { ago, DevicesSection } from "./devices-section";

const HOUR = 3_600_000;

describe("ago", () => {
    it("reads coarse and relative", () => {
        expect(ago(1_000, 30_000)).toBe("just now");
        expect(ago(0, 5 * 60_000)).toBe("5 min ago");
        expect(ago(0, 3 * HOUR)).toBe("3 h ago");
        // A clock behind never reads negative.
        expect(ago(5_000, 1_000)).toBe("just now");
    });
});

describe("DevicesSection", () => {
    beforeEach(() => {
        listDevices.mockReset();
        revoke.mockReset();
    });
    afterEach(() => cleanup());

    it("lists each device with when it paired and was last seen", async () => {
        const now = Date.now();
        listDevices.mockResolvedValue({
            devices: [
                { device_id: "d1", device_name: "Pixel 9", created_ms: now - 2 * HOUR, last_seen_ms: now - 10 * 60_000 },
                { device_id: "d2", device_name: "Tablet", created_ms: now - 30_000, last_seen_ms: 0 },
            ],
        });
        render(() => <DevicesSection />);
        const pixel = (await screen.findByText("Pixel 9")).closest("tr")!;
        expect(within(pixel).getByText("2 h ago")).toBeInTheDocument();
        expect(within(pixel).getByText("10 min ago")).toBeInTheDocument();
        const tablet = screen.getByText("Tablet").closest("tr")!;
        expect(within(tablet).getByText("never")).toBeInTheDocument();
    });

    it("Revoke removes the device", async () => {
        listDevices.mockResolvedValue({
            devices: [{ device_id: "d1", device_name: "Pixel 9", created_ms: Date.now(), last_seen_ms: 0 }],
        });
        revoke.mockResolvedValue({ revoked: true, feeds_closed: 1 });
        render(() => <DevicesSection />);
        const row = (await screen.findByText("Pixel 9")).closest("tr")!;
        fireEvent.click(within(row).getByRole("button", { name: "Revoke" }));
        await waitFor(() => expect(screen.queryByText("Pixel 9")).not.toBeInTheDocument());
        expect(revoke).toHaveBeenCalledWith({}, { device_id: "d1" });
        expect(screen.getByText(/No devices paired/)).toBeInTheDocument();
    });

    it("keeps the device and says so when Revoke fails", async () => {
        listDevices.mockResolvedValue({
            devices: [{ device_id: "d1", device_name: "Pixel 9", created_ms: Date.now(), last_seen_ms: 0 }],
        });
        revoke.mockRejectedValue(new Error("database is locked"));
        render(() => <DevicesSection />);
        const row = (await screen.findByText("Pixel 9")).closest("tr")!;
        fireEvent.click(within(row).getByRole("button", { name: "Revoke" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't revoke Pixel 9: database is locked");
        expect(screen.getByText("Pixel 9")).toBeInTheDocument();
    });

    it("says how to pair when nothing is paired", async () => {
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        expect(await screen.findByText(/No devices paired/)).toBeInTheDocument();
    });

    it("shows Cloud presence below the paired devices", async () => {
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        expect(await screen.findByText("Sign in to publish")).toBeInTheDocument();
        expect(screen.getByText("Cloud presence")).toBeInTheDocument();
    });
});
