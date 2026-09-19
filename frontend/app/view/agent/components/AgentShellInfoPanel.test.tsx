// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentShellInfoPanel — the Shell drawer's info line
 * (SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §4).
 *
 * The panel's whole job is to report state it doesn't own, so the tests feed
 * it that state directly: a `controllerstatus` payload for the shell half,
 * and a stubbed `useTrackedProcesses` for the process half. What's asserted
 * is the reporting — including the two ways it must NOT report: no process
 * claim at all on a platform without a tracker, and nothing where the agent
 * has started nothing.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentShellInfoPanel } from "./AgentShellInfoPanel";
import type { TrackedProcessInfo, TrackingConfidence } from "../hooks/useTrackedProcesses";

// Captured `controllerstatus` handler, so a test can publish a payload.
let statusHandler: ((event: { data: unknown }) => void) | undefined;

vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (sub: { eventType: string; handler: (e: { data: unknown }) => void }) => {
        if (sub.eventType === "controllerstatus") statusHandler = sub.handler;
        return () => {};
    },
}));

const [agentList, setAgentList] = createSignal<TrackedProcessInfo[]>([]);
const [shellList, setShellList] = createSignal<TrackedProcessInfo[]>([]);
const [confidence, setConfidence] = createSignal<TrackingConfidence>("high");

vi.mock("../hooks/useTrackedProcesses", () => ({
    useTrackedProcesses: (blockId: () => string | undefined) => ({
        // The agent pane's own block vs the drawer shell's sub-block.
        list: () => (blockId() === "sub-1" ? shellList() : agentList()),
        confidence,
        refresh: () => {},
    }),
}));

const proc = (pid: number, command: string, rss = 0): TrackedProcessInfo => ({
    pid,
    command,
    rss_bytes: rss,
    started_at_ms: 0,
});

const publishStatus = (data: Record<string, unknown>) => statusHandler?.({ data });

const renderPanel = () =>
    render(() => <AgentShellInfoPanel blockId="block-1" shellSubBlockId="sub-1" cwd="C:\\repo\\app" />);

describe("AgentShellInfoPanel", () => {
    afterEach(cleanup);

    beforeEach(() => {
        statusHandler = undefined;
        setAgentList([]);
        setShellList([]);
        setConfidence("high");
    });

    it("reports the shell's name, pid and cwd once the controller says so", () => {
        renderPanel();
        // Mid-bucket on purpose: the panel samples `now` when it renders,
        // fractionally before this line runs, so an exact 12-minute offset
        // floors to 11m.
        publishStatus({ shellprocstatus: "running", shellprocpid: 18432, shellprocname: "pwsh", spawn_ts_ms: Date.now() - 12.5 * 60_000 });
        expect(screen.getByText("pwsh")).toBeInTheDocument();
        expect(screen.getByText("pid 18432")).toBeInTheDocument();
        expect(screen.getByText("12m")).toBeInTheDocument();
        expect(screen.getByText(/repo.app/)).toBeInTheDocument();
    });

    it("surfaces a non-zero exit and drops the uptime", () => {
        renderPanel();
        publishStatus({ shellprocstatus: "running", shellprocpid: 1, shellprocname: "bash", spawn_ts_ms: Date.now() - 60_000 });
        publishStatus({ shellprocstatus: "done", shellprocexitcode: 1, shellprocpid: 1, shellprocname: "bash" });
        expect(screen.getByText("exited 1")).toBeInTheDocument();
        expect(screen.queryByText("1m")).toBeNull();
    });

    it("counts what the agent started, with total memory", () => {
        renderPanel();
        setAgentList([proc(100, "C:\\nodejs\\node.exe", 200 * 1024 * 1024), proc(101, "C:\\nodejs\\node.exe", 112 * 1024 * 1024)]);
        expect(screen.getByText("2 started by the agent")).toBeInTheDocument();
        expect(screen.getByText("312 MB")).toBeInTheDocument();
    });

    it("says nothing about processes when the agent has started none", () => {
        renderPanel();
        expect(screen.queryByText(/started by the agent/)).toBeNull();
        expect(screen.queryByText(/process tracking unavailable/)).toBeNull();
    });

    it("counts the drawer shell's own processes separately", () => {
        renderPanel();
        setAgentList([proc(100, "C:\\nodejs\\node.exe")]);
        setShellList([proc(200, "C:\\python\\python.exe")]);
        expect(screen.getByText("1 started by the agent")).toBeInTheDocument();
        expect(screen.getByText("1 from this shell")).toBeInTheDocument();
    });

    it("admits when the platform has no tracker rather than claiming zero", () => {
        setConfidence("none");
        renderPanel();
        setAgentList([]);
        expect(screen.getByText(/process tracking unavailable/)).toBeInTheDocument();
        expect(screen.queryByText(/started by the agent/)).toBeNull();
    });

    it("marks a best-effort count as approximate", () => {
        setConfidence("best_effort");
        renderPanel();
        setAgentList([proc(100, "/usr/bin/node")]);
        expect(screen.getByText(/≈/)).toBeInTheDocument();
    });
});
