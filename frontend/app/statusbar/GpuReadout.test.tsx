// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ busy: null as any, setBusy: null as any }));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("./GpuPanel", () => ({ GpuPanel: () => <div data-stand="GpuPanel" /> }));
vi.mock("./ui-busy", async (orig) => {
    const real = await orig<typeof import("./ui-busy")>();
    const [busy, setBusy] = createSignal<number | null>(null);
    h.busy = busy;
    h.setBusy = setBusy;
    return { ...real, sharedUiBusy: () => busy };
});

import { closeGpuPanel } from "./gpu-panel-state";
import { GpuReadout, UiBusyReadout } from "./SystemStatsParts";
import type { SysStats } from "./SystemStatsState";

const stats = (gpu: number | null, gpuMem: number | null = null): SysStats => ({
    cpu: 10,
    gpu,
    gpuMem,
    memUsed: 1,
    memTotal: 2,
    commitUsed: 0,
    commitTotal: 0,
    diskRead: 0,
    diskWrite: 0,
    netSent: 0,
    netRecv: 0,
    watchVolumeFreeGb: null,
    watchVolumeFreePct: null,
    pagefileSystemManaged: null,
});

const WARN = "var(--warning-text-color)";

afterEach(() => {
    cleanup();
    closeGpuPanel();
    h.setBusy(null);
});

describe("the GPU meter beside CPU and Mem", () => {
    it("is not there at all while no GPU is readable: not 0%, not an error", () => {
        const { container } = render(() => <GpuReadout s={() => stats(null)} />);
        expect(container.textContent).toBe("");
        expect(container.querySelector(".stat-separator")).toBeNull();
    });

    it("reads as a meter with its separator, in the bar's plain colour", () => {
        const { container } = render(() => <GpuReadout s={() => stats(35, 40)} />);
        const meter = container.querySelector<HTMLButtonElement>("button.stat-gpu")!;
        expect(meter.textContent).toBe("GPU 35%");
        expect(container.querySelector(".stat-separator")?.nextElementSibling).toBe(meter);
        expect(meter.style.color).toBe("var(--secondary-text-color)");
    });

    it("turns the warning colour at 90% use, and at 90% memory", () => {
        const use = render(() => <GpuReadout s={() => stats(91, 10)} />);
        expect(use.container.querySelector<HTMLElement>(".stat-gpu")!.style.color).toBe(WARN);
        cleanup();
        const mem = render(() => <GpuReadout s={() => stats(10, 90)} />);
        expect(mem.container.querySelector<HTMLElement>(".stat-gpu")!.style.color).toBe(WARN);
    });

    it("opens the panel on a click and closes it on the next", () => {
        const { container } = render(() => <GpuReadout s={() => stats(35)} />);
        const meter = container.querySelector<HTMLButtonElement>("button.stat-gpu")!;
        expect(meter.getAttribute("aria-expanded")).toBe("false");
        expect(container.querySelector("[data-stand=GpuPanel]")).toBeNull();
        fireEvent.click(meter);
        expect(meter.getAttribute("aria-expanded")).toBe("true");
        expect(container.querySelector("[data-stand=GpuPanel]")).toBeTruthy();
        fireEvent.click(meter);
        expect(container.querySelector("[data-stand=GpuPanel]")).toBeNull();
    });
});

describe("the UI busy meter", () => {
    it("is not there while this window has nothing to say", () => {
        const { container } = render(() => <UiBusyReadout />);
        expect(container.textContent).toBe("");
    });

    it("reads UI n%, and turns the warning colour above 50%", () => {
        const { container } = render(() => <UiBusyReadout />);
        h.setBusy(12);
        const meter = container.querySelector<HTMLElement>(".stat-ui")!;
        expect(meter.textContent).toBe("UI 12%");
        expect(meter.style.color).toBe("var(--secondary-text-color)");
        h.setBusy(51);
        expect(container.querySelector<HTMLElement>(".stat-ui")!.style.color).toBe(WARN);
        h.setBusy(null);
        expect(container.querySelector(".stat-ui")).toBeNull();
    });

    it("says in its tooltip that it covers this window only", () => {
        const { container } = render(() => <UiBusyReadout />);
        h.setBusy(5);
        const tip = container.querySelector<HTMLElement>(".stat-ui")!.getAttribute("data-tip")!;
        expect(tip).toContain("this window only");
        expect(tip).toContain("not agents");
        expect(tip).toContain("background");
    });
});
