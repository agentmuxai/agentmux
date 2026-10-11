// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import * as Plot from "@observablehq/plot";
import dayjs from "dayjs";
import * as htl from "htl";
import type { JSX } from "solid-js";
import { batch, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { throttle } from "throttle-debounce";

import "./sysinfo-plot.scss";

import type { DataItem } from "./sysinfo-types";
import {
    buildPlotAxisLabelOptions,
    computeAutoMaxY,
    computeLeftMarginForTicks,
    computePlotMargins,
    resolveDomainBound,
    xTickFitsBeforeRightEdge,
} from "./sysinfo-util";

type SingleLinePlotProps = {
    plotData: Array<DataItem>;
    yval: string;
    yvalMeta: TimeSeriesMeta;
    blockId: string;
    defaultColor: string;
    title?: boolean;
    sparkline?: boolean;
    targetLen: number;
    intervalSecs: number;
};

// Module-level counter so each SingleLinePlot instance gets a unique gradient
// id even if two instances with the same blockId+yval briefly coexist in the
// DOM during dock/float transitions. Document-scoped SVG ids conflict across
// SVGs; a per-instance suffix prevents the wrong gradient being resolved.
let _gradientSeq = 0;

/** At most one chart rebuild per this many ms while its pane is resizing. */
const CHART_RESIZE_REDRAW_MS = 100;

/** Width in px of the widest y-axis tick label in a rendered plot; 0 if it can't be measured. */
function widestYTickLabel(plot: Element): number {
    let widest = 0;
    for (const t of Array.from(plot.querySelectorAll('[aria-label="y-axis tick label"] text'))) {
        const w = (t as SVGGraphicsElement).getBBox?.().width ?? 0;
        if (w > widest) widest = w;
    }
    return widest;
}

/** Class on every chart's SVG; sysinfo-plot.scss carries Plot's rules for it. */
export const PLOT_CLASS = "sysinfo-plot";

/**
 * Ready a freshly drawn plot before it enters the document.
 *
 * Plot puts a `<style>` in every SVG it draws. A chart redrawn as its pane
 * resizes then changed the document's stylesheets each time, and now and then
 * that invalidated every font on the page and relaid out the whole tab in the
 * middle of a window drag. Those rules are in sysinfo-plot.scss instead.
 * The drawing stretches with its SVG between throttled redraws.
 */
export function preparePlotSvg(plot: Element): void {
    plot.querySelector(":scope > style")?.remove();
    plot.setAttribute("preserveAspectRatio", "none");
}

function SingleLinePlot(props: SingleLinePlotProps): JSX.Element {
    let containerRef!: HTMLDivElement;
    const [plotWidth, setPlotWidth] = createSignal(0);
    const [plotHeight, setPlotHeight] = createSignal(0);
    // Stable unique id for this component instance — set once, never changes.
    const gradientId = `gradient-${props.blockId}-${props.yval}-${++_gradientSeq}`;

    onMount(() => {
        if (!containerRef) return;
        // A trailing timer here held the chart at its old size for a whole
        // window drag (SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md §4.2).
        // Redrawing on every tick instead cost ~8 ms a frame, since Plot builds
        // a new SVG each time. So a drag redraws at most every
        // CHART_RESIZE_REDRAW_MS, first and last size included, and the drawn
        // SVG stretches to the pane in between (see the effect below).
        // One batch, so the chart effect runs once for both dimensions.
        const resize = throttle(CHART_RESIZE_REDRAW_MS, (width: number, height: number) => {
            batch(() => {
                setPlotWidth(width);
                setPlotHeight(height);
            });
        });
        const rszObs = new ResizeObserver((entries) => {
            const rect = entries[entries.length - 1].contentRect;
            resize(rect.width, rect.height);
        });
        rszObs.observe(containerRef);
        onCleanup(() => {
            rszObs.disconnect();
            resize.cancel();
        });
    });

    createEffect(() => {
        const {
            plotData,
            yval,
            yvalMeta,
            blockId,
            defaultColor,
            title = false,
            sparkline = false,
            targetLen,
            intervalSecs,
        } = props;
        const pw = plotWidth();
        const ph = plotHeight();

        if (!containerRef) return;
        // Remove previously appended plots
        while (containerRef.firstChild) {
            containerRef.removeChild(containerRef.firstChild);
        }

        if (plotData == null || plotData.length === 0) return;

        const marks: Plot.Markish[] = [];
        const decimalPlaces = yvalMeta?.decimalPlaces ?? 0;
        let color = yvalMeta?.color;
        if (!color) color = defaultColor;

        marks.push(
            () => htl.svg`<defs>
      <linearGradient id="${gradientId}" gradientTransform="rotate(90)">
        <stop offset="0%" stop-color="${color}" stop-opacity="0.7" />
        <stop offset="100%" stop-color="${color}" stop-opacity="0" />
      </linearGradient>
        </defs>`
        );

        marks.push(
            Plot.lineY(plotData, {
                stroke: color,
                strokeWidth: 2,
                x: "ts",
                y: yval,
            })
        );

        marks.push(
            Plot.areaY(plotData, {
                fill: `url(#${gradientId})`,
                x: "ts",
                y: yval,
            })
        );

        if (title) {
            marks.push(
                Plot.text([yvalMeta?.name], {
                    frameAnchor: "top-left",
                    dx: 4,
                    fill: "var(--grey-text-color)",
                })
            );
        }

        const labelY = yvalMeta?.label ?? "?";
        marks.push(
            Plot.ruleX(
                plotData,
                Plot.pointerX({
                    x: "ts",
                    py: yval,
                    stroke: "var(--grey-text-color)",
                    strokeWidth: 1,
                    strokeDasharray: 2,
                })
            )
        );
        marks.push(
            Plot.ruleY(
                plotData,
                Plot.pointerX({
                    px: "ts",
                    y: yval,
                    stroke: "var(--grey-text-color)",
                    strokeWidth: 1,
                    strokeDasharray: 2,
                })
            )
        );
        marks.push(
            Plot.tip(
                plotData,
                Plot.pointerX({
                    x: "ts",
                    y: yval,
                    fill: "var(--main-bg-color)",
                    anchor: "middle",
                    dy: -30,
                    title: (d: any) =>
                        `${dayjs.unix(d.ts / 1000).format("h:mm:ss A")} ${Number(d[yval]).toFixed(decimalPlaces)}${labelY}`,
                    textPadding: 3,
                })
            )
        );
        marks.push(
            Plot.dot(
                plotData,
                Plot.pointerX({ x: "ts", y: yval, fill: color, r: 3, stroke: "var(--main-text-color)", strokeWidth: 1 })
            )
        );

        // Dynamic (auto-scaled) max for metrics with no natural ceiling
        // (network/disk) or a fixed ceiling that wastes most of the chart
        // when actual usage sits well below it (memory) — see
        // computeAutoMaxY's own doc comment and
        // docs/reports/REPORT_SYSINFO_COMBINED_CHART_RESEARCH_2026_08_17.md.
        // CPU keeps its fixed 0-100 (autoMaxY unset): auto-scaling an
        // already-bounded percentage would make ordinary noise read as
        // dramatic spikes.
        const hardCapY = resolveDomainBound(yvalMeta?.maxy, plotData[plotData.length - 1]);
        const maxY = yvalMeta?.autoMaxY
            ? computeAutoMaxY(plotData, yval, yvalMeta?.autoMaxYFloor ?? 1, hardCapY)
            : (hardCapY ?? 100);
        const minY = resolveDomainBound(yvalMeta?.miny, plotData[plotData.length - 1]) ?? 0;
        const maxX = plotData[plotData.length - 1].ts;
        const minX = maxX - targetLen * intervalSecs * 1000;

        // `nice: true` rounds the computed domain OUTWARD to human-friendly
        // tick values (e.g. 0-87 -> 0-100) — good for an uncapped autoMaxY
        // metric (network/disk), where there's no physical ceiling to
        // violate. Disabled specifically when a hard cap is in effect
        // (memory's mem:total): nicing can push the rendered axis max past
        // the cap, visually showing headroom that doesn't exist and
        // defeating computeAutoMaxY's hard-cap guarantee (reagentx P1 on PR
        // #2638). CPU's fixed [0, 100] doesn't need nicing either way — a
        // round domain already.
        const niceY = yvalMeta?.autoMaxY && hardCapY == null;

        // Tight, panel-appropriate margins and no axis labels — see
        // computePlotMargins/buildPlotAxisLabelOptions for the rationale.
        const margins = computePlotMargins(sparkline, title);
        const axisLabels = buildPlotAxisLabelOptions();

        const buildPlot = (marginLeft: number) => {
            const plot = Plot.plot({
                className: PLOT_CLASS,
                axis: !sparkline,
                ...margins,
                marginLeft,
                x: {
                    grid: true,
                    ...axisLabels.x,
                    tickFormat: (d: number) =>
                        xTickFitsBeforeRightEdge(d, minX, maxX, pw, marginLeft, margins.marginRight)
                            ? dayjs.unix(d / 1000).format("h:mm A")
                            : "",
                    domain: [minX, maxX],
                },
                y: { ...axisLabels.y, domain: [minY, maxY], nice: niceY },
                width: pw,
                height: ph,
                marks: marks,
            });
            preparePlotSvg(plot);
            return plot;
        };
        let plot = buildPlot(margins.marginLeft);
        containerRef.append(plot);
        if (!sparkline) {
            // Fit the left margin to the labels as actually rendered, so the
            // widest sits PLOT_EDGE_PAD_PX from the pane's edge. One refit only.
            const fitted = computeLeftMarginForTicks(widestYTickLabel(plot));
            if (Math.abs(fitted - margins.marginLeft) >= 1) {
                const refit = buildPlot(fitted);
                containerRef.replaceChild(refit, plot);
                plot = refit;
            }
        }
        onCleanup(() => {
            plot.remove();
        });
    });

    return <div ref={containerRef!} class="min-h-[100px]" />;
}

export { SingleLinePlot };
