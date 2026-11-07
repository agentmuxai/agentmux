// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The per-core CPU panel's state: the latest per-core readings from the local `sysinfo` event, which layout fits
 * them (rows, cells or heatmap), the heatmap's square size, the hovered or focused core that drives the readout
 * line, and the heatmap's arrow-key navigation.
 */

import { createEffect, createMemo, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";

export interface Core {
    idx: number;
    pct: number;
}

export type Tier = "rows" | "cells" | "heat";

// Tier thresholds (tunable). ≤ROW_MAX → rows; ≤CELL_MAX → cells; else heatmap.
const ROW_MAX = 16;
const CELL_MAX = 64;

// Heatmap geometry.
const HEAT_CONTENT_WIDTH = 340; // px available to the square grid
const HEAT_TARGET_HEIGHT = 300; // grow rows up to here before squares shrink / scroll
const HEAT_GAP = 2;
const SQ_MIN = 10;
const SQ_MAX = 22;

const CPU_KEY = /^cpu:(\d+)$/;

export type CpuCoresState = {
    cores: Accessor<Core[]>;
    aggregate: Accessor<number>;
    tier: () => Tier;
    heat: Accessor<{ cols: number; sq: number }>;
    panelWidth: () => number;
    readout: () => string;
    setActiveIdx: (idx: number | null) => void;
    rovingIdx: Accessor<number>;
    setRovingIdx: (idx: number) => void;
    setHeatGridRef: (el: HTMLDivElement) => void;
    onHeatKeyDown: (e: KeyboardEvent) => void;
};

/**
 * Heatmap sizing: pick the LARGEST square (≤ SQ_MAX) at which all cores fit
 * within the target height, packing as many columns as the fixed width
 * allows at that size. Squares stay big while they fit (more legible), then
 * shrink as the count grows; only once even SQ_MIN overflows does the height
 * cap take over and the grid scrolls. With W=340/H=300: ~64–128 cores stay
 * at SQ_MAX (22px), ~256 settle around the mid-teens (~17px), and the
 * SQ_MIN (10px) floor isn't reached until ~600+ cores, beyond which it scrolls.
 */
function heatSizing(coreCount: number): { cols: number; sq: number } {
    const n = Math.max(1, coreCount);
    for (let s = SQ_MAX; s > SQ_MIN; s--) {
        const cols = Math.max(1, Math.floor((HEAT_CONTENT_WIDTH + HEAT_GAP) / (s + HEAT_GAP)));
        const rows = Math.ceil(n / cols);
        if (rows * (s + HEAT_GAP) - HEAT_GAP <= HEAT_TARGET_HEIGHT) {
            return { cols, sq: s };
        }
    }
    // Nothing fits at >SQ_MIN — use SQ_MIN with max columns; scroll handles it.
    const cols = Math.max(1, Math.floor((HEAT_CONTENT_WIDTH + HEAT_GAP) / (SQ_MIN + HEAT_GAP)));
    return { cols, sq: SQ_MIN };
}

/** Reads the local `sysinfo` event while mounted: the per-core readings in index order, and the aggregate. */
function watchSysInfo(setCores: (cores: Core[]) => void, setAggregate: (pct: number) => void): void {
    onMount(() => {
        const unsub = onMuxEvent(WpsEvent.SysInfo, (data) => {
                const vals = data.values;
                const next: Core[] = [];
                for (const key in vals) {
                    const m = CPU_KEY.exec(key);
                    if (m) next.push({ idx: Number(m[1]), pct: vals[key] ?? 0 });
                }
                next.sort((a, b) => a.idx - b.idx);
                setCores(next);
                setAggregate(vals["cpu"] ?? 0);
            }, "local");
        onCleanup(() => unsub?.());
    });
}

/** The panel's state, created in the order the readings, the hovered core and the roving tab stop are needed. */
export function createCpuCores(): CpuCoresState {
    const [cores, setCores] = createSignal<Core[]>([]);
    const [aggregate, setAggregate] = createSignal(0);
    // Hovered/focused core drives the readout line (cheaper + a11y-friendlier
    // than 128 simultaneous tooltips in heatmap mode). Stored by core *index*,
    // not the Core object — the sysinfo handler mints new objects each tick, so
    // an object ref would go stale; the index lets the readout re-derive the
    // live value from `cores()` and update while the user keeps hovering.
    const [activeIdx, setActiveIdx] = createSignal<number | null>(null);
    // Roving tabindex for the heatmap: exactly one square is in the tab order
    // (tabindex 0); the rest are -1, and arrow keys move focus. This is the
    // standard grid a11y pattern — without it, 128+ squares would each be a
    // tab stop. `rovingIdx` is a core index (== grid position, since cores are
    // a contiguous 0..N-1 run sorted by index).
    const [rovingIdx, setRovingIdx] = createSignal(0);
    let heatGridRef: HTMLDivElement | undefined;

    // Keep the roving index within range as the core count changes.
    createEffect(() => {
        const n = cores().length;
        if (n > 0 && rovingIdx() > n - 1) setRovingIdx(n - 1);
    });

    watchSysInfo(setCores, setAggregate);

    const tier = (): Tier => {
        const n = cores().length;
        if (n <= ROW_MAX) return "rows";
        if (n <= CELL_MAX) return "cells";
        return "heat";
    };

    const heat = createMemo(() => heatSizing(cores().length));

    const panelWidth = (): number => (tier() === "rows" ? 260 : 360);

    const readout = (): string => {
        const i = activeIdx();
        if (i != null) {
            const c = cores().find((x) => x.idx === i);
            if (c) return `Core ${c.idx} — ${Math.round(c.pct)}%`;
        }
        return `${cores().length} cores`;
    };

    // Arrow-key navigation across the heatmap grid (roving tabindex). Moves the
    // tab stop and DOM focus by ±1 (left/right) or ±cols (up/down), Home/End to
    // the ends; also drives the readout via setActiveIdx.
    const onHeatKeyDown = (e: KeyboardEvent) => {
        const n = cores().length;
        if (n === 0) return;
        const cols = heat().cols;
        const cur = Math.min(rovingIdx(), n - 1);
        let next = cur;
        switch (e.key) {
            case "ArrowRight": next = Math.min(n - 1, cur + 1); break;
            case "ArrowLeft": next = Math.max(0, cur - 1); break;
            case "ArrowDown": next = Math.min(n - 1, cur + cols); break;
            case "ArrowUp": next = Math.max(0, cur - cols); break;
            case "Home": next = 0; break;
            case "End": next = n - 1; break;
            default: return;
        }
        e.preventDefault();
        setRovingIdx(next);
        setActiveIdx(next);
        const el = heatGridRef?.querySelector<HTMLElement>(`[data-core-idx="${next}"]`);
        el?.focus();
    };

    return {
        cores,
        aggregate,
        tier,
        heat,
        panelWidth,
        readout,
        setActiveIdx,
        rovingIdx,
        setRovingIdx,
        setHeatGridRef: (el) => {
            heatGridRef = el;
        },
        onHeatKeyDown,
    };
}
