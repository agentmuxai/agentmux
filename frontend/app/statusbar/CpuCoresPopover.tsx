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
 * Positioning + airspace mirror TokenBreakdownPopover (the canonical
 * status-bar popover): usePaneOverlay, computeMenuPosition top-end, autoUpdate.
 */

import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { autoUpdate } from "@floating-ui/dom";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { assertMenuInPaintableArea, computeMenuPosition } from "@/app/util/menu-position";
import { waveEventSubscribe } from "@/app/store/wps";
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
    anchorRect: DOMRect | null;
    ref?: (el: HTMLDivElement) => void;
}

export const CpuCoresPopover = (props: CpuCoresPopoverProps): JSX.Element => {
    let rootRef: HTMLDivElement | undefined;

    // Airspace cut so the popover paints over any browser-pane HWND the status
    // bar overlaps — same primitive as <Modal> and TokenBreakdownPopover.
    usePaneOverlay(() => rootRef);

    const [cores, setCores] = createSignal<Core[]>([]);
    const [aggregate, setAggregate] = createSignal(0);
    // Hovered/focused core drives the readout line (cheaper + a11y-friendlier
    // than 128 simultaneous tooltips in heatmap mode).
    const [active, setActive] = createSignal<Core | null>(null);

    onMount(() => {
        const unsub = waveEventSubscribe({
            eventType: "sysinfo",
            scope: "local",
            handler: (event) => {
                const vals = (event as WaveEvent)?.data?.values;
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
    // genuinely shrink toward SQ_MIN as the count grows; only once SQ_MIN still
    // overflows does the height cap take over and the grid scrolls. So e.g.
    // ~64–128 cores stay near SQ_MAX, ~256 lands near SQ_MIN, and beyond that it
    // scrolls.
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

    // ── Positioning (mirrors TokenBreakdownPopover) ──────────────────────────
    const [floatingStyle, setFloatingStyle] = createSignal<JSX.CSSProperties>({
        position: "fixed",
        left: "0px",
        top: "0px",
    });
    let cleanupAutoUpdate: (() => void) | null = null;

    const registerFloating = (el: HTMLDivElement) => {
        rootRef = el;
        props.ref?.(el);
        requestAnimationFrame(() => {
            const r = props.anchorRect;
            if (!r || !(el instanceof Element)) return;
            const update = async () => {
                const cur = props.anchorRect;
                if (!cur) return;
                const pos = await computeMenuPosition({ anchor: cur, placement: "top-end" }, el);
                setFloatingStyle(pos.style);
            };
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(
                { getBoundingClientRect: () => props.anchorRect ?? r },
                el,
                update,
            );
            assertMenuInPaintableArea(el, "cpu-cores-popover");
        });
    };

    onCleanup(() => cleanupAutoUpdate?.());

    const readout = (): string => {
        const a = active();
        if (a) return `Core ${a.idx} — ${Math.round(a.pct)}%`;
        return `${cores().length} cores`;
    };

    return (
        <div
            ref={registerFloating}
            class="cpu-cores-popover"
            classList={{ [`cpu-cores-popover--${tier()}`]: true }}
            role="dialog"
            aria-label="Per-core CPU usage"
            data-pane-overlay
            style={{ ...floatingStyle(), width: `${panelWidth()}px` }}
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
                        <For each={cores()}>
                            {(c) => (
                                <div
                                    class="cpu-core"
                                    onMouseEnter={() => setActive(c)}
                                    onMouseLeave={() => setActive(null)}
                                >
                                    <span class="cpu-core-label">
                                        {tier() === "rows" ? `Core ${c.idx}` : `C${c.idx}`}
                                    </span>
                                    <span class="cpu-core-bar" aria-hidden="true">
                                        <span
                                            class="cpu-core-bar-fill"
                                            style={{
                                                width: `${Math.min(100, c.pct)}%`,
                                                "background-color": loadColor(c.pct),
                                            }}
                                        />
                                    </span>
                                    <span class="cpu-core-pct" style={{ color: loadColor(c.pct) }}>
                                        {Math.round(c.pct)}%
                                    </span>
                                </div>
                            )}
                        </For>
                    </div>
                </Show>

                <Show when={tier() === "heat"}>
                    <div
                        class="cpu-cores-heatmap"
                        role="list"
                        style={{
                            "--cols": String(heat().cols),
                            "--sq": `${heat().sq}px`,
                        }}
                    >
                        <For each={cores()}>
                            {(c) => (
                                <span
                                    class="cpu-core-square"
                                    role="listitem"
                                    tabindex="0"
                                    title={`Core ${c.idx} — ${Math.round(c.pct)}%`}
                                    aria-label={`Core ${c.idx}, ${Math.round(c.pct)}%`}
                                    style={{ "background-color": loadColor(c.pct) }}
                                    onMouseEnter={() => setActive(c)}
                                    onMouseLeave={() => setActive(null)}
                                    onFocus={() => setActive(c)}
                                    onBlur={() => setActive(null)}
                                />
                            )}
                        </For>
                    </div>
                </Show>
            </Show>
        </div>
    );
};

CpuCoresPopover.displayName = "CpuCoresPopover";
