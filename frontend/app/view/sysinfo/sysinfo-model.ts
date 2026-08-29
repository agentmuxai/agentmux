// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, WOS } from "@/store/global";
import * as util from "@/util/util";
import { createMemo } from "solid-js";
import type { SignalAtom } from "@/util/util";
import { createSignalAtom } from "@/util/util";

import { getConnStatusAtom } from "@/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { WpsEvent } from "@/app/store/wps-events";
import { TabRpcClient } from "@/app/store/rpc-util";

import { DataItem, DefaultNumPoints, DefaultPlotMeta, PlotTypes } from "./sysinfo-types";
import { convertWaveEventToDataItem, getGapThresholdMs } from "./sysinfo-util";

// ---------------------------------------------------------------------------
// Sample ring-buffer reducer
// ---------------------------------------------------------------------------

function makeBlankItem(template: DataItem, ts: number): DataItem {
    const blank: DataItem = { ts };
    for (const key in template) {
        if (key !== "ts") (blank as any)[key] = NaN;
    }
    (blank as any).blank = 1;
    return blank;
}

type SampleAction =
    | { type: "RESET"; items: DataItem[]; intervalSecs: number; numPoints: number }
    | { type: "APPEND"; item: DataItem; intervalSecs: number; numPoints: number };

/**
 * How far ahead of the newest sample a point must sit before the jump counts
 * as a real BACKWARDS clock step rather than ordinary slew.
 *
 * This governs only whether a visible break is drawn. It is NOT a retention
 * tolerance: which points may remain is decided separately and strictly
 * (`d.ts <= newest`), because any retained point ahead of the newest sample
 * leaves the series non-monotonic regardless of how it got there. Sharing one
 * threshold for both jobs was reagentx's P1 on PR #2832.
 *
 * Sample timestamps are wall-clock (`SystemTime::now()` in
 * `agentmux-srv/src/backend/sysinfo.rs`), so they can jump backwards on an NTP
 * correction, a manual clock set, or a VM resume. Both of this reducer's
 * original trim/gap rules assumed time only moves forward, which made a
 * backwards step permanently corrupt the series — see this module's tests and
 * `frontend/app/statusbar/backend-uptime.ts` for the live 2081 -> 2026 case.
 *
 * Reuses the same threshold as a "true break" so slew (a sub-tick nudge) keeps
 * the history while a real step clears it.
 */
function staleAheadCutoff(newestTs: number, intervalSecs: number): number {
    return newestTs + getGapThresholdMs(intervalSecs);
}

/**
 * Keep only a strictly increasing run, anchored on the NEWEST sample: walk
 * backwards from the end and drop any earlier entry that isn't strictly older
 * than the one after it.
 *
 * Needed because persisted history arrives in ring (insertion) order, not
 * timestamp order — so a clock step leaves earlier slots holding LATER
 * timestamps. Bracketing such a seam with `prev.ts + 1` / `cur.ts - 1` blanks
 * cannot repair that (the blanks inherit the same inversion), which was
 * codex's P2 on PR #2832: history `[10000, 8000]` emitted
 * `10000, 10001, 7999, 8000`. Dropping the superseded segment is the only
 * monotonic answer, and anchoring on the newest sample is what makes it the
 * *right* segment to drop — the post-step samples are the live ones.
 */
function keepStrictlyIncreasing(items: DataItem[]): DataItem[] {
    const kept: DataItem[] = [];
    let limit = Infinity;
    for (let i = items.length - 1; i >= 0; i--) {
        if (items[i].ts < limit) {
            kept.push(items[i]);
            limit = items[i].ts;
        }
    }
    return kept.reverse();
}

export function sampleReducer(state: DataItem[], action: SampleAction): DataItem[] {
    if (action.type === "RESET") {
        const { items, intervalSecs, numPoints } = action;
        if (items.length === 0) return [];
        const targetLen = numPoints + 1;
        const gapThreshold = getGapThresholdMs(intervalSecs);
        // Ring order is insertion order, so the LAST item is the newest by
        // arrival even when the clock stepped mid-history; anchor both bounds
        // on it and drop anything stamped implausibly far ahead of it.
        const latestTs = items[items.length - 1].ts;
        const cutoffTs = latestTs - intervalSecs * 1000 * targetLen;
        // Window-trim first, then enforce monotonicity — `keepStrictlyIncreasing`
        // subsumes "drop anything ahead of the newest sample" (reagentx P1)
        // and also repairs out-of-order ring content in general (codex P2).
        const filtered = keepStrictlyIncreasing(items.filter((d) => d.ts >= cutoffTs));
        if (filtered.length === 0) return [];
        const template = filtered[filtered.length - 1];
        const result: DataItem[] = [];
        if (filtered[0].ts > cutoffTs) {
            result.push(makeBlankItem(template, cutoffTs));
            result.push(makeBlankItem(template, filtered[0].ts - 1));
        }
        result.push(filtered[0]);
        for (let i = 1; i < filtered.length; i++) {
            const prev = filtered[i - 1];
            const cur = filtered[i];
            // A backwards seam can't reach here: `keepStrictlyIncreasing`
            // above guarantees `cur.ts > prev.ts`, so this only ever sees a
            // genuine forward gap.
            if (cur.ts - prev.ts > gapThreshold) {
                result.push(makeBlankItem(template, prev.ts + 1));
                result.push(makeBlankItem(template, cur.ts - 1));
            }
            result.push(cur);
        }
        return result;
    }

    if (action.type === "APPEND") {
        const { item, intervalSecs, numPoints } = action;
        const intervalMs = intervalSecs * 1000;

        // TWO SEPARATE QUESTIONS, deliberately not sharing a threshold
        // (reagentx P1 on PR #2832 — the first version of this fix used one
        // for both and reintroduced the very defect it set out to remove):
        //
        //   1. WHICH POINTS MAY REMAIN — anything stamped ahead of this sample
        //      must go, with no tolerance. The window trim below can never
        //      remove them (their ts exceeds any cutoff derived from the new,
        //      earlier ts, so `d.ts >= cutoffTs` stays true forever), and
        //      keeping even one leaves the series non-monotonic. Using the
        //      step threshold here let a point in `(item.ts, item.ts +
        //      threshold]` survive and sit AFTER the new sample in x.
        //      Strict `<`, not `<=`: a buffered point sharing this sample's
        //      exact ts is superseded by it, and keeping both would put two
        //      points on the same x.
        const fresh = state.filter((d) => d.ts < item.ts);
        //
        //   2. WAS THIS A STEP, OR JUST SLEW — only a jump past the gap
        //      threshold is a real discontinuity worth drawing a break for. A
        //      sub-tick nudge silently drops the one overtaken point (rule 1
        //      still applies) rather than scarring the chart with a sentinel.
        const clockStepped = state.some((d) => d.ts > staleAheadCutoff(item.ts, intervalSecs));

        const cutoffTs = item.ts - intervalMs * (numPoints + 1);
        const trimmed = fresh.filter((d) => d.ts >= cutoffTs);
        const last = trimmed.length > 0 ? trimmed[trimmed.length - 1] : null;

        if (clockStepped) {
            // A real discontinuity: mark it so the line doesn't connect across
            // the seam. Both gap branches below test `gap > ...`, which a
            // negative gap can never satisfy — that's why this is handled here
            // rather than as another case inside them.
            //
            // The sentinel is only inserted when it fits strictly between the
            // two: `last.ts + 1` equals `item.ts` when the surviving point is
            // adjacent to it, and emitting both would put two samples on the
            // same x. The seam is invisible at 1ms anyway.
            if (last && last.ts + 1 < item.ts) {
                return [...trimmed, makeBlankItem(last, last.ts + 1), item];
            }
            return [...trimmed, item];
        }

        if (last) {
            const gap = item.ts - last.ts;
            if (gap > intervalMs * 1.5 && gap <= intervalMs * 3.5) {
                // 1–3 missed ticks: zero-order hold — extend the last value
                const steps = Math.round(gap / intervalMs) - 1;
                const held: DataItem[] = [];
                for (let i = 1; i <= steps; i++) {
                    held.push({ ...last, ts: last.ts + i * intervalMs });
                }
                return [...trimmed, ...held, item];
            } else if (gap > intervalMs * 3.5) {
                // True break (connection hiccup / long pause): NaN sentinel
                return [...trimmed, makeBlankItem(last, last.ts + 1), item];
            }
        }

        return [...trimmed, item];
    }

    return state;
}

// ---------------------------------------------------------------------------

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
    connStatus: () => ConnStatus;
    plotMetaAtom: SignalAtom<Map<string, TimeSeriesMeta>>;
    plotTypeSelectedAtom: () => string;
    intervalSecsAtom: () => number;

    private dispatch(action: SampleAction) {
        this.dataAtom._set(sampleReducer(this.dataAtom(), action));
    }

    resetData(items: DataItem[]) {
        try {
            this.dispatch({
                type: "RESET",
                items,
                intervalSecs: this.getConfiguredInterval(),
                numPoints: this.numPoints(),
            });
        } catch (e) {
            console.error("sysinfo: resetData error", e);
        }
    }

    appendData(item: DataItem) {
        try {
            this.dispatch({
                type: "APPEND",
                item,
                intervalSecs: this.getConfiguredInterval(),
                numPoints: this.numPoints(),
            });
        } catch (e) {
            console.error("sysinfo: appendData error", e);
        }
    }

    constructor(blockId: string, viewType: string) {
        this.viewType = viewType;
        this.blockId = blockId;
        this.blockAtom = WOS.getWaveObjectAtom<Block>(`block:${blockId}`);

        this.dataAtom = createSignalAtom<DataItem[]>([]);
        this.loadingAtom = createSignalAtom(true);
        this.plotMetaAtom = createSignalAtom(new Map(Object.entries(DefaultPlotMeta)));
        this.manageConnection = createMemo(() => true);

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

        this.viewName = createMemo(() => this.plotTypeSelectedAtom());

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
                event: WpsEvent.SysInfo,
                scope: connName,
                maxitems: numPoints,
            });
            if (initialData == null) return;
            const initialDataItems: DataItem[] = initialData.map(convertWaveEventToDataItem);
            this.resetData(initialDataItems);
        } catch (e) {
            console.log("Error loading initial data for sysinfo", e);
        } finally {
            this.loadingAtom._set(false);
        }
    }

    getSettingsMenuItems(): ContextMenuItem[] {
        const fullConfig = atoms.fullConfigAtom();
        const termThemes = fullConfig?.termthemes ?? {};
        const termThemeKeys = Object.keys(termThemes);
        const plotData = this.dataAtom();

        termThemeKeys.sort((a, b) => {
            return (termThemes[a]["display:order"] ?? 0) - (termThemes[b]["display:order"] ?? 0);
        });
        const fullMenu: ContextMenuItem[] = [];
        let submenu: ContextMenuItem[];
        if (plotData.length == 0) {
            submenu = [];
        } else {
            const currentlySelected = this.plotTypeSelectedAtom();
            submenu = Object.keys(PlotTypes).map((plotType) => {
                const dataTypes = PlotTypes[plotType](plotData[plotData.length - 1]);
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

        fullMenu.push({ label: "Plot Type", submenu: submenu });
        fullMenu.push({ type: "separator" });
        return fullMenu;
    }

    getBodyContextMenuItems(): ContextMenuItem[] {
        const plotData = this.dataAtom();
        if (plotData.length === 0) return [];

        const currentlySelected = this.plotTypeSelectedAtom();
        return Object.keys(PlotTypes).map((plotType): ContextMenuItem => ({
            label: plotType,
            type: "radio",
            checked: currentlySelected === plotType,
            click: async () => {
                const dataTypes = PlotTypes[plotType](plotData[plotData.length - 1]);
                await RpcApi.SetMetaCommand(TabRpcClient, {
                    oref: WOS.makeORef("block", this.blockId),
                    meta: { "graph:metrics": dataTypes, "sysinfo:type": plotType },
                });
            },
        }));
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
