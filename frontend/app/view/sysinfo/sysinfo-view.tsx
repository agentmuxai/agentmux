// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { windowResizing } from "@/app/platform/window-resize";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { holdPaneContent, trackPaneContent } from "@/app/store/pane-content-holds";
import clsx from "clsx";
import type { JSX } from "solid-js";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";

import type { SysinfoViewModel } from "./sysinfo-model";
import { Overview } from "./sysinfo-overview";
import { SingleLinePlot } from "./sysinfo-plot";
import { convertTimeSeriesToDataItem } from "./sysinfo-util";

type SysinfoViewProps = {
    blockId: string;
    model: SysinfoViewModel;
};

function SysinfoView(props: SysinfoViewProps): JSX.Element {
    const { model, blockId } = props;
    // The chart is empty until its history arrives: hold the pane until then
    // (pane-content-holds.ts).
    onCleanup(trackPaneContent(blockId));
    onCleanup(() => model.dispose());
    const releaseContent = holdPaneContent(blockId);
    onCleanup(releaseContent);
    createEffect(() => {
        if (!model.loadingAtom()) releaseContent();
    });
    const connName = createMemo(() => model.connection());
    const connStatus = createMemo(() => model.connStatus());

    let lastConnName = connName();

    // Reload data when connection changes
    createEffect(() => {
        const cs = connStatus();
        const cn = connName();
        if (cs?.status != "connected") return;
        if (lastConnName !== cn) {
            lastConnName = cn;
            model.loadInitialData();
        }
    });

    // Subscribe to sysinfo events — hand every sample to the reducer.
    // The reducer joins a short gap between two real samples and breaks the
    // series at a long one. No gap-triggered reload here: that caused
    // the chart to blank under load and created a drop/reload feedback loop.
    createEffect(() => {
        const cn = connName();
        const unsubFn = onMuxEvent(
            WpsEvent.SysInfo,
            (data) => {
                if (model.loadingAtom()) return;
                const dataItem = convertTimeSeriesToDataItem(data);
                if (dataItem == null) return;
                model.appendData(dataItem);
            },
            cn
        );
        console.log("subscribe to sysinfo", cn);
        onCleanup(() => unsubFn());
    });

    // Keep the chart visible while a reload is in flight — the reducer holds
    // the previous data until resetData() replaces it with fresh history.
    return (
        <Show when={connStatus()?.status == "connected"}>
            <SysinfoViewInner blockId={blockId} model={model} />
        </Show>
    );
}

// How often the charts take new samples. They used to rebuild an SVG each
// time, which at 1 Hz cost ~13% sustained GPU process CPU, so this was 2 s.
// uPlot redraws its canvas instead (sysinfo-plot.tsx), so the charts follow
// the default 1 Hz sampling.
const CHART_UPDATE_INTERVAL_MS = 1000;

function SysinfoViewInner(props: SysinfoViewProps): JSX.Element {
    const { model } = props;

    // The charts take the model's samples on a timer (CHART_UPDATE_INTERVAL_MS)
    // rather than on every event, so a burst (a backfill, a resumed lane) is one
    // redraw, not many.
    const [plotData, setPlotData] = createSignal(model.dataAtom());
    // Paused: the charts hold what they show while samples keep arriving.
    const [paused, setPaused] = createSignal(false);
    onMount(() => {
        // Skipped while the window is being resized: rebuilding the chart reads
        // layout, which mid-resize forced the frame's whole layout early
        // (ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md §2.3). The next tick
        // after the resize catches up.
        const id = setInterval(() => {
            if (!windowResizing() && !paused()) setPlotData(model.dataAtom());
        }, CHART_UPDATE_INTERVAL_MS);
        onCleanup(() => clearInterval(id));
    });
    // Take the samples at once when a load (first, or after a reconnect)
    // finishes, rather than at the next tick: a pane that mounted before its
    // history arrived would otherwise sit empty for up to a whole
    // CHART_UPDATE_INTERVAL_MS. Only `loadingAtom` is tracked; tracking
    // `dataAtom` too would take every sample as it comes and undo the timer.
    createEffect(
        on(
            () => model.loadingAtom(),
            (loading) => {
                if (!loading) setPlotData(model.dataAtom());
            }
        )
    );
    const yvals = createMemo(() => model.metrics());
    const plotMeta = createMemo(() => model.plotMetaAtom());
    const targetLen = createMemo(() => model.numPoints() + 1);
    const intervalSecs = createMemo(() => model.intervalSecsAtom());

    const title = createMemo(() => yvals().length > 1);
    // The "CPU + Mem + Net" plot type stacks as a single column of 3
    // full-width rows instead of falling into the 2-column grid below: at 2
    // columns, 3 panels wrap to a 2+1 layout that leaves the second row
    // half empty. Keyed off the plot type itself, NOT `yvals().length === 3`
    // (reagentx P2 on PR #2638) — "All CPU" also produces exactly 3 panels
    // on any machine with exactly 3 detected cores, which would otherwise
    // incorrectly force that view into the single-column layout meant only
    // for this one plot type.
    const cols1 = createMemo(() => model.plotTypeSelectedAtom() === "CPU + Mem + Net");
    const cols2 = createMemo(() => yvals().length > 2 && !cols1());

    const overview = createMemo(() => model.plotTypeSelectedAtom() === "Overview");

    return (
        <Show
            when={!overview()}
            fallback={
                <Overview
                    plotData={plotData()}
                    targetLen={targetLen()}
                    intervalSecs={intervalSecs()}
                    windowSecs={model.windowSecs()}
                    onWindow={(secs) => model.setWindow(secs)}
                    paused={paused()}
                    onPause={(p) => {
                        setPaused(p);
                        if (!p) setPlotData(model.dataAtom());
                    }}
                    focus={model.focus()}
                    onFocus={(area) => model.setFocus(area)}
                    syncKey={`sysinfo-${model.blockId}`}
                />
            }
        >
            <div class="flex flex-col flex-grow mb-0 overflow-y-auto">
                <div
                    class={clsx("w-full h-full grid grid-rows-[repeat(auto-fit,minmax(100px,1fr))] gap-[10px]", {
                        "grid-cols-1": cols1(),
                        "grid-cols-2": cols2(),
                    })}
                >
                    <For each={yvals()}>
                        {(yval) => (
                            <SingleLinePlot
                                plotData={plotData()}
                                yval={yval}
                                yvalMeta={plotMeta().get(yval)}
                                blockId={model.blockId}
                                defaultColor={"var(--accent-color)"}
                                title={title()}
                                targetLen={targetLen()}
                                intervalSecs={intervalSecs()}
                                syncKey={`sysinfo-${model.blockId}`}
                            />
                        )}
                    </For>
                </div>
            </div>
        </Show>
    );
}

export { SysinfoView };
