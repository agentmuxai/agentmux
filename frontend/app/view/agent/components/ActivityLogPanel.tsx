// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ActivityLogPanel — collapsible log panel docked above the composer.
 * Shows the per-pane activity log (launch flow, subprocess lifecycle,
 * slash command outcomes, errors). Collapsed by default; a one-line
 * summary (most recent entry + total count) is shown in the header.
 * Auto-expands when new entries arrive, unless the user has explicitly
 * collapsed it — in that case it stays closed until the user reopens it.
 *
 * Replaces the old `.agent-status-log` block that lived at the top of
 * the conversation scroll area and grew unbounded over the session —
 * see `agentmux-ai/AGENT_PANE_ACTIVITY_LOG_SPEC.md`.
 */

import { For, Show, createEffect, createMemo, createSignal, type Accessor, type JSX } from "solid-js";
import type { LogLine } from "../types";

interface ActivityLogPanelProps {
    entries: Accessor<LogLine[]>;
}

export const ActivityLogPanel = (props: ActivityLogPanelProps): JSX.Element => {
    const [isOpen, setIsOpen] = createSignal(false);
    const [expandedIds, setExpandedIds] = createSignal<Set<string>>(new Set());
    // True after the user explicitly clicks to close — suppresses auto-expand
    // until they reopen the panel themselves, preventing non-bang log entries
    // (subprocess lifecycle, slash outcomes) from fighting the user's intent.
    let userCollapsed = false;

    // Auto-expand when new entries arrive, but only if the user hasn't
    // explicitly collapsed the panel since the last open.
    let prevLength = props.entries().length;
    createEffect(() => {
        const len = props.entries().length;
        if (len > prevLength && !userCollapsed) {
            setIsOpen(true);
        }
        prevLength = len;
    });

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
                    onClick={() => {
                        const next = !isOpen();
                        userCollapsed = !next; // track explicit close to suppress auto-expand
                        setIsOpen(next);
                    }}
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
                                        class="agent-status-line agent-status-line--toggle"
                                        classList={{
                                            "agent-status-line--error": line.level === "error",
                                            "agent-status-line--warn": line.level === "warn",
                                            "agent-status-line--expanded": isExpanded(),
                                        }}
                                        // Click toggles full/truncated text — the
                                        // expand affordance the removed hover strip
                                        // used to provide.
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
                </Show>
            </div>
        </Show>
    );
};

ActivityLogPanel.displayName = "ActivityLogPanel";
