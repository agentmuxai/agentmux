// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const openPane = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: openPane }));
const pinsList = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { PinsListCommand: pinsList } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import type { LanInstance } from "@/types/rpc/LanInstance";
import { LanPeerList } from "./LanPeerRow";

// The component reads the clock when it renders, so the clock is frozen at this second for every test.
const NOW = 1_800_000_000;
const inst = (over: Partial<LanInstance> = {}): LanInstance =>
    ({
        instance_id: "i1",
        hostname: "charlie",
        version: "0.59.40",
        channel: "stable",
        os: "linux",
        address: "203.0.113.30",
        port: 4100,
        agents: ["Parlo", "Opaz"],
        first_seen: NOW - 100,
        last_seen: NOW - 42,
        ...over,
    }) as LanInstance;

describe("LanPeerList", () => {
    beforeEach(() => {
        vi.useFakeTimers({ toFake: ["Date"] });
        vi.setSystemTime(NOW * 1000);
        openPane.mockReset();
        pinsList.mockReset();
        pinsList.mockResolvedValue({ lan: [], wan: [] });
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("shows the name and version, and the details only once opened", async () => {
        render(() => <LanPeerList peers={[inst()]} />);
        expect(screen.getByText("charlie")).toBeTruthy();
        expect(screen.getByText("v0.59.40")).toBeTruthy();
        expect(screen.queryByText("203.0.113.30:4100")).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: /charlie/ }));
        expect(screen.getByText("203.0.113.30:4100")).toBeTruthy();
        expect(screen.getByText("Linux · stable")).toBeTruthy();
        expect(screen.getByText(/42 s ago/)).toBeTruthy();
    });

    it("opens the Swarm from an open row with agents", async () => {
        render(() => <LanPeerList peers={[inst()]} />);
        fireEvent.click(screen.getByRole("button", { name: /charlie/ }));
        fireEvent.click(screen.getByText("Show in Swarm"));
        expect(openPane).toHaveBeenCalledWith("swarm");
    });

    it("has no Show in Swarm for a peer with no agents", () => {
        render(() => <LanPeerList peers={[inst({ agents: [] })]} />);
        fireEvent.click(screen.getByRole("button", { name: /charlie/ }));
        expect(screen.queryByText("Show in Swarm")).toBeNull();
    });

    it("warns on a peer whose agent's key was refused and links to Pinned senders", async () => {
        pinsList.mockResolvedValue({
            lan: [
                {
                    agent_id: "parlo",
                    key_fp: "aa",
                    pinned_at: 1,
                    uid_pins: 0,
                    refused: { key_fp: "bb", last_seen_at: NOW - 60, count: 3 },
                },
            ],
            wan: [],
        });
        render(() => <LanPeerList peers={[inst()]} />);
        await waitFor(() => expect(screen.getByTestId("lan-peer-key-warning-icon")).toBeTruthy());
        fireEvent.click(screen.getByRole("button", { name: /charlie/ }));
        expect(screen.getByTestId("lan-peer-key-warning").textContent).toContain("3 times");
        fireEvent.click(screen.getByText(/Pinned senders/));
        expect(openPane).toHaveBeenCalledWith(
            "settings",
            { meta: { view: "settings", "settings:section": "devices" } },
            { "settings:section": "devices" }
        );
    });

    it("shows the rows without a warning when the pins cannot be read", async () => {
        pinsList.mockRejectedValue(new Error("no"));
        render(() => <LanPeerList peers={[inst()]} />);
        await waitFor(() => expect(pinsList).toHaveBeenCalled());
        expect(screen.queryByTestId("lan-peer-key-warning-icon")).toBeNull();
        expect(screen.getByText("charlie")).toBeTruthy();
    });

    it("keeps a row open when an event replaces the peer objects", () => {
        const [peers, setPeers] = createSignal([inst({ instance_id: "a" }), inst({ instance_id: "b", hostname: "other" })]);
        render(() => <LanPeerList peers={peers()} />);
        fireEvent.click(screen.getByRole("button", { name: /other/ }));
        expect(screen.getAllByText("203.0.113.30:4100")).toHaveLength(1);
        // The same peers as new objects, as the next laninstances event delivers them.
        setPeers([inst({ instance_id: "a" }), inst({ instance_id: "b", hostname: "other" })]);
        expect(screen.getAllByText("203.0.113.30:4100")).toHaveLength(1);
        expect(screen.getByRole("button", { name: /other/ }).getAttribute("aria-expanded")).toBe("true");
    });
});
