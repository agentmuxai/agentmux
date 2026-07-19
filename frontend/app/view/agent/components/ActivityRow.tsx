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
import { useTick } from "@/app/hook/useTick";
import { capChars, createChunkCapper, MAX_TOOL_OUTPUT_LINES } from "./output-cap";
import { OutputHiddenMarker } from "./OutputHiddenMarker";
import {
    createDispatchDetail,
    subagentDisplayLabel,
    type ActiveSubagent,
    type DispatchDetail,
    type SubagentEvent,
} from "../../swarm/swarm-model";
import { KIND_SIGIL, type PinnedActivity } from "../activity/types";
import type { ToolLogChunk } from "../types";

/** One-line text summary per subagent event kind — deliberately simpler than
 *  the Swarm pane's own `SubagentDetailEvent` (no per-event expand/collapse,
 *  no dedicated CSS): the dock reuses the existing shell-log line chrome
 *  (`.agent-tool-log-line`) instead of depending on `swarm-view.scss`, which
 *  only loads once the user has opened a Swarm pane this session. */
function subagentEventLine(e: SubagentEvent): string {
    const t = e.event_type;
    switch (t.type) {
        case "text": return t.content;
        case "result": return t.content;
        case "progress": return t.output;
        case "tool_use": return `→ ${t.name}`;
        case "tool_result": return t.is_error ? `✗ ${t.preview}` : t.preview;
    }
}

/** Terminal-status glyph for one member row inside an expanded group roster
 *  — mirrors ActivityRow's own top-level `sigil()` (running/done/stopped),
 *  just scoped to a single `ActiveSubagent` instead of a `PinnedActivity`. */
function memberSigil(status: ActiveSubagent["status"]): string {
    switch (status) {
        case "active": return KIND_SIGIL.subagent;
        case "completed": return "✓";
        case "abandoned": return "■";
    }
}

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
    const tick = useTick(1000);

    const elapsed = createMemo(() => {
        const a = props.activity();
        if (!a) return "";
        const end = a.endedAt ?? (tick(), Date.now());
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
        const a = props.activity();
        if (!a) return undefined;
        if (a.shell) {
            const chunks = a.shell.log.chunks;
            for (let i = chunks.length - 1; i >= 0; i--) {
                const c = chunks[i];
                if ((c.kind === "stdout" || c.kind === "stderr") && c.content.trim()) {
                    return c.content.trim();
                }
            }
            return undefined;
        }
        if (a.subagent) {
            // Cheap tail from data already on ActiveSubagent — no extra
            // fetch/subscribe just to render a collapsed row's tail (that
            // cost is reserved for the expanded view, below).
            const n = a.subagent.event_count;
            return n > 0 ? `${n} event${n === 1 ? "" : "s"}` : undefined;
        }
        if (a.subagentGroup) {
            const members = a.subagentGroup.members;
            const active = members.filter((m) => m.status === "active").length;
            return active > 0 ? `${active}/${members.length} active` : `${members.length} subagents`;
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

    // Expanded subagent transcript — created only while this row is actually
    // expanded (mirrors SwarmViewModel's own lazy `getSubagentDetail` cache),
    // disposed on collapse/unmount so an idle dock isn't holding N live
    // event subscriptions for subagents nobody is looking at.
    const [dispatchDetail, setDispatchDetail] = createSignal<DispatchDetail | null>(null);
    createEffect(() => {
        const sub = props.activity()?.subagent;
        if (!props.expanded() || !sub) {
            setDispatchDetail(null);
            return;
        }
        const detail = createDispatchDetail(sub.dispatch_id);
        setDispatchDetail(detail);
        onCleanup(() => detail.dispose());
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

                    <Show when={props.expanded() && a().subagent}>
                        <div
                            class="agent-activity-log agent-tool-overlay-log"
                            onClick={(e) => e.stopPropagation()}
                        >
                            <Show when={dispatchDetail() && dispatchDetail()!.entriesAtom().length === 0}>
                                <pre class="agent-tool-log-line">No activity yet</pre>
                            </Show>
                            <For each={dispatchDetail()?.entriesAtom() ?? []}>
                                {(entry) => <pre class="agent-tool-log-line">{subagentEventLine(entry.event)}</pre>}
                            </For>
                        </div>
                    </Show>

                    {/* Group roster — a compact per-member summary, not a live
                        transcript: opening one dock row's expand shouldn't
                        subscribe to N members' event streams at once (a
                        single Task/Workflow run can spawn dozens). Full
                        per-member transcripts remain the Swarm pane's job. */}
                    <Show when={props.expanded() && a().subagentGroup}>
                        <div
                            class="agent-activity-log agent-tool-overlay-log"
                            onClick={(e) => e.stopPropagation()}
                        >
                            <For each={a().subagentGroup!.members}>
                                {(member) => (
                                    <pre class="agent-tool-log-line">
                                        {memberSigil(member.status)} {subagentDisplayLabel(member)}
                                        {member.event_count > 0
                                            ? ` — ${member.event_count} event${member.event_count === 1 ? "" : "s"}`
                                            : ""}
                                    </pre>
                                )}
                            </For>
                        </div>
                    </Show>
                </div>
            )}
        </Show>
    );
};

ActivityRow.displayName = "ActivityRow";
