// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Pair a device" in the host popover: the QR carries a one-time
 * `agentmux://pair` URL from `viewer.pair-start`, never the instance key.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const pairStart = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ViewerPairStartCommand: (...args: unknown[]) => pairStart(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const [paired, setPaired] = createSignal<{ deviceId: string; deviceName: string } | null>(null);
vi.mock("@/store/global", () => ({ viewerPairedAtom: () => paired() }));
const toCanvas = vi.fn();
vi.mock("qrcode", () => ({ default: { toCanvas: (...args: unknown[]) => toCanvas(...args) } }));

import { countdown, PairDevicePanel } from "./PairDevicePanel";

const URL_ = "agentmux://pair?v=1&host=198.51.100.7&port=29702&fp=ab&code=ABCDEFGH23&hostname=h&channel=stable";

describe("countdown", () => {
    it("reads m:ss and ends at zero", () => {
        expect(countdown(120_000, 0)).toBe("2:00");
        expect(countdown(120_000, 61_500)).toBe("0:59");
        expect(countdown(120_000, 119_001)).toBe("0:01");
        expect(countdown(120_000, 120_000)).toBeNull();
        expect(countdown(120_000, 200_000)).toBeNull();
    });
});

describe("PairDevicePanel", () => {
    beforeEach(() => {
        pairStart.mockReset();
        toCanvas.mockReset();
        setPaired(null);
    });
    afterEach(() => cleanup());

    it("explains that pairing needs LAN discovery while it is off", () => {
        render(() => <PairDevicePanel lanDiscoveryEnabled={() => false} />);
        expect(screen.getByRole("button", { name: "Pair a device" })).toBeDisabled();
        expect(screen.getByText(/Turn on LAN discovery to pair a device/)).toBeInTheDocument();
        expect(pairStart).not.toHaveBeenCalled();
    });

    it("shows the pairing URL as a QR with a countdown", async () => {
        pairStart.mockResolvedValue({ url: URL_, expires_ms: Date.now() + 120_000 });
        render(() => <PairDevicePanel lanDiscoveryEnabled={() => true} />);
        fireEvent.click(screen.getByRole("button", { name: "Pair a device" }));
        await waitFor(() => expect(toCanvas).toHaveBeenCalled());
        expect(toCanvas.mock.calls[0][1]).toBe(URL_);
        expect(String(toCanvas.mock.calls[0][1])).not.toContain("agentmux://connect");
        expect(screen.getByTestId("pair-countdown").textContent).toMatch(/for [12]:\d\d/);
        expect(screen.getByRole("button", { name: "New code" })).toBeInTheDocument();
    });

    it("copies the pairing link, for another AgentMux computer's Tower", async () => {
        const writeText = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
        pairStart.mockResolvedValue({ url: URL_, expires_ms: Date.now() + 120_000 });
        render(() => <PairDevicePanel lanDiscoveryEnabled={() => true} />);
        fireEvent.click(screen.getByRole("button", { name: "Pair a device" }));
        fireEvent.click(await screen.findByRole("button", { name: "Copy link" }));
        expect(writeText).toHaveBeenCalledWith(URL_);
        expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
    });

    it("New code asks for another", async () => {
        pairStart.mockResolvedValue({ url: URL_, expires_ms: Date.now() + 120_000 });
        render(() => <PairDevicePanel lanDiscoveryEnabled={() => true} />);
        fireEvent.click(screen.getByRole("button", { name: "Pair a device" }));
        await screen.findByRole("button", { name: "New code" });
        fireEvent.click(screen.getByRole("button", { name: "New code" }));
        await waitFor(() => expect(pairStart).toHaveBeenCalledTimes(2));
    });

    it("says when the code has expired instead of showing a dead QR", async () => {
        pairStart.mockResolvedValue({ url: URL_, expires_ms: Date.now() - 1 });
        render(() => <PairDevicePanel lanDiscoveryEnabled={() => true} />);
        fireEvent.click(screen.getByRole("button", { name: "Pair a device" }));
        expect(await screen.findByText(/This code expired/)).toBeInTheDocument();
        expect(screen.queryByLabelText("Pairing QR code")).not.toBeInTheDocument();
    });

    it("shows why there is no QR when the server refuses", async () => {
        pairStart.mockRejectedValue(new Error("The viewer isn't listening on this network."));
        render(() => <PairDevicePanel lanDiscoveryEnabled={() => true} />);
        fireEvent.click(screen.getByRole("button", { name: "Pair a device" }));
        expect(await screen.findByRole("alert")).toHaveTextContent("The viewer isn't listening on this network.");
        expect(toCanvas).not.toHaveBeenCalled();
    });

    it("takes the QR down once a device pairs", async () => {
        pairStart.mockResolvedValue({ url: URL_, expires_ms: Date.now() + 120_000 });
        render(() => <PairDevicePanel lanDiscoveryEnabled={() => true} />);
        fireEvent.click(screen.getByRole("button", { name: "Pair a device" }));
        await screen.findByLabelText("Pairing QR code");
        setPaired({ deviceId: "d1", deviceName: "Pixel 9" });
        expect(await screen.findByText(/Paired Pixel 9/)).toBeInTheDocument();
        expect(screen.queryByLabelText("Pairing QR code")).not.toBeInTheDocument();
    });
});
