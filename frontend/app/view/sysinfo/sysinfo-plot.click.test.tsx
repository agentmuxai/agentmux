// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A click on a sysinfo chart reaches the pane around it, so the pane takes focus (the pane's own onClick). Run
 * against the real uPlot (canvas, Path2D and matchMedia stubbed, jsdom has none): with the press unbound, uPlot
 * counted every click after a mouse move as the end of a drag and stopped it before the pane saw it (Kanban #615).
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
    (window as any).matchMedia = () => ({
        addEventListener() {},
        removeEventListener() {},
        addListener() {},
        removeListener() {},
        matches: false,
    });
});
vi.mock("uplot/dist/uPlot.min.css", () => ({}));

import { FocusView } from "./sysinfo-focus";
import { sampleReducer } from "./sysinfo-model";
import { SingleLinePlot } from "./sysinfo-plot";

class FixedSizeObserver {
    constructor(private cb: ResizeObserverCallback) {}
    observe(el: Element) {
        this.cb([{ target: el, contentRect: { width: 600, height: 200 } } as ResizeObserverEntry], this as any);
    }
    unobserve() {}
    disconnect() {}
}

/** A 2D context that accepts every call and reads back what was set, enough for uPlot to build and draw. */
const ctx: any = new Proxy(
    {},
    {
        get: (t: any, k) => {
            if (k in t) return t[k];
            if (k === "measureText") return () => ({ width: 10 });
            if (k === "createLinearGradient") return () => ({ addColorStop() {} });
            return () => {};
        },
        set: (t: any, k, v) => ((t[k] = v), true),
    }
);

beforeEach(() => {
    vi.stubGlobal("ResizeObserver", FixedSizeObserver);
    vi.stubGlobal(
        "Path2D",
        class {
            constructor() {
                return new Proxy(this, { get: (t: any, k) => (k in t ? t[k] : () => {}) });
            }
        }
    );
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(ctx);
});

afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
});

describe("a click on a sysinfo chart", () => {
    it("reaches the pane around it, after the pointer moved over the chart", async () => {
        const paneClicks = vi.fn();
        const { container } = render(() => (
            <div onClick={paneClicks}>
                <SingleLinePlot
                    plotData={[{ ts: 1_791_900_000_000, cpu: 40 } as any, { ts: 1_791_900_001_000, cpu: 42 } as any]}
                    yval="cpu"
                    yvalMeta={
                        { name: "CPU", label: "%", color: "#a3e635", decimalPlaces: 0, maxy: 100 } as TimeSeriesMeta
                    }
                    blockId="b"
                    defaultColor="#a3e635"
                    targetLen={300}
                    intervalSecs={1}
                />
            </div>
        ));
        await vi.waitFor(() => expect(container.querySelector(".u-over")).not.toBeNull());
        const over = container.querySelector(".u-over") as HTMLElement;
        over.getBoundingClientRect = () =>
            ({ left: 0, top: 0, width: 600, height: 200, right: 600, bottom: 200 }) as DOMRect;
        const at = { clientX: 300, clientY: 100, bubbles: true, button: 0 };
        over.dispatchEvent(new MouseEvent("mousemove", at));
        over.dispatchEvent(new MouseEvent("mousedown", at));
        over.dispatchEvent(new MouseEvent("mouseup", at));
        over.dispatchEvent(new MouseEvent("click", at));
        expect(paneClicks).toHaveBeenCalledTimes(1);
    });
});

describe("a sysinfo chart over time", () => {
    const base = 1_791_900_000_000;
    const rows = (n: number, cores: boolean) =>
        Array.from({ length: n }).reduce<any[]>(
            (acc, _, i) =>
                sampleReducer(acc, {
                    type: "APPEND",
                    item: { ts: base + i * 1000, cpu: 40 + i, ...(cores ? { "cpu:0": 10, "cpu:1": 20 } : {}) } as any,
                    intervalSecs: 1,
                    numPoints: 300,
                }),
            []
        );

    // A rebuild replaces uPlot's `.u-over`, so a press that straddled one lost its click (Kanban #615).
    for (const cores of [false, true]) {
        it(`keeps its plot element when a sample arrives (${cores ? "with" : "without"} per-core data)`, async () => {
            const [data, setData] = createSignal(rows(3, cores));
            const { container } = render(() => (
                <FocusView area="cpu" plotData={data()} targetLen={300} intervalSecs={1} syncKey="k" />
            ));
            await vi.waitFor(() => expect(container.querySelector(".u-over")).not.toBeNull());
            const before = Array.from(container.querySelectorAll(".u-over"));
            for (let n = 4; n < 9; n++) {
                setData(rows(n, cores));
                await new Promise((r) => setTimeout(r, 20));
            }
            const after = Array.from(container.querySelectorAll(".u-over"));
            expect(after).toHaveLength(before.length);
            expect(after.every((el, i) => el === before[i])).toBe(true);
        });
    }
});
