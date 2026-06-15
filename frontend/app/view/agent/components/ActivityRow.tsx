// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActivityRow — one uniform row in the pinned activity dock. Kind-agnostic
 * chrome (sigil + title + elapsed + tail + stop); the expanded view dispatches
 * by kind (Phase 1: shell → streaming log).
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md (§4)
 */

import clsx from "clsx";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import { capChars, createChunkCapper, MAX_TOOL_OUTPUT_LINES } from "./output-cap";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import { KIND_SIGIL, type PinnedActivity } from "../activity/types";
import type { ToolLogChunk } from "../types";

const KIND_CLASS: Record<string, string> = {
    stdout: "agent-tool-log-line--stdout",
    stderr: "agent-tool-log-line--stderr",
    system: "agent-tool-log-line--system",
};

function formatElapsed(ms: number): string {
    const totalSec = Math.max(0, Math.floor(ms / 1000));
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    return `${m}:${String(s).padStart(2, "0")}`;
}

interface ActivityRowProps {
    /** Reactive accessor — returns undefined if the activity just left. */
    activity: () => PinnedActivity | undefined;
    expanded: () => boolean;
    onToggle: () => void;
    onStop: () => void;
    onDismiss: () => void;
}

export const ActivityRow = (props: ActivityRowProps): JSX.Element => {
    const [now, setNow] = createSignal(Date.now());

    const isRunning = createMemo(() => props.activity()?.status === "running");
    createEffect(() => {
        if (!isRunning()) return;
        const id = setInterval(() => setNow(Date.now()), 1000);
        onCleanup(() => clearInterval(id));
    });

    const elapsed = createMemo(() => {
        const a = props.activity();
        if (!a) return "";
        const end = a.endedAt ?? now();
        return formatElapsed(end - a.startedAt);
    });

    // Terminal statuses override the kind sigil with a result glyph.
    const sigil = createMemo(() => {
        const a = props.activity();
        if (!a) return "";
        switch (a.status) {
            case "running": return KIND_SIGIL[a.kind];
            case "done": return "✓";
            case "error": return "✗";
            case "stopped": return "■";
        }
    });

    const tail = createMemo((): string | undefined => {
        const sh = props.activity()?.shell;
        if (!sh) return undefined;
        const chunks = sh.log.chunks;
        for (let i = chunks.length - 1; i >= 0; i--) {
            const c = chunks[i];
            if ((c.kind === "stdout" || c.kind === "stderr") && c.content.trim()) {
                return c.content.trim();
            }
        }
        return undefined;
    });

    // Expanded shell log — same cap + renderer as PersistentShellBlock.
    const chunkCap = createChunkCapper(MAX_TOOL_OUTPUT_LINES);
    const capped = createMemo(() => {
        const sh = props.activity()?.shell;
        return sh
            ? chunkCap(sh.log.chunks as ToolLogChunk[])
            : { chunks: [] as ToolLogChunk[], hiddenLines: 0 };
    });

    return (
        <Show when={props.activity()}>
            {(a) => (
                <div
                    class={clsx("agent-activity-row", a().kind, a().status, {
                        expanded: props.expanded(),
                    })}
                >
                    <div class="agent-activity-summary" onClick={props.onToggle}>
                        <span class="agent-activity-sigil">{sigil()}</span>
                        <span class="agent-activity-title">{a().title}</span>
                        <span class="agent-activity-elapsed">[{elapsed()}]</span>
                        <Show when={tail()}>
                            <span class="agent-activity-tail">↳ {tail()}</span>
                        </Show>
                        <Show when={a().canStop}>
                            <button
                                class="agent-activity-stop"
                                title="Stop"
                                onClick={(e) => { e.stopPropagation(); props.onStop(); }}
                            >
                                ■
                            </button>
                        </Show>
                        <Show when={!a().canStop && a().status === "error"}>
                            <button
                                class="agent-activity-dismiss"
                                title="Dismiss"
                                onClick={(e) => { e.stopPropagation(); props.onDismiss(); }}
                            >
                                ×
                            </button>
                        </Show>
                    </div>

                    <Show when={props.expanded() && a().shell}>
                        <div
                            class="agent-activity-log agent-tool-overlay-log"
                            onClick={(e) => e.stopPropagation()}
                        >
                            <Show when={capped().hiddenLines > 0}>
                                <OutputHiddenMarker hidden={capped().hiddenLines} noun="line" from="tail" />
                            </Show>
                            <For each={capped().chunks}>
                                {(chunk) => (
                                    <pre class={`agent-tool-log-line ${KIND_CLASS[chunk.kind] ?? ""}`}>
                                        {capChars(chunk.content)}
                                    </pre>
                                )}
                            </For>
                            <Show when={a().shell!.log.open}>
                                <div class="agent-shell-streaming-indicator" />
                            </Show>
                        </div>
                    </Show>
                </div>
            )}
        </Show>
    );
};

ActivityRow.displayName = "ActivityRow";
