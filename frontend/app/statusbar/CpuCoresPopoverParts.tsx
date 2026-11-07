// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The per-core CPU panel's views: the title with the average, the readout line, and the three layouts. */

import { Index, Show, type JSX } from "solid-js";
import { tip } from "@/app/element/tip";
import { cpuColor, loadColor } from "./cpu-color";
import type { CpuCoresState } from "./CpuCoresPopoverState";

/** The title and the average across all cores. */
export function CpuHeader(props: { s: CpuCoresState }): JSX.Element {
    return (
        <div class="cpu-cores-header">
            <span class="cpu-cores-title">CPU Usage</span>
            <span class="cpu-cores-aggregate" style={{ color: cpuColor(props.s.aggregate()) }}>
                avg {Math.round(props.s.aggregate())}%
            </span>
        </div>
    );
}

/** The readout line (the hovered core, or the core count), with the heatmap's legend. */
export function CpuSubtitle(props: { s: CpuCoresState }): JSX.Element {
    return (
        <div class="cpu-cores-subtitle">
            <span>{props.s.readout()}</span>
            <Show when={props.s.tier() === "heat"}>
                <span class="cpu-cores-legend" aria-hidden="true">
                    idle<span class="cpu-cores-legend-ramp" />busy
                </span>
            </Show>
        </div>
    );
}

/** Rows and cells share a flex/grid scroll area; the heatmap uses its own computed-size square grid. */
export function CoreList(props: { s: CpuCoresState }): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.tier() !== "heat"}>
            <div class="cpu-cores-list" classList={{ "cpu-cores-grid": s.tier() === "cells" }}>
                <Index each={s.cores()}>
                    {(c) => (
                        <div
                            class="cpu-core"
                            onMouseEnter={() => s.setActiveIdx(c().idx)}
                            onMouseLeave={() => s.setActiveIdx(null)}
                        >
                            <span class="cpu-core-label">
                                {s.tier() === "rows" ? `Core ${c().idx}` : `C${c().idx}`}
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
    );
}

/** One square per core, one tab stop, arrow keys to move between them. */
export function CoreHeatmap(props: { s: CpuCoresState }): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.tier() === "heat"}>
            <div
                ref={s.setHeatGridRef}
                class="cpu-cores-heatmap"
                role="group"
                aria-label="Per-core CPU heatmap, arrow keys to navigate"
                onKeyDown={s.onHeatKeyDown}
                style={{
                    "--cols": String(s.heat().cols),
                    "--sq": `${s.heat().sq}px`,
                }}
            >
                <Index each={s.cores()}>
                    {(c) => (
                        <span
                            class="cpu-core-square"
                            data-core-idx={c().idx}
                            tabindex={c().idx === s.rovingIdx() ? 0 : -1}
                            {...tip(`Core ${c().idx} — ${Math.round(c().pct)}%`)}
                            aria-label={`Core ${c().idx}, ${Math.round(c().pct)}%`}
                            style={{ "background-color": loadColor(c().pct) }}
                            onMouseEnter={() => s.setActiveIdx(c().idx)}
                            onMouseLeave={() => s.setActiveIdx(null)}
                            onFocus={() => { s.setRovingIdx(c().idx); s.setActiveIdx(c().idx); }}
                            onBlur={() => s.setActiveIdx(null)}
                        />
                    )}
                </Index>
            </div>
        </Show>
    );
}
