// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import clsx from "clsx";
import type { JSX } from "solid-js";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";

import type { SysinfoViewModel } from "./sysinfo-model";
import { SingleLinePlot } from "./sysinfo-plot";
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
    // Sync immediately whenever the initial (or a reconnect-triggered) load
    // finishes, instead of waiting for the next throttle tick. `plotData`'s
    // initial value above is a one-time snapshot taken at mount — if the
    // pane mounts before loadInitialData()'s RPC resolves (the common
    // case), it captures `[]`, and without this effect the chart would sit
    // blank until the interval above happened to fire, up to
    // CHART_UPDATE_INTERVAL_MS (2s) after the real data was already ready.
    // That interval exists to throttle STEADY-STATE repaints, not to gate
    // the first paint — this was the actual cause of a slow-feeling first
    // load, not a missing loading indicator or slow RPC.
    //
    // `on()` scopes tracking to loadingAtom() ONLY — model.dataAtom() is
    // read for its current value but deliberately NOT tracked, or this
    // effect would re-fire on every ~1Hz sample and defeat the throttle
    // above entirely (the exact ~13% sustained GPU cost the throttle was
    // added to fix, see CHART_UPDATE_INTERVAL_MS's comment).
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
    // Exactly 3 metrics (the "CPU + Mem + Net" plot type) stacks as a single
    // column of 3 full-width rows instead of falling into the 2-column grid
    // below: at 2 columns, 3 panels wrap to a 2+1 layout that leaves the
    // second row half empty. >3 (the "All CPU" per-core plot type, up to 32
    // panels) keeps 2 columns — this change is scoped to the specific case
    // that was actually losing space.
    const cols1 = createMemo(() => yvals().length === 3);
    const cols2 = createMemo(() => yvals().length > 2 && !cols1());

    return (
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
                        />
                    )}
                </For>
            </div>
        </div>
    );
}

export { SysinfoView };
