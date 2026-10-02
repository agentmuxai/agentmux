// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CpuCoresPopover — click-opened panel anchored under the status-bar CPU
 * readout. Shows live per-core CPU usage. The backend already publishes both
 * the aggregate (`cpu`) and every per-core value (`cpu:0`..`cpu:N`) in the
 * `sysinfo`/`local` event, so this is a pure-frontend view.
 *
 * The layout is adaptive (SPEC_STATUSBAR_CPU_CORES_PANEL_2026_06_15.md §4.3):
 *   ≤16 cores → labeled rows (label + bar + %)
 *   17–64     → compact cells (index + % + mini bar)
 *   65+       → heatmap of computed-size squares, detail on hover/focus
 *
 * Renders through AnchoredPopover (positioning top-end, chrome zoom, dismiss,
 * the airspace cut).
 */

import { createEffect, createMemo, createSignal, Index, onCleanup, onMount, Show, type JSX } from "solid-js";
import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { cpuColor, loadColor } from "./cpu-color";

interface Core {
    idx: number;
    pct: number;
}

type Tier = "rows" | "cells" | "heat";

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

interface CpuCoresPopoverProps {
    /** The status bar's CPU readout. */
    anchor: PopoverAnchor;
    onClose: () => void;
}

export const CpuCoresPopover = (props: CpuCoresPopoverProps): JSX.Element => {
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

    onMount(() => {
        const unsub = muxEventSubscribe({
            eventType: WpsEvent.SysInfo,
            scope: "local",
            handler: (event) => {
                const vals = (event as MuxEvent)?.data?.values;
                if (vals == null) return;
                const next: Core[] = [];
                for (const key in vals) {
                    const m = CPU_KEY.exec(key);
                    if (m) next.push({ idx: Number(m[1]), pct: vals[key] ?? 0 });
                }
                next.sort((a, b) => a.idx - b.idx);
                setCores(next);
                setAggregate(vals["cpu"] ?? 0);
            },
        });
        onCleanup(() => unsub?.());
    });

    const tier = (): Tier => {
        const n = cores().length;
        if (n <= ROW_MAX) return "rows";
        if (n <= CELL_MAX) return "cells";
        return "heat";
    };

    // Heatmap sizing: pick the LARGEST square (≤ SQ_MAX) at which all cores fit
    // within the target height, packing as many columns as the fixed width
    // allows at that size. Squares stay big while they fit (more legible), then
    // shrink as the count grows; only once even SQ_MIN overflows does the height
    // cap take over and the grid scrolls. With W=340/H=300: ~64–128 cores stay
    // at SQ_MAX (22px), ~256 settle around the mid-teens (~17px), and the
    // SQ_MIN (10px) floor isn't reached until ~600+ cores, beyond which it scrolls.
    const heat = createMemo(() => {
        const n = Math.max(1, cores().length);
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
    });

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

    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-end"
            onDismiss={props.onClose}
            class={`cpu-cores-popover cpu-cores-popover--${tier()}`}
            role="dialog"
            aria-label="Per-core CPU usage"
            style={{ width: `${panelWidth()}px` }}
        >
            <div class="cpu-cores-header">
                <span class="cpu-cores-title">CPU Usage</span>
                <span class="cpu-cores-aggregate" style={{ color: cpuColor(aggregate()) }}>
                    avg {Math.round(aggregate())}%
                </span>
            </div>
            <div class="cpu-cores-subtitle">
                <span>{readout()}</span>
                <Show when={tier() === "heat"}>
                    <span class="cpu-cores-legend" aria-hidden="true">
                        idle<span class="cpu-cores-legend-ramp" />busy
                    </span>
                </Show>
            </div>

            <Show
                when={cores().length > 0}
                fallback={<div class="cpu-cores-empty">Reading CPU…</div>}
            >
                {/* Rows + cells share a flex/grid scroll area; heatmap uses its
                    own computed-size square grid. */}
                <Show when={tier() !== "heat"}>
                    <div class="cpu-cores-list" classList={{ "cpu-cores-grid": tier() === "cells" }}>
                        <Index each={cores()}>
                            {(c) => (
                                <div
                                    class="cpu-core"
                                    onMouseEnter={() => setActiveIdx(c().idx)}
                                    onMouseLeave={() => setActiveIdx(null)}
                                >
                                    <span class="cpu-core-label">
                                        {tier() === "rows" ? `Core ${c().idx}` : `C${c().idx}`}
                                    </span>
                                    <span class="cpu-core-bar" aria-hidden="true">
                                        <span
                                            class="cpu-core-bar-fill"
                                            style={{
                                                width: `${Math.min(100, c().pct)}%`,
                                                "background-color": loadColor(c().pct),
                                            }}
                                        />
                                    </span>
                                    <span class="cpu-core-pct" style={{ color: loadColor(c().pct) }}>
                                        {Math.round(c().pct)}%
                                    </span>
                                </div>
                            )}
                        </Index>
                    </div>
                </Show>

                <Show when={tier() === "heat"}>
                    <div
                        ref={heatGridRef}
                        class="cpu-cores-heatmap"
                        role="group"
                        aria-label="Per-core CPU heatmap, arrow keys to navigate"
                        onKeyDown={onHeatKeyDown}
                        style={{
                            "--cols": String(heat().cols),
                            "--sq": `${heat().sq}px`,
                        }}
                    >
                        <Index each={cores()}>
                            {(c) => (
                                <span
                                    class="cpu-core-square"
                                    data-core-idx={c().idx}
                                    tabindex={c().idx === rovingIdx() ? 0 : -1}
                                    title={`Core ${c().idx} — ${Math.round(c().pct)}%`}
                                    aria-label={`Core ${c().idx}, ${Math.round(c().pct)}%`}
                                    style={{ "background-color": loadColor(c().pct) }}
                                    onMouseEnter={() => setActiveIdx(c().idx)}
                                    onMouseLeave={() => setActiveIdx(null)}
                                    onFocus={() => { setRovingIdx(c().idx); setActiveIdx(c().idx); }}
                                    onBlur={() => setActiveIdx(null)}
                                />
                            )}
                        </Index>
                    </div>
                </Show>
            </Show>
        </AnchoredPopover>
    );
};

CpuCoresPopover.displayName = "CpuCoresPopover";
