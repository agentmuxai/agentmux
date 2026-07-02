// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActivityLogPanel — the per-pane activity log docked above the composer
 * (launch flow, subprocess lifecycle, slash command outcomes, errors).
 *
 * This panel is mounted ONLY while the composer's details region is open —
 * i.e. the "Log" button in `AgentComposerStrip` is the single expand/collapse
 * toggle for the whole list. So the panel renders its full entries list
 * DIRECTLY: there is no second header toggle / one-line-summary layer.
 * (The redundant middle collapse level was removed per
 * SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02 Part C.) The only remaining
 * collapse is per-entry: clicking a row toggles truncated ↔ full text.
 *
 * Replaces the old `.agent-status-log` block that lived at the top of the
 * conversation scroll area and grew unbounded — see
 * `agentmux-ai/AGENT_PANE_ACTIVITY_LOG_SPEC.md`.
 */

import { For, Show, createSignal, type Accessor, type JSX } from "solid-js";
import type { LogLine } from "../types";

interface ActivityLogPanelProps {
    entries: Accessor<LogLine[]>;
}

export const ActivityLogPanel = (props: ActivityLogPanelProps): JSX.Element => {
    // Per-entry expand (truncated ↔ full text) — the sole collapse level here.
    const [expandedIds, setExpandedIds] = createSignal<Set<string>>(new Set());

    // Latest entry's level drives the container error/warn accent (the rows
    // also carry their own per-line level styling).
    const lastLevel = (): LogLine["level"] | undefined => {
        const list = props.entries();
        return list.length > 0 ? list[list.length - 1].level : undefined;
    };

    const toggleExpanded = (id: string) => {
        setExpandedIds((prev) => {
            const next = new Set(prev);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            return next;
        });
    };

    return (
        <Show when={props.entries().length > 0}>
            <div
                class="agent-activity-log"
                classList={{
                    "agent-activity-log--has-error": lastLevel() === "error",
                    "agent-activity-log--has-warn": lastLevel() === "warn",
                }}
            >
                <div class="agent-activity-log-body">
                    <For each={props.entries()}>
                        {(line) => {
                            const isExpanded = () => expandedIds().has(line.id);
                            return (
                                <div
                                    class="agent-status-line agent-status-line--toggle"
                                    classList={{
                                        "agent-status-line--error": line.level === "error",
                                        "agent-status-line--warn": line.level === "warn",
                                        "agent-status-line--expanded": isExpanded(),
                                    }}
                                    // Click toggles full/truncated text — the
                                    // per-entry expand affordance.
                                    onClick={() => toggleExpanded(line.id)}
                                >
                                    <span class="agent-status-line-text">
                                        <span class="agent-status-tag">[{line.tag}]</span> {line.text}
                                    </span>
                                </div>
                            );
                        }}
                    </For>
                </div>
            </div>
        </Show>
    );
};

ActivityLogPanel.displayName = "ActivityLogPanel";
