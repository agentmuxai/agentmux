// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import * as Plot from "@observablehq/plot";
import dayjs from "dayjs";
import * as htl from "htl";
import type { JSX } from "solid-js";
import { createEffect, createSignal, onCleanup, onMount } from "solid-js";

import type { DataItem } from "./sysinfo-types";
import { computeAutoMaxY, resolveDomainBound } from "./sysinfo-util";

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

function SingleLinePlot(props: SingleLinePlotProps): JSX.Element {
    let containerRef!: HTMLDivElement;
    const [plotWidth, setPlotWidth] = createSignal(0);
    const [plotHeight, setPlotHeight] = createSignal(0);
    // Stable unique id for this component instance — set once, never changes.
    const gradientId = `gradient-${props.blockId}-${props.yval}-${++_gradientSeq}`;

    onMount(() => {
        if (!containerRef) return;
        let resizeTimer: ReturnType<typeof setTimeout> | null = null;
        let hasFirstSize = false;
        const rszObs = new ResizeObserver((entries) => {
            if (!hasFirstSize) {
                // First event after mount: apply immediately for instant first paint.
                // Subsequent events during dock/undock animations are debounced to
                // prevent overlapping SVGs from sharing the same gradient id.
                hasFirstSize = true;
                for (const entry of entries) {
                    setPlotWidth(entry.contentRect.width);
                    setPlotHeight(entry.contentRect.height);
                }
                return;
            }
            if (resizeTimer) clearTimeout(resizeTimer);
            resizeTimer = setTimeout(() => {
                for (const entry of entries) {
                    setPlotWidth(entry.contentRect.width);
                    setPlotHeight(entry.contentRect.height);
                }
            }, 150);
        });
        rszObs.observe(containerRef);
        onCleanup(() => {
            if (resizeTimer) clearTimeout(resizeTimer);
            rszObs.disconnect();
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

        const plot = Plot.plot({
            axis: !sparkline,
            x: {
                grid: true,
                label: "time",
                tickFormat: (d: number) => dayjs.unix(d / 1000).format("h:mm A"),
                domain: [minX, maxX],
            },
            y: { label: labelY, domain: [minY, maxY], nice: niceY },
            width: pw,
            height: ph,
            marks: marks,
        });

        containerRef.append(plot);
        onCleanup(() => {
            plot.remove();
        });
    });

    return <div ref={containerRef!} class="min-h-[100px]" />;
}

export { SingleLinePlot };
