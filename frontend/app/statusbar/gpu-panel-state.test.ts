// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { GpuSampleResult } from "@/types/rpc/GpuSampleResult";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import {
    closeGpuPanel,
    gpuPanelAnchor,
    MIN_POLL_MS,
    pollDelay,
    RETRY_POLL_MS,
    toggleGpuPanel,
    WARMING_POLL_MS,
} from "./gpu-panel-state";

const sample = (over: Partial<GpuSampleResult>): GpuSampleResult => ({
    ts_ms: 1,
    os: "windows",
    state: "ok",
    adapters: [],
    processes: [],
    per_process: true,
    interval_ms: 2000,
    sample_ms: 5,
    widened: false,
    ...over,
});

describe("when the panel asks again", () => {
    it("waits what srv says, never less than two seconds", () => {
        expect(pollDelay(sample({ interval_ms: 8000 }))).toBe(8000);
        expect(pollDelay(sample({ interval_ms: 500 }))).toBe(MIN_POLL_MS);
    });

    it("asks again soon after the first of the two reads the counters need", () => {
        expect(pollDelay(sample({ state: "warming" }))).toBe(WARMING_POLL_MS);
    });

    it("waits longer after a failed call", () => {
        expect(pollDelay(null)).toBe(RETRY_POLL_MS);
    });
});

describe("which readout the panel is open from", () => {
    it("opens from a readout, closes from the same one, and moves between the two", () => {
        const meter = {} as HTMLElement;
        const item = {} as HTMLElement;
        expect(gpuPanelAnchor()).toBeNull();
        toggleGpuPanel(meter);
        expect(gpuPanelAnchor()).toBe(meter);
        toggleGpuPanel(item);
        expect(gpuPanelAnchor()).toBe(item);
        toggleGpuPanel(item);
        expect(gpuPanelAnchor()).toBeNull();
        toggleGpuPanel(meter);
        closeGpuPanel();
        expect(gpuPanelAnchor()).toBeNull();
    });
});
