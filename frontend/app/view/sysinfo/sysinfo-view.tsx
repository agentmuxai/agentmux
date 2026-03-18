// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { waveEventSubscribe } from "@/app/store/wps";
import clsx from "clsx";
import { createEffect, createMemo, For, onCleanup, Show } from "solid-js";
import type { JSX } from "solid-js";

import type { SysinfoViewModel } from "./sysinfo-model";
import { SingleLinePlot } from "./sysinfo-plot";
import type { DataItem } from "./sysinfo-types";
import { convertWaveEventToDataItem, getGapThresholdMs } from "./sysinfo-util";

type SysinfoViewProps = {
    blockId: string;
    model: SysinfoViewModel;
};

function SysinfoView(props: SysinfoViewProps): JSX.Element {
    const { model, blockId } = props;
    const connName = createMemo(() => model.connection());
    const connStatus = createMemo(() => model.connStatus());
    const loading = createMemo(() => model.loadingAtom());

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

    // Subscribe to sysinfo events
    createEffect(() => {
        const cn = connName();
        const unsubFn = waveEventSubscribe({
            eventType: "sysinfo",
            scope: cn,
            handler: (event) => {
                if (model.loadingAtom()) return;
                const dataItem = convertWaveEventToDataItem(event);
                if (dataItem == null) return;
                const prevData: DataItem[] = model.dataAtom();
                const prevLastTs = prevData[prevData.length - 1]?.ts ?? 0;
                const intervalSecs = model.intervalSecsAtom();
                const gapThreshold = getGapThresholdMs(intervalSecs);
                if (dataItem.ts - prevLastTs > gapThreshold) {
                    model.loadInitialData();
                } else {
                    model.addContinuousData(dataItem);
                }
            },
        });
        console.log("subscribe to sysinfo", cn);
        onCleanup(() => unsubFn());
    });

    return (
        <Show when={connStatus()?.status == "connected" && !loading()}>
            <SysinfoViewInner blockId={blockId} model={model} />
        </Show>
    );
}

function SysinfoViewInner(props: SysinfoViewProps): JSX.Element {
    const { model } = props;
    const plotData = createMemo(() => model.dataAtom());
    const yvals = createMemo(() => model.metrics());
    const plotMeta = createMemo(() => model.plotMetaAtom());
    const targetLen = createMemo(() => model.numPoints() + 1);
    const intervalSecs = createMemo(() => model.intervalSecsAtom());

    const title = createMemo(() => true);
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
