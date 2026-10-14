// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Sysinfo view's pause: the Overview's own, and lifted when a classic plot type is shown. */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const seen = vi.hoisted(() => ({ overview: [] as any[] }));

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/store/pane-content-holds", () => ({ holdPaneContent: () => () => {}, trackPaneContent: () => () => {} }));
vi.mock("@/app/platform/window-resize", () => ({ windowResizing: () => false }));
vi.mock("./sysinfo-plot", () => ({ SingleLinePlot: () => null }));
vi.mock("./sysinfo-overview", () => ({
    Overview: (props: any) => {
        seen.overview.push(props);
        return null;
    },
}));

import { SysinfoView } from "./sysinfo-view";

afterEach(() => {
    cleanup();
    seen.overview.length = 0;
});

function fakeModel() {
    const [plotType, setPlotType] = createSignal("Overview");
    const model: any = {
        blockId: "b",
        plotTypeSelectedAtom: plotType,
        dataAtom: () => [],
        loadingAtom: () => false,
        connStatus: () => ({ status: "connected" }),
        connection: () => "local",
        metrics: () => ["cpu"],
        plotMetaAtom: () => new Map(),
        numPoints: () => 120,
        intervalSecsAtom: () => 1,
        windowSecs: () => 120,
        focus: () => null,
        setWindow: vi.fn(),
        setFocus: vi.fn(),
        dispose: vi.fn(),
        loadInitialData: vi.fn(),
        appendData: vi.fn(),
    };
    return { model, setPlotType };
}

describe("SysinfoView pause", () => {
    it("is lifted when a classic plot type, which has no resume control, is shown", () => {
        const { model, setPlotType } = fakeModel();
        render(() => <SysinfoView blockId="b" model={model} />);
        const overview = () => seen.overview[seen.overview.length - 1];
        overview().onPause(true);
        expect(overview().paused).toBe(true);
        setPlotType("CPU");
        setPlotType("Overview");
        expect(overview().paused).toBe(false);
    });

    it("holds while the Overview stays", () => {
        const { model } = fakeModel();
        render(() => <SysinfoView blockId="b" model={model} />);
        const overview = () => seen.overview[seen.overview.length - 1];
        overview().onPause(true);
        expect(overview().paused).toBe(true);
    });
});
