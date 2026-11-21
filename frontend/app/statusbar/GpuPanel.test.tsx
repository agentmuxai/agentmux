// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { GpuSampleResult } from "@/types/rpc/GpuSampleResult";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    sample: vi.fn(),
    openPane: vi.fn(async () => undefined),
    gfx: { classification: "hardware", renderer: "NVIDIA GeForce RTX 4070" } as {
        classification: string;
        renderer: string;
    },
}));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { GpuSampleCommand: h.sample } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/palette-open-pane", () => ({ openPane: h.openPane }));
vi.mock("@/util/gpuutil", () => ({ getGpuInfo: () => h.gfx }));
vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (p: { class?: string; role?: JSX.HTMLAttributes<HTMLDivElement>["role"]; children?: JSX.Element }) => (
        <div class={p.class} role={p.role}>
            {p.children}
        </div>
    ),
}));

import { GpuPanel } from "./GpuPanel";

const GB = 1024 ** 3;
const result = (over: Partial<GpuSampleResult> = {}): GpuSampleResult => ({
    ts_ms: 1,
    os: "windows",
    state: "ok",
    adapters: [
        {
            luid: "0x00000000_0x00000001",
            name: "NVIDIA GeForce RTX 4070",
            kind: "discrete",
            use_pct: 35,
            dedicated_used: 3 * GB,
            dedicated_total: 12 * GB,
            shared_used: 1 * GB,
            shared_total: 16 * GB,
        },
    ],
    processes: [
        { pid: 4, name: "chrome.exe", use_pct: 20, dedicated: 1 * GB, shared: 0 },
        { pid: 9, name: "agentmux-host.exe", label: "GPU", kind: "agentmux", use_pct: 8, dedicated: 0.5 * GB, shared: 0 },
    ],
    others: { processes: 3, use_pct: 4, dedicated: 0.25 * GB },
    per_process: true,
    interval_ms: 2000,
    sample_ms: 7,
    widened: false,
    ...over,
});

const mount = (busy: number | null = 12) => render(() => <GpuPanel anchor={null} uiBusy={busy} onClose={() => undefined} />);

beforeEach(() => {
    h.sample.mockReset();
    h.openPane.mockClear();
    h.gfx.classification = "hardware";
});
afterEach(cleanup);

describe("the GPU panel", () => {
    it("shows the adapter with its use and memory, and the processes in the order srv sent", async () => {
        h.sample.mockResolvedValue(result());
        const { container } = mount();
        await waitFor(() => expect(screen.getByText("35%")).toBeTruthy());
        expect(container.textContent).toContain("Discrete");
        expect(container.textContent).toContain("3.0 GB / 12 GB");
        const names = [...container.querySelectorAll(".gpu-process-list .gpu-process-name")].map((n) => n.textContent);
        expect(names).toEqual(["chrome.exe", "agentmux-host.exeGPU", "3 other processes"]);
        expect(container.querySelector(".gpu-process-others .gpu-process-use")?.textContent).toBe("4%");
    });

    it("opens Tower from an AgentMux row, and closes itself", async () => {
        h.sample.mockResolvedValue(result());
        const onClose = vi.fn();
        const { container } = render(() => <GpuPanel anchor={null} uiBusy={null} onClose={onClose} />);
        await waitFor(() => expect(container.querySelector(".gpu-process-own")).toBeTruthy());
        expect(container.querySelectorAll("button.gpu-process").length).toBe(1);
        fireEvent.click(container.querySelector(".gpu-process-own")!);
        expect(onClose).toHaveBeenCalled();
        expect(h.openPane).toHaveBeenCalledWith({
            meta: { view: "tower", "tower:view": "processes", "tower:only": "agentmux" },
        });
    });

    it("says what the counters said when there is no adapter to show", async () => {
        h.sample.mockResolvedValue(result({ state: "unavailable", adapters: [], processes: [], others: undefined }));
        const { container } = mount();
        await waitFor(() => expect(container.textContent).toContain("gives no GPU counters"));
        expect(container.querySelector(".gpu-adapters")).toBeNull();
        expect(container.querySelector(".gpu-processes")).toBeNull();
    });

    it("says a system does not read GPU figures yet, by its name", async () => {
        h.sample.mockResolvedValue(result({ state: "unsupported", os: "macos", adapters: [], processes: [] }));
        const { container } = mount();
        await waitFor(() => expect(container.textContent).toContain("does not read GPU figures on macOS yet"));
    });

    it("says where only the adapter's figures are known", async () => {
        h.sample.mockResolvedValue(result({ per_process: false, processes: [], others: undefined, os: "macos" }));
        const { container } = mount();
        await waitFor(() => expect(container.textContent).toContain("does not say which process uses the GPU"));
        expect(container.querySelector(".gpu-process-list")).toBeNull();
    });

    it("says software rendering, with the renderer string, and still opens", async () => {
        h.gfx.classification = "software";
        h.gfx.renderer = "Google SwiftShader";
        h.sample.mockResolvedValue(result({ state: "ok", adapters: [], processes: [], others: undefined }));
        const { container } = mount();
        expect(container.textContent).toContain("Software rendering: no GPU is in use");
        expect(container.textContent).toContain("Google SwiftShader");
        await waitFor(() => expect(container.textContent).toContain("No GPU found"));
    });

    it("says when a read was slow enough to space the reads out", async () => {
        h.sample.mockResolvedValue(result({ widened: true, interval_ms: 8000, sample_ms: 90 }));
        const { container } = mount();
        await waitFor(() => expect(container.textContent).toContain("a read took 90 ms"));
        expect(container.querySelector(".gpu-footer-widened")).toBeTruthy();
    });

    it("shows the page's busy share in the footer, or says when it is not shown", async () => {
        h.sample.mockResolvedValue(result());
        const shown = mount(12);
        await waitFor(() => expect(shown.container.textContent).toContain("UI busy 12%"));
        cleanup();
        const hidden = mount(null);
        await waitFor(() => expect(hidden.container.querySelector(".gpu-footer-ui")?.textContent).toContain("in front"));
    });

    it("says when srv could not be asked", async () => {
        h.sample.mockRejectedValue(new Error("gone"));
        const { container } = mount();
        await waitFor(() => expect(container.textContent).toContain("could not be asked"));
    });
});
