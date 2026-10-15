// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Sysinfo view: one control row for the simple and the Advanced views, its
 * pause and table shared by both; and nothing taken or drawn while the pane is
 * off screen.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const seen = vi.hoisted(() => ({ overview: [] as any[], bar: [] as any[], table: [] as any[], subscribed: 0, unsubscribed: 0 }));

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
    MeasureTable: (props: any) => {
        seen.table.push(props);
        return null;
    },
}));
vi.mock("./sysinfo-bar", () => ({
    SysinfoBar: (props: any) => {
        seen.bar.push(props);
        return null;
    },
}));

import { SysinfoView } from "./sysinfo-view";

afterEach(() => {
    cleanup();
    seen.overview.length = 0;
    seen.bar.length = 0;
    seen.table.length = 0;
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
        setSimpleView: vi.fn(),
        dispose: vi.fn(),
        loadInitialData: vi.fn(),
        appendData: vi.fn(),
    };
    return { model, setPlotType, setVisible };
}

describe("SysinfoView controls", () => {
    const bar = () => seen.bar[seen.bar.length - 1];

    it("shows one control row in both modes, with each mode's views", () => {
        const { model, setPlotType } = fakeModel();
        render(() => <SysinfoView blockId="b" model={model} />);
        expect(bar().views.map((v: any) => v.value)).toEqual(["cpu", "mem", "disk", "net", "everything"]);
        expect(bar().view).toBe("cpu");
        setPlotType("Mem");
        expect(bar().views.map((v: any) => v.label).slice(0, 3)).toEqual(["CPU", "Cores", "Memory"]);
        expect(bar().view).toBe("Mem");
        bar().onView("Net");
        expect(model.setSimpleView).toHaveBeenCalledWith("Net");
    });

    it("keeps a pause across modes, since both can resume it", () => {
        const { model, setPlotType } = fakeModel();
        render(() => <SysinfoView blockId="b" model={model} />);
        bar().onPause(true);
        setPlotType("CPU");
        expect(bar().paused).toBe(true);
    });

    it("shows a simple view's figures as a table of its area", () => {
        const { model, setPlotType } = fakeModel();
        setPlotType("Net (Sent/Recv)");
        render(() => <SysinfoView blockId="b" model={model} />);
        bar().onTable(true);
        expect(seen.table[seen.table.length - 1].focus).toBe("net");
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
