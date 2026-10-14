// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Settings › Paired devices: the list from `viewer.devices`, Can send, and Revoke. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const listDevices = vi.fn();
const revoke = vi.fn();
const canSend = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ViewerDevicesCommand: (...args: unknown[]) => listDevices(...args),
        ViewerRevokeCommand: (...args: unknown[]) => revoke(...args),
        ViewerCanSendCommand: (...args: unknown[]) => canSend(...args),
        SetConfigCommand: (...args: unknown[]) => setConfig(...args),
        PresenceStatusCommand: () => Promise.resolve({ state: "signed_out", since_ms: 0, record_version: 2 }),
        MuxBusStatusCommand: () => Promise.reject(new Error("no srv")),
        MuxBusCloudConfigCommand: () => Promise.reject(new Error("no srv")),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const setConfig = vi.fn();
let settings: Record<string, unknown> = {};
vi.mock("@/app/store/global", () => ({ viewerPairedAtom: () => null, settingsAtom: () => settings }));

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
        canSend.mockReset();
        setConfig.mockReset();
        settings = {};
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

    it("Can send is off until turned on, and turns on through viewer.can-send", async () => {
        listDevices.mockResolvedValue({
            devices: [
                { device_id: "d1", device_name: "Pixel 9", created_ms: Date.now(), last_seen_ms: 0, can_send: false, has_key: true },
            ],
        });
        canSend.mockResolvedValue({ can_send: true });
        render(() => <DevicesSection />);
        const toggle = await screen.findByRole("switch", { name: "Pixel 9 can send messages" });
        expect(toggle).toHaveAttribute("aria-checked", "false");
        fireEvent.click(toggle);
        await waitFor(() =>
            expect(screen.getByRole("switch", { name: "Pixel 9 can send messages" })).toHaveAttribute("aria-checked", "true")
        );
        expect(canSend).toHaveBeenCalledWith({}, { device_id: "d1", can_send: true });
    });

    it("a device without a key says Pair again instead of a switch", async () => {
        listDevices.mockResolvedValue({
            devices: [
                { device_id: "d1", device_name: "Old phone", created_ms: Date.now(), last_seen_ms: 0, can_send: false, has_key: false },
            ],
        });
        render(() => <DevicesSection />);
        const row = (await screen.findByText("Old phone")).closest("tr")!;
        expect(within(row).getByText("Pair again")).toBeInTheDocument();
        expect(within(row).queryByRole("switch")).not.toBeInTheDocument();
    });

    it("keeps the switch where it was and says why when the change fails", async () => {
        listDevices.mockResolvedValue({
            devices: [
                { device_id: "d1", device_name: "Pixel 9", created_ms: Date.now(), last_seen_ms: 0, can_send: false, has_key: true },
            ],
        });
        canSend.mockRejectedValue(new Error("Pixel 9 paired before devices had keys. Pair it again to let it send."));
        render(() => <DevicesSection />);
        const toggle = await screen.findByRole("switch", { name: "Pixel 9 can send messages" });
        fireEvent.click(toggle);
        expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't change Pixel 9: Pixel 9 paired before");
        expect(toggle).toHaveAttribute("aria-checked", "false");
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
    it("offers Share terminals with paired devices, off unless set", async () => {
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        const toggle = await screen.findByRole("switch", { name: "Share terminals with paired devices" });
        expect(toggle).toHaveAttribute("aria-checked", "false");
        fireEvent.click(toggle);
        expect(setConfig).toHaveBeenCalledWith({}, { "viewer:shareterminals": true });
    });

    it("turning terminal sharing off clears the setting", async () => {
        settings = { "viewer:shareterminals": true };
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        const toggle = await screen.findByRole("switch", { name: "Share terminals with paired devices" });
        expect(toggle).toHaveAttribute("aria-checked", "true");
        fireEvent.click(toggle);
        expect(setConfig).toHaveBeenCalledWith({}, { "viewer:shareterminals": null });
    });

    // SPEC_STATUSBAR_HIDE_MUXBUS_SETTING_2026_10_08.md
    it("offers the MuxBus Cloud status-bar toggle, on when the setting is absent", async () => {
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        const toggle = await screen.findByRole("switch", { name: "Show MuxBus Cloud in the status bar" });
        expect(toggle).toHaveAttribute("aria-checked", "true");
    });

    it("turning it off writes statusbar:showmuxbuscloud = false", async () => {
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        fireEvent.click(await screen.findByRole("switch", { name: "Show MuxBus Cloud in the status bar" }));
        expect(setConfig).toHaveBeenCalledWith({}, { "statusbar:showmuxbuscloud": false });
    });

    it("reads off when the setting is false", async () => {
        settings = { "statusbar:showmuxbuscloud": false };
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        const toggle = await screen.findByRole("switch", { name: "Show MuxBus Cloud in the status bar" });
        expect(toggle).toHaveAttribute("aria-checked", "false");
    });

    it("offers Publish this computer to my devices, on when the setting is absent", async () => {
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        const toggle = await screen.findByRole("switch", { name: "Publish this computer to my devices" });
        expect(toggle).toHaveAttribute("aria-checked", "true");
        fireEvent.click(toggle);
        expect(setConfig).toHaveBeenCalledWith({}, { "cloud:publishpresence": false });
    });

    it("reads Publish this computer off when the setting is false", async () => {
        settings = { "cloud:publishpresence": false };
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        const toggle = await screen.findByRole("switch", { name: "Publish this computer to my devices" });
        expect(toggle).toHaveAttribute("aria-checked", "false");
    });

    it("offers Share what my agents are working on, on when the setting is absent", async () => {
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        const toggle = await screen.findByRole("switch", { name: "Share what my agents are working on" });
        expect(toggle).toHaveAttribute("aria-checked", "true");
        fireEvent.click(toggle);
        expect(setConfig).toHaveBeenCalledWith({}, { "swarm:sharesummaries": false });
    });

    it("reads Share what my agents are working on off when the setting is false", async () => {
        settings = { "swarm:sharesummaries": false };
        listDevices.mockResolvedValue({ devices: [] });
        render(() => <DevicesSection />);
        const toggle = await screen.findByRole("switch", { name: "Share what my agents are working on" });
        expect(toggle).toHaveAttribute("aria-checked", "false");
    });
});
