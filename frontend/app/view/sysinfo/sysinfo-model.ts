// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { atoms, globalStore, WOS } from "@/store/global";
import * as util from "@/util/util";
import * as jotai from "jotai";

import { getConnStatusAtom } from "@/store/global";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";

import { DataItem, DefaultNumPoints, DefaultPlotMeta, PlotTypes } from "./sysinfo-types";
import { convertWaveEventToDataItem, getGapThresholdMs } from "./sysinfo-util";

class SysinfoViewModel implements ViewModel {
    viewType: string;
    blockAtom: jotai.Atom<Block>;
    termMode: jotai.Atom<string>;
    htmlElemFocusRef: React.RefObject<HTMLInputElement>;
    blockId: string;
    viewIcon: jotai.Atom<string>;
    viewText: jotai.Atom<string>;
    viewName: jotai.Atom<string>;
    dataAtom: jotai.PrimitiveAtom<Array<DataItem>>;
    addInitialDataAtom: jotai.WritableAtom<unknown, [DataItem[]], void>;
    addContinuousDataAtom: jotai.WritableAtom<unknown, [DataItem], void>;
    incrementCount: jotai.WritableAtom<unknown, [], Promise<void>>;
    loadingAtom: jotai.PrimitiveAtom<boolean>;
    numPoints: jotai.Atom<number>;
    metrics: jotai.Atom<string[]>;
    connection: jotai.Atom<string>;
    manageConnection: jotai.Atom<boolean>;
    filterOutNowsh: jotai.Atom<boolean>;
    connStatus: jotai.Atom<ConnStatus>;
    plotMetaAtom: jotai.PrimitiveAtom<Map<string, TimeSeriesMeta>>;
    endIconButtons: jotai.Atom<IconButtonDecl[]>;
    plotTypeSelectedAtom: jotai.Atom<string>;
    intervalSecsAtom: jotai.Atom<number>;

    constructor(blockId: string, viewType: string) {
        this.viewType = viewType;
        this.blockId = blockId;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);
        this.addInitialDataAtom = jotai.atom(null, (get, set, points) => {
            const targetLen = get(this.numPoints) + 1;
            const intervalSecs = this.getConfiguredInterval();
            const gapThreshold = getGapThresholdMs(intervalSecs);
            try {
                const newDataRaw = [...points];
                if (newDataRaw.length == 0) {
                    return;
                }
                const latestItemTs = newDataRaw[newDataRaw.length - 1]?.ts ?? 0;
                const cutoffTs = latestItemTs - intervalSecs * 1000 * targetLen;
                const blankItemTemplate = { ...newDataRaw[newDataRaw.length - 1] };
                for (const key in blankItemTemplate) {
                    blankItemTemplate[key] = NaN;
                }

                const newDataFiltered = newDataRaw.filter((dataItem) => dataItem.ts >= cutoffTs);
                if (newDataFiltered.length == 0) {
                    return;
                }
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
                set(this.dataAtom, newDataWithGaps);
            } catch (e) {
                console.log("Error adding data to sysinfo", e);
            }
        });
        this.addContinuousDataAtom = jotai.atom(null, (get, set, newPoint) => {
            const targetLen = get(this.numPoints) + 1;
            const intervalSecs = this.getConfiguredInterval();
            let data = get(this.dataAtom);
            try {
                const latestItemTs = newPoint?.ts ?? 0;
                const cutoffTs = latestItemTs - intervalSecs * 1000 * targetLen;
                data.push(newPoint);
                const newData = data.filter((dataItem) => dataItem.ts >= cutoffTs);
                set(this.dataAtom, newData);
            } catch (e) {
                console.log("Error adding data to sysinfo", e);
            }
        });
        this.plotMetaAtom = jotai.atom(new Map(Object.entries(DefaultPlotMeta)));
        this.manageConnection = jotai.atom(true);
        this.filterOutNowsh = jotai.atom(true);
        this.loadingAtom = jotai.atom(true);
        this.numPoints = jotai.atom((get) => {
            const blockData = get(this.blockAtom);
            const metaNumPoints = blockData?.meta?.["graph:numpoints"];
            if (metaNumPoints == null || metaNumPoints <= 0) {
                return DefaultNumPoints;
            }
            return metaNumPoints;
        });
        this.metrics = jotai.atom((get) => {
            let plotType = get(this.plotTypeSelectedAtom);
            const plotData = get(this.dataAtom);
            try {
                const metrics = PlotTypes[plotType](plotData[plotData.length - 1]);
                if (metrics == null || !Array.isArray(metrics)) {
                    return ["cpu"];
                }
                return metrics;
            } catch (e) {
                return ["cpu"];
            }
        });
        this.plotTypeSelectedAtom = jotai.atom((get) => {
            const blockData = get(this.blockAtom);
            const plotType = blockData?.meta?.["sysinfo:type"];
            if (plotType == null || typeof plotType != "string") {
                return "CPU";
            }
            return plotType;
        });
        this.viewIcon = jotai.atom((get) => {
            return "chart-line"; // should not be hardcoded
        });
        this.viewName = jotai.atom((get) => {
            return get(this.plotTypeSelectedAtom);
        });
        this.incrementCount = jotai.atom(null, async (get, set) => {
            const meta = get(this.blockAtom).meta;
            const count = meta.count ?? 0;
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", this.blockId),
                meta: { count: count + 1 },
            });
        });
        this.connection = jotai.atom((get) => {
            const blockData = get(this.blockAtom);
            const connValue = blockData?.meta?.connection;
            if (util.isBlank(connValue)) {
                return "local";
            }
            return connValue;
        });
        this.dataAtom = jotai.atom([]);
        this.loadInitialData();
        this.connStatus = jotai.atom((get) => {
            const blockData = get(this.blockAtom);
            const connName = blockData?.meta?.connection;
            const connAtom = getConnStatusAtom(connName);
            return get(connAtom);
        });
        this.intervalSecsAtom = jotai.atom((get) => {
            const fullConfig = get(atoms.fullConfigAtom);
            const val = fullConfig?.settings?.["telemetry:interval"];
            if (val == null || val <= 0) {
                return 1.0;
            }
            return val as number;
        });
    }

    /** Read the configured telemetry interval from settings (default 1.0s). */
    getConfiguredInterval(): number {
        return globalStore.get(this.intervalSecsAtom);
    }

    get viewComponent(): ViewComponent {
        return null; // set by the view module to avoid circular import
    }

    async loadInitialData() {
        globalStore.set(this.loadingAtom, true);
        try {
            const numPoints = globalStore.get(this.numPoints);
            const connName = globalStore.get(this.connection);
            const initialData = await RpcApi.EventReadHistoryCommand(TabRpcClient, {
                event: "sysinfo",
                scope: connName,
                maxitems: numPoints,
            });
            if (initialData == null) {
                return;
            }
            const initialDataItems: DataItem[] = initialData.map(convertWaveEventToDataItem);
            globalStore.set(this.addInitialDataAtom, initialDataItems);
        } catch (e) {
            console.log("Error loading initial data for sysinfo", e);
        } finally {
            globalStore.set(this.loadingAtom, false);
        }
    }

    getSettingsMenuItems(): ContextMenuItem[] {
        const fullConfig = globalStore.get(atoms.fullConfigAtom);
        const termThemes = fullConfig?.termthemes ?? {};
        const termThemeKeys = Object.keys(termThemes);
        const plotData = globalStore.get(this.dataAtom);

        termThemeKeys.sort((a, b) => {
            return (termThemes[a]["display:order"] ?? 0) - (termThemes[b]["display:order"] ?? 0);
        });
        const fullMenu: ContextMenuItem[] = [];
        let submenu: ContextMenuItem[];
        if (plotData.length == 0) {
            submenu = [];
        } else {
            submenu = Object.keys(PlotTypes).map((plotType) => {
                const dataTypes = PlotTypes[plotType](plotData[plotData.length - 1]);
                const currentlySelected = globalStore.get(this.plotTypeSelectedAtom);
                const menuItem: ContextMenuItem = {
                    label: plotType,
                    type: "radio",
                    checked: currentlySelected == plotType,
                    click: async () => {
                        await RpcApi.SetMetaCommand(TabRpcClient, {
                            oref: WOS.makeORef("block", this.blockId),
                            meta: { "graph:metrics": dataTypes, "sysinfo:type": plotType },
                        });
                    },
                };
                return menuItem;
            });
        }

        fullMenu.push({
            label: "Plot Type",
            submenu: submenu,
        });
        fullMenu.push({ type: "separator" });
        return fullMenu;
    }

    getDefaultData(): DataItem[] {
        const numPoints = globalStore.get(this.numPoints);
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
