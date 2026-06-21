// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import clsx from "clsx";
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";

import type { SysinfoViewModel } from "./sysinfo-model";
import { SingleLinePlot } from "./sysinfo-plot";
import type { DataItem } from "./sysinfo-types";
import { convertWaveEventToDataItem } from "./sysinfo-util";

type SysinfoViewProps = {
    blockId: string;
    model: SysinfoViewModel;
};

function SysinfoView(props: SysinfoViewProps): JSX.Element {
    const { model, blockId } = props;
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
    // The reducer handles missing ticks via zero-order hold (≤3 missed) or
    // a NaN sentinel (true break). No gap-triggered reload here: that caused
    // the chart to blank under load and created a drop/reload feedback loop.
    createEffect(() => {
        const cn = connName();
        const unsubFn = waveEventSubscribe({
            eventType: WpsEvent.SysInfo,
            scope: cn,
            handler: (event) => {
                if (model.loadingAtom()) return;
                const dataItem = convertWaveEventToDataItem(event);
                if (dataItem == null) return;
                model.appendData(dataItem);
            },
        });
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

// Chart rebuild interval. The chart shows 5-min history so a 2s lag is
// imperceptible, but the full 1Hz SVG rebuild (font hinting + Temporal
// API for every axis tick) caused ~13% sustained GPU process CPU.
const CHART_UPDATE_INTERVAL_MS = 2000;

function SysinfoViewInner(props: SysinfoViewProps): JSX.Element {
    const { model } = props;

    // Throttle chart data to avoid rebuilding the SVG on every sysinfo
    // event (default 1Hz). At 1Hz, full SVG recreation forced font
    // hinting + Temporal date formatting for every x-axis label each
    // second, causing ~13% sustained GPU process CPU. This throttle
    // reduces repaints to ~0.5Hz while the status-bar stats still
    // update at the full sysinfo rate (cheap text DOM updates).
    const [plotData, setPlotData] = createSignal(model.dataAtom());
    onMount(() => {
        const id = setInterval(() => setPlotData(model.dataAtom()), CHART_UPDATE_INTERVAL_MS);
        onCleanup(() => clearInterval(id));
    });
    const yvals = createMemo(() => model.metrics());
    const plotMeta = createMemo(() => model.plotMetaAtom());
    const targetLen = createMemo(() => model.numPoints() + 1);
    const intervalSecs = createMemo(() => model.intervalSecsAtom());

    const title = createMemo(() => yvals().length > 1);
    const cols2 = createMemo(() => yvals().length > 2);

    return (
        <div class="flex flex-col flex-grow mb-0 overflow-y-auto">
            <div
                class={clsx(
                    "w-full h-full grid grid-rows-[repeat(auto-fit,minmax(100px,1fr))] gap-[10px]",
                    { "grid-cols-2": cols2() }
                )}
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
                        />
                    )}
                </For>
            </div>
        </div>
    );
}

export { SysinfoView };
