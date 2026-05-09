// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dev-mode perf HUD. Floating panel in the bottom-right corner that
 * shows aggregated stats from `perfStore`. Toggle with Ctrl+Shift+P.
 *
 * Polls `perfStore.snapshot()` once per second when visible. Hidden in
 * release builds via the `import.meta.env.DEV` gate; the toggle hook
 * is also no-opped in release.
 *
 * No styling system imports — keeps the HUD self-contained so it
 * still works during a perf investigation that disables the main
 * stylesheet.
 */

import { createSignal, onCleanup, onMount, Show, type JSX, For } from "solid-js";
import { perfStore } from "./store";

interface HudSnapshot {
    longTasks: { count: number; p50: number; p75: number; p95: number; max: number };
    longTasksLast5s: number;
    interactionsTopLatency: Array<{ key: string; p75: number; p95: number; max: number }>;
    ipcTopByP95: Array<{ key: string; p95: number; max: number; count: number }>;
}

function flatten(snap: ReturnType<typeof perfStore.snapshot>): HudSnapshot {
    const interactionsArr: Array<{
        key: string;
        p75: number;
        p95: number;
        max: number;
    }> = [];
    for (const [key, q] of snap.interactions) {
        interactionsArr.push({ key, p75: q.p75, p95: q.p95, max: q.max });
    }
    interactionsArr.sort((a, b) => b.p95 - a.p95);
    return {
        longTasks: snap.longTasks,
        longTasksLast5s: snap.longTasksLast5s,
        interactionsTopLatency: interactionsArr.slice(0, 5),
        ipcTopByP95: snap.ipcTopByP95.map((r) => ({
            key: r.key,
            p95: r.q.p95,
            max: r.q.max,
            count: r.q.count,
        })),
    };
}

export function PerfHud(): JSX.Element {
    const [visible, setVisible] = createSignal(false);
    const [snap, setSnap] = createSignal<HudSnapshot>(flatten(perfStore.snapshot()));
    let pollInterval: ReturnType<typeof setInterval> | null = null;

    const onKey = (e: KeyboardEvent) => {
        // Ctrl+Shift+P (or Meta+Shift+P on macOS where Ctrl is rare).
        if (e.shiftKey && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "p") {
            e.preventDefault();
            setVisible((v) => !v);
        }
    };

    onMount(() => {
        window.addEventListener("keydown", onKey);
        pollInterval = setInterval(() => {
            if (visible()) setSnap(flatten(perfStore.snapshot()));
        }, 1000);
    });

    onCleanup(() => {
        window.removeEventListener("keydown", onKey);
        if (pollInterval) clearInterval(pollInterval);
    });

    return (
        <Show when={visible()}>
            <div
                style={{
                    position: "fixed",
                    bottom: "8px",
                    right: "8px",
                    "z-index": "999999",
                    "background-color": "rgba(20, 20, 20, 0.92)",
                    color: "#d0d0d0",
                    "font-family": "Menlo, Consolas, monospace",
                    "font-size": "11px",
                    padding: "8px 10px",
                    border: "1px solid #444",
                    "border-radius": "6px",
                    "max-width": "320px",
                    "box-shadow": "0 4px 12px rgba(0,0,0,0.5)",
                    "pointer-events": "none",
                }}
            >
                <div style={{ "font-weight": "bold", color: "#7af", "margin-bottom": "4px" }}>
                    ⏱ Perf HUD &nbsp;
                    <span style={{ color: "#777", "font-weight": "normal" }}>
                        (Ctrl+Shift+P)
                    </span>
                </div>
                <div style={{ "margin-top": "4px" }}>
                    <span style={{ color: "#999" }}>Long tasks:</span>{" "}
                    {snap().longTasksLast5s} in last 5s
                    <Show when={snap().longTasks.count > 0}>
                        {" "}
                        (max {snap().longTasks.max.toFixed(0)}ms,
                        P95 {snap().longTasks.p95.toFixed(0)}ms)
                    </Show>
                </div>
                <Show when={snap().interactionsTopLatency.length > 0}>
                    <div style={{ "margin-top": "6px", color: "#999" }}>
                        Interactions (top by P95):
                    </div>
                    <For each={snap().interactionsTopLatency}>
                        {(row) => (
                            <div style={{ "padding-left": "8px" }}>
                                <span style={{ color: "#aaa" }}>{row.key}</span>{" "}
                                P75 {row.p75.toFixed(0)}ms{" "}
                                P95 {row.p95.toFixed(0)}ms
                            </div>
                        )}
                    </For>
                </Show>
                <Show when={snap().ipcTopByP95.length > 0}>
                    <div style={{ "margin-top": "6px", color: "#999" }}>
                        IPC (top by P95):
                    </div>
                    <For each={snap().ipcTopByP95}>
                        {(row) => (
                            <div style={{ "padding-left": "8px" }}>
                                <span style={{ color: "#aaa" }}>{row.key}</span>{" "}
                                P95 {row.p95.toFixed(0)}ms ({row.count} samples)
                            </div>
                        )}
                    </For>
                </Show>
            </div>
        </Show>
    );
}
