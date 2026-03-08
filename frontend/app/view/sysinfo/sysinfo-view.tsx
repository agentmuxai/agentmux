// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { waveEventSubscribe } from "@/app/store/wps";
import { globalStore } from "@/store/global";
import clsx from "clsx";
import * as jotai from "jotai";
import * as React from "react";
import { OverlayScrollbarsComponent, OverlayScrollbarsComponentRef } from "overlayscrollbars-react";

import type { SysinfoViewModel } from "./sysinfo-model";
import { SingleLinePlot } from "./sysinfo-plot";
import type { DataItem } from "./sysinfo-types";
import { convertWaveEventToDataItem, getGapThresholdMs } from "./sysinfo-util";

type SysinfoViewProps = {
    blockId: string;
    model: SysinfoViewModel;
};

function SysinfoView({ model, blockId }: SysinfoViewProps) {
    const connName = jotai.useAtomValue(model.connection);
    const lastConnName = React.useRef(connName);
    const connStatus = jotai.useAtomValue(model.connStatus);
    const addContinuousData = jotai.useSetAtom(model.addContinuousDataAtom);
    const addContinuousDataRef = React.useRef(addContinuousData);
    addContinuousDataRef.current = addContinuousData;
    const loading = jotai.useAtomValue(model.loadingAtom);

    React.useEffect(() => {
        if (connStatus?.status != "connected") {
            return;
        }
        if (lastConnName.current !== connName) {
            lastConnName.current = connName;
            model.loadInitialData();
        }
    }, [connStatus.status, connName]);
    React.useEffect(() => {
        const unsubFn = waveEventSubscribe({
            eventType: "sysinfo",
            scope: connName,
            handler: (event) => {
                const loading = globalStore.get(model.loadingAtom);
                if (loading) {
                    return;
                }
                const dataItem = convertWaveEventToDataItem(event);
                if (dataItem == null) {
                    return;
                }
                const prevData: DataItem[] = globalStore.get(model.dataAtom);
                const prevLastTs = prevData[prevData.length - 1]?.ts ?? 0;
                const intervalSecs = globalStore.get(model.intervalSecsAtom);
                const gapThreshold = getGapThresholdMs(intervalSecs);
                if (dataItem.ts - prevLastTs > gapThreshold) {
                    model.loadInitialData();
                } else {
                    addContinuousDataRef.current(dataItem);
                }
            },
        });
        console.log("subscribe to sysinfo", connName);
        return () => {
            unsubFn();
        };
    }, [connName]);
    if (connStatus?.status != "connected") {
        return null;
    }
    if (loading) {
        return null;
    }
    return <SysinfoViewInner key={connStatus?.connection ?? "local"} blockId={blockId} model={model} />;
}

const SysinfoViewInner = React.memo(({ model }: SysinfoViewProps) => {
    const plotData = jotai.useAtomValue(model.dataAtom);
    const yvals = jotai.useAtomValue(model.metrics);
    const plotMeta = jotai.useAtomValue(model.plotMetaAtom);
    const osRef = React.useRef<OverlayScrollbarsComponentRef>(null);
    const targetLen = jotai.useAtomValue(model.numPoints) + 1;
    const intervalSecs = jotai.useAtomValue(model.intervalSecsAtom);
    let title = false;
    let cols2 = false;
    if (yvals.length > 1) {
        title = true;
    }
    if (yvals.length > 2) {
        cols2 = true;
    }

    return (
        <OverlayScrollbarsComponent
            ref={osRef}
            className="flex flex-col flex-grow mb-0 overflow-y-auto"
            options={{ scrollbars: { autoHide: "leave" } }}
        >
            <div className={clsx("w-full h-full grid grid-rows-[repeat(auto-fit,minmax(100px,1fr))] gap-[10px]", { "grid-cols-2": cols2 })}>
                {yvals.map((yval, idx) => {
                    return (
                        <SingleLinePlot
                            key={`plot-${model.blockId}-${yval}`}
                            plotData={plotData}
                            yval={yval}
                            yvalMeta={plotMeta.get(yval)}
                            blockId={model.blockId}
                            defaultColor={"var(--accent-color)"}
                            title={title}
                            targetLen={targetLen}
                            intervalSecs={intervalSecs}
                        />
                    );
                })}
            </div>
        </OverlayScrollbarsComponent>
    );
});

export { SysinfoView };
