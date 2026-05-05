// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActivityLogPanel — collapsible log panel docked above the composer.
 * Shows the per-pane activity log (launch flow, subprocess lifecycle,
 * slash command outcomes, errors). Collapsed by default; a one-line
 * summary (most recent entry + total count) is shown in the header.
 * Expansion is purely user-driven — clicking the header toggles it.
 *
 * Replaces the old `.agent-status-log` block that lived at the top of
 * the conversation scroll area and grew unbounded over the session —
 * see `agentmux-ai/AGENT_PANE_ACTIVITY_LOG_SPEC.md`.
 */

import { For, Show, createMemo, createSignal, type Accessor, type JSX } from "solid-js";
import type { LogLine } from "../types";
import { NodeHoverStrip } from "./NodeHoverStrip";

interface ActivityLogPanelProps {
    entries: Accessor<LogLine[]>;
}

export const ActivityLogPanel = (props: ActivityLogPanelProps): JSX.Element => {
    const [isOpen, setIsOpen] = createSignal(false);
    const [expandedIds, setExpandedIds] = createSignal<Set<string>>(new Set());

    const mostRecent = createMemo(() => {
        const list = props.entries();
        return list.length > 0 ? list[list.length - 1] : null;
    });

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
                    "agent-activity-log--open": isOpen(),
                    "agent-activity-log--has-error": mostRecent()?.level === "error",
                    "agent-activity-log--has-warn": mostRecent()?.level === "warn",
                }}
            >
                <button
                    type="button"
                    class="agent-activity-log-header"
                    onClick={() => setIsOpen(!isOpen())}
                    title={isOpen() ? "Collapse shell log" : "Expand shell log"}
                    aria-expanded={isOpen()}
                >
                    <span class="agent-activity-log-chevron">{isOpen() ? "⌄" : "›"}</span>
                    <Show when={!isOpen() && mostRecent()}>
                        {(entry) => (
                            <span class="agent-activity-log-preview">
                                <span class="agent-status-tag">[{entry().tag}]</span>
                                <span class="agent-activity-log-preview-text">{entry().text}</span>
                            </span>
                        )}
                    </Show>
                    <span class="agent-activity-log-count">
                        {props.entries().length}
                        {props.entries().length === 1 ? " entry" : " entries"}
                    </span>
                </button>
                <Show when={isOpen()}>
                    <div class="agent-activity-log-body">
                        <For each={props.entries()}>
                            {(line) => {
                                const isExpanded = () => expandedIds().has(line.id);
                                return (
                                    <div
                                        class="hover-strip-host agent-status-line"
                                        classList={{
                                            "agent-status-line--error": line.level === "error",
                                            "agent-status-line--warn": line.level === "warn",
                                            "agent-status-line--expanded": isExpanded(),
                                        }}
                                    >
                                        <span class="agent-status-line-text">
                                            <span class="agent-status-tag">[{line.tag}]</span> {line.text}
                                        </span>
                                        <NodeHoverStrip
                                            nodeId={line.id}
                                            timestamp={line.timestamp}
                                            canExpand
                                            isExpanded={isExpanded()}
                                            onExpand={() => toggleExpanded(line.id)}
                                        />
                                    </div>
                                );
                            }}
                        </For>
                    </div>
                </Show>
            </div>
        </Show>
    );
};

ActivityLogPanel.displayName = "ActivityLogPanel";
