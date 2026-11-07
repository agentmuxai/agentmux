// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Reducer-stack diagnostics panel — slice #9 Phase 5.
 *
 * Floating dev-mode panel that surfaces `dispatchRecordsAtom` from
 * `command-source.ts`: every reducer dispatch across every slice
 * (browser-pane-state, agent-pane-state, layout, launcher-event, ...)
 * lands in this view in real time.
 *
 * **Why this exists:** when a multi-pane focus or pool-drift bug
 * fires, the question is always "what happened in the last 200 ms,
 * in what order, across which panes?". Tail-grepping host logs works
 * but loses structure (every line is plain text, every record is
 * spread across 5+ lines of `state-write`+`dispatch` diag entries).
 * The audit ring already has the data structured; this panel just
 * renders it.
 *
 * Toggle with Ctrl+Alt+Shift+F12 (the `dev:diagnostics` row in keybindings/defaults.ts). Hidden in release builds via the
 * `isDev()` gate at the mount site; the toggle hook is no-opped in
 * release.
 *
 * Self-contained styling — same approach as the perf HUD
 * (`frontend/perf/hud.tsx`) so the panel still works during a
 * stylesheet-disabled investigation.
 */

import { For, Show, type JSX } from "solid-js";
import { Button } from "@/element/ui";
import { __resetDispatchLog } from "@/store/command-source";
import { AgentPanePerfSection } from "./agent-pane-perf-section";
import { shortcutFor } from "@/app/keybindings";
import { Z_DEV_DIAGNOSTICS } from "@/app/zindex";
import { TextInput } from "@/app/element/ui";
import { ageMs, createDiagPanel, type DiagPanelState } from "./diag-panel-state";

/** The title and the count of dispatches recorded. */
function DiagHeader(props: { s: DiagPanelState }): JSX.Element {
    const s = props.s;
    return (
        <div
            style={{
                display: "flex",
                "align-items": "center",
                gap: "8px",
                "margin-bottom": "6px",
            }}
        >
            <div style={{ "font-weight": "bold", color: "#7af" }}>
                🔬 Reducer Dispatch Ring
            </div>
            <span style={{ color: "#777" }}>
                ({s.totalDispatches()} total — {shortcutFor("dev:diagnostics")})
            </span>
        </div>
    );
}

/** The slice dropdown, the key filter and the clear button. */
function DiagFilters(props: { s: DiagPanelState }): JSX.Element {
    const s = props.s;
    return (
        <div
            style={{
                display: "flex",
                gap: "6px",
                "margin-bottom": "6px",
                "align-items": "center",
            }}
        >
            <select
                value={s.sliceFilter()}
                onInput={(e) => s.setSliceFilter(e.currentTarget.value)}
                style={{
                    "background-color": "#222",
                    color: "#d0d0d0",
                    border: "1px solid #444",
                    "border-radius": "3px",
                    padding: "2px 4px",
                    "font-family": "inherit",
                    "font-size": "inherit",
                }}
            >
                <option value="">all slices</option>
                <For each={s.availableSlices()}>
                    {(s) => <option value={s}>{s}</option>}
                </For>
            </select>
            <TextInput
                variant="plain"
                placeholder="filter by key…"
                value={s.keyFilter()}
                onInput={(e) => s.setKeyFilter(e.currentTarget.value)}
                style={{
                    flex: "1",
                    "background-color": "#222",
                    color: "#d0d0d0",
                    border: "1px solid #444",
                    "border-radius": "3px",
                    padding: "2px 6px",
                    "font-family": "inherit",
                    "font-size": "inherit",
                }}
            />
            <Button density="compact" onClick={() => __resetDispatchLog()} title="Wipe the dispatch ring (dev only)">
                clear
            </Button>
        </div>
    );
}

/** The newest dispatches, or why there are none to show. */
function DiagRows(props: { s: DiagPanelState }): JSX.Element {
    const s = props.s;
    return (
        <div
            style={{
                "overflow-y": "auto",
                flex: "1",
                "border-top": "1px solid #333",
                "padding-top": "4px",
            }}
        >
            <Show
                when={s.rows().length > 0}
                fallback={
                    <div style={{ color: "#888", "padding": "8px" }}>
                        {s.totalDispatches() === 0
                            ? "No dispatches recorded yet."
                            : "No matches for the current filter."}
                    </div>
                }
            >
                <table
                    style={{
                        width: "100%",
                        "border-collapse": "collapse",
                    }}
                >
                    <thead>
                        <tr style={{ color: "#888" }}>
                            <th style={hdrStyle}>age</th>
                            <th style={hdrStyle}>slice</th>
                            <th style={hdrStyle}>key</th>
                            <th style={hdrStyle}>command</th>
                            <th style={hdrStyle}>source</th>
                            <th style={hdrStyle}>events</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={s.rows()}>
                            {(row) => (
                                <tr>
                                    {/* Age is computed inline so only the cell
                                        re-renders on the 1 Hz tick — the row
                                        identity stays stable across ticks (see
                                        the rows memo's comment). */}
                                    <td style={cellStyle}>{ageMs(s.now(), row.at)}</td>
                                    <td style={cellStyle}>
                                        <span style={{ color: "#aaa" }}>{row.slice}</span>
                                    </td>
                                    <td style={cellStyle}>{row.keyShort}</td>
                                    <td style={cellStyle}>
                                        <span style={{ color: "#7af" }}>{row.cmdType}</span>
                                    </td>
                                    <td style={cellStyle}>{row.source}</td>
                                    <td style={cellStyle}>{row.eventCount}</td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
        </div>
    );
}

export function DiagPanel(): JSX.Element {
    const s = createDiagPanel();

    return (
        <Show when={s.visible()}>
            <div
                style={{
                    position: "fixed",
                    bottom: "8px",
                    left: "8px",
                    "z-index": Z_DEV_DIAGNOSTICS, // under the perf HUD, so they don't fight
                    "background-color": "rgba(20, 20, 20, 0.94)",
                    color: "#d0d0d0",
                    "font-family": "Menlo, Consolas, monospace",
                    "font-size": "11px",
                    padding: "8px 10px",
                    border: "1px solid #444",
                    "border-radius": "6px",
                    width: "560px",
                    "max-height": "60vh",
                    display: "flex",
                    "flex-direction": "column",
                    "box-shadow": "0 4px 12px rgba(0, 0, 0, 0.5)",
                }}
            >
                <DiagHeader s={s} />
                <DiagFilters s={s} />
                <DiagRows s={s} />
                <AgentPanePerfSection />
            </div>
        </Show>
    );
}

const hdrStyle = {
    "text-align": "left" as const,
    padding: "2px 6px",
    "font-weight": "normal",
    "border-bottom": "1px solid #333",
};

const cellStyle = {
    padding: "2px 6px",
    "white-space": "nowrap" as const,
    "vertical-align": "top" as const,
};
