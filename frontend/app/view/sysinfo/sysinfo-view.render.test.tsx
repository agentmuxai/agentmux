// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Sysinfo view: its pause, the area view's own, lifted when a classic plot
 * type is shown; and nothing taken or drawn while the pane is off screen.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const seen = vi.hoisted(() => ({ overview: [] as any[], subscribed: 0, unsubscribed: 0 }));

vi.mock("@/app/store/mps", () => ({
    onMuxEvent: () => {
        seen.subscribed++;
        return () => seen.unsubscribed++;
    },
}));
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
    seen.subscribed = 0;
    seen.unsubscribed = 0;
});

function fakeModel(shown = true) {
    const [plotType, setPlotType] = createSignal("Overview");
    const [visible, setVisible] = createSignal(shown);
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
        focus: () => "cpu",
        visible,
        setWindow: vi.fn(),
        setFocus: vi.fn(),
        dispose: vi.fn(),
        loadInitialData: vi.fn(),
        appendData: vi.fn(),
    };
    return { model, setPlotType, setVisible };
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

describe("SysinfoView off screen", () => {
    it("takes no samples while hidden, and reads what it missed when shown again", () => {
        const { model, setVisible } = fakeModel();
        render(() => <SysinfoView blockId="b" model={model} />);
        expect(seen.subscribed).toBe(1);
        setVisible(false);
        expect(seen.unsubscribed).toBe(1);
        expect(model.loadInitialData).not.toHaveBeenCalled();
        setVisible(true);
        expect(seen.subscribed).toBe(2);
        expect(model.loadInitialData).toHaveBeenCalledTimes(1);
    });

    it("never subscribes while it opens hidden", () => {
        const { model } = fakeModel(false);
        render(() => <SysinfoView blockId="b" model={model} />);
        expect(seen.subscribed).toBe(0);
    });
});
