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
 * Toggle with **Ctrl+Shift+D**. Hidden in release builds via the
 * `isDev()` gate at the mount site; the toggle hook is no-opped in
 * release.
 *
 * Self-contained styling — same approach as the perf HUD
 * (`frontend/perf/hud.tsx`) so the panel still works during a
 * stylesheet-disabled investigation.
 */

import {
    createEffect,
    createMemo,
    createSignal,
    For,
    onCleanup,
    onMount,
    Show,
    type JSX,
} from "solid-js";
import {
    dispatchRecordsAtom,
    describeSource,
    type DispatchRecord,
    __resetDispatchLog,
} from "@/store/command-source";
import { AgentPanePerfSection } from "./agent-pane-perf-section";

const DEFAULT_DISPLAY_LIMIT = 80;

interface DisplayRow {
    /** Index into the live records array — used for stable keys. */
    idx: number;
    /** Original record's wall-clock timestamp. The `age` string is
     *  computed inline in the render so it doesn't pollute the
     *  `rows` memo with a time dependency — see the createMemo
     *  comment for why that matters. */
    at: number;
    slice: string;
    /** Per-pane key, truncated to 7 chars when present. */
    keyShort: string;
    /** Command discriminator (e.g. "Navigate", "PaneClicked"). */
    cmdType: string;
    source: string;
    /** Number of events emitted by the dispatch. */
    eventCount: number;
}

function commandType(cmd: unknown): string {
    if (cmd != null && typeof cmd === "object" && "type" in cmd) {
        const t = (cmd as { type?: unknown }).type;
        if (typeof t === "string") return t;
    }
    return "?";
}

function ageMs(now: number, at: number): string {
    const dt = now - at;
    if (dt < 1000) return `${dt}ms ago`;
    if (dt < 60_000) return `${(dt / 1000).toFixed(1)}s ago`;
    return `${(dt / 60_000).toFixed(1)}min ago`;
}

function toRow(idx: number, rec: DispatchRecord): DisplayRow {
    return {
        idx,
        at: rec.at,
        slice: rec.slice,
        keyShort: rec.key ? rec.key.slice(0, 7) : "—",
        cmdType: commandType(rec.command),
        source: describeSource(rec.source),
        eventCount: rec.events.length,
    };
}

export function DiagPanel(): JSX.Element {
    const [visible, setVisible] = createSignal(false);
    const [now, setNow] = createSignal(Date.now());
    const [sliceFilter, setSliceFilter] = createSignal<string>("");
    const [keyFilter, setKeyFilter] = createSignal<string>("");

    let ageInterval: ReturnType<typeof setInterval> | null = null;

    const onKey = (e: KeyboardEvent) => {
        // Ctrl+Shift+D (or Meta+Shift+D on macOS where Ctrl is rare).
        if (e.shiftKey && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d") {
            e.preventDefault();
            setVisible((v) => !v);
        }
    };

    onMount(() => {
        window.addEventListener("keydown", onKey);
    });

    // Tick the displayed `age` once per second while the panel is
    // visible. The records signal already drives re-renders on new
    // dispatches; this just refreshes the relative-time strings on
    // existing rows. Only runs while visible — same bounded-cost
    // discipline as the perf HUD's setInterval gating.
    createEffect(() => {
        if (visible()) {
            setNow(Date.now());
            ageInterval = setInterval(() => setNow(Date.now()), 1000);
        } else if (ageInterval != null) {
            clearInterval(ageInterval);
            ageInterval = null;
        }
    });

    onCleanup(() => {
        window.removeEventListener("keydown", onKey);
        if (ageInterval != null) clearInterval(ageInterval);
    });

    // Available slices for the filter dropdown — derived from the
    // current records. Recomputed reactively so a new slice that
    // shows up appears in the dropdown without a manual refresh.
    const availableSlices = createMemo(() => {
        const seen = new Set<string>();
        for (const r of dispatchRecordsAtom()) seen.add(r.slice);
        return Array.from(seen).sort();
    });

    // Filtered + sliced view, newest first. Limited to
    // DEFAULT_DISPLAY_LIMIT rows so a runaway dispatch (which the
    // audit ring is precisely there to catch) doesn't wedge the
    // panel itself.
    //
    // **Important:** this memo deliberately does NOT depend on `now()`.
    // If it did, the 1 Hz age-tick interval would force a rebuild of
    // every DisplayRow and `<For>` would re-create every <tr> on
    // each tick — exactly the kind of inefficiency the panel exists
    // to surface. The `age` cell reads `now()` inline in the render
    // (Solid's fine-grained reactivity isolates the update to that
    // single text node).
    const rows = createMemo<DisplayRow[]>(() => {
        const all = dispatchRecordsAtom();
        const slice = sliceFilter();
        const key = keyFilter().trim().toLowerCase();
        const filtered: DisplayRow[] = [];
        // Walk newest-to-oldest so we can stop once we've collected
        // the display limit — saves work on large rings.
        for (let i = all.length - 1; i >= 0 && filtered.length < DEFAULT_DISPLAY_LIMIT; i--) {
            const r = all[i];
            if (slice !== "" && r.slice !== slice) continue;
            if (key !== "") {
                const recKey = (r.key ?? "").toLowerCase();
                if (!recKey.includes(key)) continue;
            }
            filtered.push(toRow(i, r));
        }
        return filtered;
    });

    const totalDispatches = createMemo(() => dispatchRecordsAtom().length);

    return (
        <Show when={visible()}>
            <div
                style={{
                    position: "fixed",
                    bottom: "8px",
                    left: "8px",
                    "z-index": "999998", // one less than perf HUD so they don't fight
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
                        ({totalDispatches()} total — Ctrl+Shift+D)
                    </span>
                </div>
                <div
                    style={{
                        display: "flex",
                        gap: "6px",
                        "margin-bottom": "6px",
                        "align-items": "center",
                    }}
                >
                    <select
                        value={sliceFilter()}
                        onInput={(e) => setSliceFilter(e.currentTarget.value)}
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
                        <For each={availableSlices()}>
                            {(s) => <option value={s}>{s}</option>}
                        </For>
                    </select>
                    <input
                        type="text"
                        placeholder="filter by key…"
                        value={keyFilter()}
                        onInput={(e) => setKeyFilter(e.currentTarget.value)}
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
                    <button
                        onClick={() => __resetDispatchLog()}
                        style={{
                            "background-color": "#332",
                            color: "#d0d0d0",
                            border: "1px solid #555",
                            "border-radius": "3px",
                            padding: "2px 8px",
                            cursor: "pointer",
                            "font-family": "inherit",
                            "font-size": "inherit",
                        }}
                        title="Wipe the dispatch ring (dev only)"
                    >
                        clear
                    </button>
                </div>
                <div
                    style={{
                        "overflow-y": "auto",
                        flex: "1",
                        "border-top": "1px solid #333",
                        "padding-top": "4px",
                    }}
                >
                    <Show
                        when={rows().length > 0}
                        fallback={
                            <div style={{ color: "#888", "padding": "8px" }}>
                                {totalDispatches() === 0
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
                                <For each={rows()}>
                                    {(row) => (
                                        <tr>
                                            {/* Age is computed inline so only the cell
                                                re-renders on the 1 Hz tick — the row
                                                identity stays stable across ticks (see
                                                the rows memo's comment). */}
                                            <td style={cellStyle}>{ageMs(now(), row.at)}</td>
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
