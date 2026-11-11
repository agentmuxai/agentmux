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
        expect(screen.getByText("Linux")).toBeTruthy();
        expect(screen.getByTestId("lan-peer-data-channel").textContent).toBe("Data instable");
        expect(screen.getByText(/42 sec\. ago/)).toBeTruthy();
    });

    it("calls a release a release whatever channel its data is in", () => {
        // A release that a local build was upgraded to keeps that build's channel for its data.
        const upgraded = inst({ channel: "local-main-b28b7a-0000000a", build: "release", install: "linux-appimage" });
        render(() => <LanPeerList peers={[upgraded]} />);
        fireEvent.click(screen.getByRole("button", { name: /charlie/ }));
        expect(screen.getByText("Linux · release · AppImage")).toBeTruthy();
        expect(screen.getByTestId("lan-peer-data-channel").textContent).toBe("Data inlocal-main-b28b7a-0000000a");
        expect(screen.queryByText(/local build/)).toBeNull();
    });

    it("calls a local build a local build, and says nothing of a peer that did not say", () => {
        const peers = [
            inst({ instance_id: "a", hostname: "alpha", channel: "local-main-b28b7a-0000000a", build: "local" }),
            inst({ instance_id: "b", hostname: "beta", channel: "local-main-b28b7a-0000000a" }),
        ];
        render(() => <LanPeerList peers={peers} />);
        fireEvent.click(screen.getByRole("button", { name: /alpha/ }));
        fireEvent.click(screen.getByRole("button", { name: /beta/ }));
        expect(screen.getByText("Linux · local build")).toBeTruthy();
        expect(screen.getByText("Linux")).toBeTruthy();
    });

    it("opens and closes each host on its own", () => {
        const peers = [
            inst({ instance_id: "a", hostname: "alpha", address: "198.51.100.1" }),
            inst({ instance_id: "b", hostname: "beta", address: "198.51.100.2" }),
        ];
        render(() => <LanPeerList peers={peers} />);
        fireEvent.click(screen.getByRole("button", { name: /alpha/ }));
        expect(screen.getByText("198.51.100.1:4100")).toBeTruthy();
        expect(screen.queryByText("198.51.100.2:4100")).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: /beta/ }));
        expect(screen.getByText("198.51.100.1:4100")).toBeTruthy();
        expect(screen.getByText("198.51.100.2:4100")).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: /alpha/ }));
        expect(screen.queryByText("198.51.100.1:4100")).toBeNull();
        expect(screen.getByText("198.51.100.2:4100")).toBeTruthy();
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

    it("says a peer is an install with no agents, and how it is installed", () => {
        render(() => <LanPeerList peers={[inst({ hostname: "Area54", agents: [], last_polled_ok: 5, install: "windows-store", os: "windows", build: "release" })]} />);
        expect(screen.getByTestId("lan-peer-no-agents")).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: /Area54/ }));
        expect(screen.getByText("Windows · release · Microsoft Store")).toBeTruthy();
        expect(screen.getByText(/this install runs no agents/)).toBeTruthy();
    });

    it("says nothing about agents for a peer that has not been asked yet", () => {
        render(() => <LanPeerList peers={[inst({ agents: [], last_polled_ok: 0 })]} />);
        expect(screen.queryByTestId("lan-peer-no-agents")).toBeNull();
    });

    it("lists two installs of one host as two, each saying how it is installed", () => {
        const peers = [
            inst({ instance_id: "a", hostname: "Area54", channel: "stable", install: "windows-store", agents: [], last_polled_ok: 5 }),
            inst({ instance_id: "b", hostname: "Area54", channel: "stable", install: "windows-setup", agents: ["Lazo"], last_polled_ok: 5 }),
        ];
        render(() => <LanPeerList peers={peers} />);
        expect(screen.getAllByTestId("lan-peer")).toHaveLength(2);
        expect(screen.getByText("Microsoft Store")).toBeTruthy();
        expect(screen.getByText("installed")).toBeTruthy();
        expect(screen.getAllByTestId("lan-peer-no-agents")).toHaveLength(1);
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

describe("LanPeerList: the channel next to the host", () => {
    beforeEach(() => {
        vi.useFakeTimers({ toFake: ["Date"] });
        vi.setSystemTime(NOW * 1000);
        pinsList.mockReset();
        pinsList.mockResolvedValue({ lan: [], wan: [] });
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("names no channel while each host has one instance", () => {
        render(() => <LanPeerList peers={[inst({ instance_id: "a" }), inst({ instance_id: "b", hostname: "area54" })]} />);
        expect(screen.queryByText("stable")).toBeNull();
    });

    it("names each channel of a host that runs two", () => {
        const peers = [
            inst({ instance_id: "a", channel: "stable" }),
            inst({ instance_id: "b", channel: "local-3948424" }),
            inst({ instance_id: "c", hostname: "area54", channel: "stable" }),
        ];
        render(() => <LanPeerList peers={peers} />);
        expect(screen.getAllByText("stable")).toHaveLength(1);
        expect(screen.getByText("local-3948424")).toBeTruthy();
    });

    it("cuts a long channel and keeps all of it in the label and tooltip", () => {
        const long = "local-3948424-feature-branch";
        render(() => <LanPeerList peers={[inst({ instance_id: "a" }), inst({ instance_id: "b", channel: long })]} />);
        const chip = screen.getByLabelText(long);
        expect(chip.textContent).not.toBe(long);
        expect(chip.textContent).toContain("…");
    });

    it("labels a peer that is a second instance of this machine", () => {
        render(() => <LanPeerList peers={[inst({ channel: "local-1" })]} selfHost="Charlie" />);
        expect(screen.getByText("local-1")).toBeTruthy();
    });
});
