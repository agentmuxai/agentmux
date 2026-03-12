// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import * as Plot from "@observablehq/plot";
import dayjs from "dayjs";
import * as htl from "htl";
import { createEffect, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";

import type { DataItem } from "./sysinfo-types";
import { resolveDomainBound } from "./sysinfo-util";

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

function SingleLinePlot(props: SingleLinePlotProps): JSX.Element {
    let containerRef!: HTMLDivElement;
    const [plotWidth, setPlotWidth] = createSignal(0);
    const [plotHeight, setPlotHeight] = createSignal(0);

    onMount(() => {
        if (!containerRef) return;
        const rszObs = new ResizeObserver((entries) => {
            for (const entry of entries) {
                setPlotWidth(entry.contentRect.width);
                setPlotHeight(entry.contentRect.height);
            }
        });
        rszObs.observe(containerRef);
        onCleanup(() => rszObs.disconnect());
    });

    createEffect(() => {
        const { plotData, yval, yvalMeta, blockId, defaultColor, title = false, sparkline = false, targetLen, intervalSecs } = props;
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
      <linearGradient id="gradient-${blockId}-${yval}" gradientTransform="rotate(90)">
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
                fill: `url(#gradient-${blockId}-${yval})`,
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
                Plot.pointerX({ x: "ts", py: yval, stroke: "var(--grey-text-color)", strokeWidth: 1, strokeDasharray: 2 })
            )
        );
        marks.push(
            Plot.ruleY(
                plotData,
                Plot.pointerX({ px: "ts", y: yval, stroke: "var(--grey-text-color)", strokeWidth: 1, strokeDasharray: 2 })
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
                        `${dayjs.unix(d.ts / 1000).format("HH:mm:ss")} ${Number(d[yval]).toFixed(decimalPlaces)}${labelY}`,
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

        const maxY = resolveDomainBound(yvalMeta?.maxy, plotData[plotData.length - 1]) ?? 100;
        const minY = resolveDomainBound(yvalMeta?.miny, plotData[plotData.length - 1]) ?? 0;
        const maxX = plotData[plotData.length - 1].ts;
        const minX = maxX - targetLen * intervalSecs * 1000;

        const plot = Plot.plot({
            axis: !sparkline,
            x: {
                grid: true,
                label: "time",
                tickFormat: (d: number) => `${dayjs.unix(d / 1000).format("HH:mm:ss")}`,
                domain: [minX, maxX],
            },
            y: { label: labelY, domain: [minY, maxY] },
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
