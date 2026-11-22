// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The status bar's Graphics item ("GFX HW"): red while this window's page is busy, and the tip says why. */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ setBusy: null as any }));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("./GpuPanel", () => ({ GpuPanel: (p: { uiBusy: number | null }) => <div data-stand="GpuPanel" data-busy={String(p.uiBusy)} /> }));
vi.mock("@/store/global", () => ({ termRendererAtom: () => "webgl" }));
vi.mock("@/util/gpuutil", () => ({
    getGpuInfo: () => ({ classification: "hardware", renderer: "Some GPU" }),
    gpuClassBadge: () => "HW",
}));
vi.mock("./ui-busy", async (orig) => {
    const real = await orig<typeof import("./ui-busy")>();
    // Like the real meter: every reading is news, equal or not.
    const [busy, setBusy] = createSignal<number | null>(null, { equals: false });
    h.setBusy = setBusy;
    return { ...real, sharedUiBusy: () => busy };
});

import { closeGpuPanel } from "./gpu-panel-state";
import { GpuStatus } from "./GpuStatus";

const PLAIN = "var(--secondary-text-color)";
const RED = "var(--error-text-color)";

const button = (c: HTMLElement) => c.querySelector<HTMLButtonElement>("button.stat-gfx-button")!;

afterEach(() => {
    cleanup();
    closeGpuPanel();
    h.setBusy(null);
});

describe("the status bar's Graphics item while the UI is busy", () => {
    it("keeps its own colour and tip while the UI is not busy, and while the window has no reading", () => {
        const { container } = render(() => <GpuStatus />);
        expect(button(container).style.color).toBe(PLAIN);
        h.setBusy(12);
        expect(button(container).style.color).toBe(PLAIN);
        expect(button(container).getAttribute("data-tip")).toBe("Graphics: hardware accelerated — Some GPU · terminal: WEBGL");
        h.setBusy(null);
        expect(button(container).style.color).toBe(PLAIN);
    });

    it("turns red above the warn share and says why in its tip, with the figure", () => {
        const { container } = render(() => <GpuStatus />);
        h.setBusy(50);
        expect(button(container).style.color).toBe(PLAIN);
        h.setBusy(51);
        expect(button(container).style.color).toBe(RED);
        const tip = button(container).getAttribute("data-tip")!;
        expect(tip).toContain("Graphics: hardware accelerated");
        expect(tip).toContain("UI busy 51%: this window's page is drawing slow frames");
    });

    it("holds the red through one calm second and lets go at the second, so a stutter does not make it flicker", () => {
        const { container } = render(() => <GpuStatus />);
        h.setBusy(80);
        h.setBusy(10);
        expect(button(container).style.color).toBe(RED);
        h.setBusy(10);
        expect(button(container).style.color).toBe(PLAIN);
        h.setBusy(60);
        h.setBusy(10);
        h.setBusy(70);
        h.setBusy(10);
        expect(button(container).style.color).toBe(RED);
    });

    it("goes back to plain when the window is in the background", () => {
        const { container } = render(() => <GpuStatus />);
        h.setBusy(90);
        expect(button(container).style.color).toBe(RED);
        h.setBusy(null);
        expect(button(container).style.color).toBe(PLAIN);
    });

    it("gives the panel the same figure for its footer, with no UI pill anywhere in the bar", () => {
        const { container } = render(() => <GpuStatus />);
        button(container).click();
        h.setBusy(33);
        expect(container.querySelector("[data-stand=GpuPanel]")?.getAttribute("data-busy")).toBe("33");
        expect(container.querySelector(".stat-ui")).toBeNull();
    });
});
