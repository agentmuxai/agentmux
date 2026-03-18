// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { atoms, WOS } from "@/store/global";
import * as util from "@/util/util";
import { createMemo } from "solid-js";
import type { SignalAtom } from "@/util/util";
import { createSignalAtom } from "@/util/util";

import { getConnStatusAtom } from "@/store/global";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";

import { DataItem, DefaultNumPoints, DefaultPlotMeta, PlotTypes } from "./sysinfo-types";
import { convertWaveEventToDataItem, getGapThresholdMs } from "./sysinfo-util";

class SysinfoViewModel implements ViewModel {
    viewType: string;
    blockAtom: () => Block;
    blockId: string;
    viewIcon: () => string;
    viewText: () => string;
    viewName: () => string;
    dataAtom: SignalAtom<Array<DataItem>>;
    loadingAtom: SignalAtom<boolean>;
    numPoints: () => number;
    metrics: () => string[];
    connection: () => string;
    manageConnection: () => boolean;
    filterOutNowsh: () => boolean;
    connStatus: () => ConnStatus;
    plotMetaAtom: SignalAtom<Map<string, TimeSeriesMeta>>;
    plotTypeSelectedAtom: () => string;
    intervalSecsAtom: () => number;

    // Writable actions (plain methods replacing jotai write-only atoms)
    addInitialData(points: DataItem[]) {
        const targetLen = this.numPoints() + 1;
        const intervalSecs = this.getConfiguredInterval();
        const gapThreshold = getGapThresholdMs(intervalSecs);
        try {
            const newDataRaw = [...points];
            if (newDataRaw.length == 0) return;
            const latestItemTs = newDataRaw[newDataRaw.length - 1]?.ts ?? 0;
            const cutoffTs = latestItemTs - intervalSecs * 1000 * targetLen;
            const blankItemTemplate = { ...newDataRaw[newDataRaw.length - 1] };
            for (const key in blankItemTemplate) {
                (blankItemTemplate as any)[key] = NaN;
            }
            const newDataFiltered = newDataRaw.filter((dataItem) => dataItem.ts >= cutoffTs);
            if (newDataFiltered.length == 0) return;
            const newDataWithGaps: Array<DataItem> = [];
            if (newDataFiltered[0].ts > cutoffTs) {
                const blankItemStart = { ...blankItemTemplate, ts: cutoffTs };
                const blankItemEnd = { ...blankItemTemplate, ts: newDataFiltered[0].ts - 1 };
                newDataWithGaps.push(blankItemStart);
                newDataWithGaps.push(blankItemEnd);
            }
            newDataWithGaps.push(newDataFiltered[0]);
            for (let i = 1; i < newDataFiltered.length; i++) {
                const prevIdxItem = newDataFiltered[i - 1];
                const curIdxItem = newDataFiltered[i];
                const timeDiff = curIdxItem.ts - prevIdxItem.ts;
                if (timeDiff > gapThreshold) {
                    const blankItemStart = { ...blankItemTemplate, ts: prevIdxItem.ts + 1, blank: 1 };
                    const blankItemEnd = { ...blankItemTemplate, ts: curIdxItem.ts - 1, blank: 1 };
                    newDataWithGaps.push(blankItemStart);
                    newDataWithGaps.push(blankItemEnd);
                }
                newDataWithGaps.push(curIdxItem);
            }
            this.dataAtom._set(newDataWithGaps);
        } catch (e) {
            console.log("Error adding data to sysinfo", e);
        }
    }

    addContinuousData(newPoint: DataItem) {
        const targetLen = this.numPoints() + 1;
        const intervalSecs = this.getConfiguredInterval();
        let data = this.dataAtom();
        try {
            const latestItemTs = newPoint?.ts ?? 0;
            const cutoffTs = latestItemTs - intervalSecs * 1000 * targetLen;
            data.push(newPoint);
            const newData = data.filter((dataItem) => dataItem.ts >= cutoffTs);
            this.dataAtom._set(newData);
        } catch (e) {
            console.log("Error adding data to sysinfo", e);
        }
    }

    constructor(blockId: string, viewType: string) {
        this.viewType = viewType;
        this.blockId = blockId;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);

        this.dataAtom = createSignalAtom<DataItem[]>([]);
        this.loadingAtom = createSignalAtom(true);
        this.plotMetaAtom = createSignalAtom(new Map(Object.entries(DefaultPlotMeta)));
        this.manageConnection = createMemo(() => false);
        this.filterOutNowsh = createMemo(() => true);

        this.numPoints = createMemo(() => {
            const fullConfig = atoms.fullConfigAtom();
            const settingsNumPoints = fullConfig?.settings?.["telemetry:numpoints"];
            if (settingsNumPoints != null && settingsNumPoints > 0) {
                return Math.max(30, Math.min(1024, settingsNumPoints));
            }
            const blockData = this.blockAtom();
            const metaNumPoints = blockData?.meta?.["graph:numpoints"];
            if (metaNumPoints == null || metaNumPoints <= 0) return DefaultNumPoints;
            return metaNumPoints;
        });

        this.plotTypeSelectedAtom = createMemo(() => {
            const blockData = this.blockAtom();
            const plotType = blockData?.meta?.["sysinfo:type"];
            if (plotType == null || typeof plotType != "string") return "CPU";
            return plotType;
        });

        this.metrics = createMemo(() => {
            const plotType = this.plotTypeSelectedAtom();
            const plotData = this.dataAtom();
            try {
                const metrics = PlotTypes[plotType](plotData[plotData.length - 1]);
                if (metrics == null || !Array.isArray(metrics)) return ["cpu"];
                return metrics;
            } catch (e) {
                return ["cpu"];
            }
        });

        this.viewIcon = createMemo(() => "chart-line");

        this.viewName = createMemo(() => {
            const plotType = this.plotTypeSelectedAtom();
            if (plotType === "Mem") return "Memory";
            if (plotType === "Disk I/O") return "Disk";
            return plotType;
        });

        this.viewText = createMemo(() => "");

        this.connection = createMemo(() => {
            const blockData = this.blockAtom();
            const connValue = blockData?.meta?.connection;
            if (util.isBlank(connValue)) return "local";
            return connValue;
        });

        this.connStatus = createMemo(() => {
            const blockData = this.blockAtom();
            const connName = blockData?.meta?.connection;
            const connAtom = getConnStatusAtom(connName);
            return connAtom();
        });

        this.intervalSecsAtom = createMemo(() => {
            const fullConfig = atoms.fullConfigAtom();
            const val = fullConfig?.settings?.["telemetry:interval"];
            if (val == null || val <= 0) return 1.0;
            return val as number;
        });

        this.loadInitialData();
    }

    get viewComponent(): ViewComponent {
        return null; // set by the view module to avoid circular import
    }

    getConfiguredInterval(): number {
        return this.intervalSecsAtom();
    }

    async loadInitialData() {
        this.loadingAtom._set(true);
        try {
            const numPoints = this.numPoints();
            const connName = this.connection();
            const initialData = await RpcApi.EventReadHistoryCommand(TabRpcClient, {
                event: "sysinfo",
                scope: connName,
                maxitems: numPoints,
            });
            if (initialData == null) return;
            const initialDataItems: DataItem[] = initialData.map(convertWaveEventToDataItem);
            this.addInitialData(initialDataItems);
        } catch (e) {
            console.log("Error loading initial data for sysinfo", e);
        } finally {
            this.loadingAtom._set(false);
        }
    }

    getSettingsMenuItems(): ContextMenuItem[] {
        const plotData = this.dataAtom();
        const currentlySelected = this.plotTypeSelectedAtom();
        const coreTypes = ["CPU", "Mem", "Disk I/O"];

        const items: ContextMenuItem[] = coreTypes.map((plotType) => ({
            label: plotType === "Mem" ? "Memory" : plotType === "Disk I/O" ? "Disk" : plotType,
            type: "radio" as const,
            checked: currentlySelected === plotType,
            click: async () => {
                const dataItem = plotData.length > 0 ? plotData[plotData.length - 1] : ({} as DataItem);
                const dataTypes = PlotTypes[plotType](dataItem);
                await RpcApi.SetMetaCommand(TabRpcClient, {
                    oref: WOS.makeORef("block", this.blockId),
                    meta: { "graph:metrics": dataTypes, "sysinfo:type": plotType },
                });
            },
        }));

        return items;
    }

    getDefaultData(): DataItem[] {
        const numPoints = this.numPoints();
        const intervalSecs = this.getConfiguredInterval();
        const currentTime = Date.now() - intervalSecs * 1000;
        const points: DataItem[] = [];
        for (let i = numPoints; i > -1; i--) {
            points.push({ ts: currentTime - i * intervalSecs * 1000 });
        }
        return points;
    }
}

export { SysinfoViewModel };
