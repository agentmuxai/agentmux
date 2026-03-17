// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import type { SubagentViewModel, SubagentEvent, SubagentEventType } from "./subagent-model";
import "./subagent-view.scss";

export function SubagentView(props: ViewComponentProps<SubagentViewModel>): JSX.Element {
    const info = props.model.infoAtom;
    const events = props.model.eventsAtom;
    const status = props.model.statusAtom;
    const autoScroll = props.model.autoScrollAtom;

    let scrollRef: HTMLDivElement | null = null;

    // Auto-scroll to bottom when new events arrive
    createEffect(() => {
        const _ = events(); // track dependency
        if (autoScroll() && scrollRef) {
            requestAnimationFrame(() => {
                scrollRef!.scrollTop = scrollRef!.scrollHeight;
            });
        }
    });

    const handleScroll = () => {
        if (!scrollRef) return;
        const atBottom =
            scrollRef.scrollHeight - scrollRef.scrollTop - scrollRef.clientHeight < 40;
        props.model.setAutoScroll(atBottom);
    };

    const elapsed = () => {
        const i = info();
        if (!i) return "";
        const ms = Date.now() - i.last_event_at;
        if (ms < 1000) return "just now";
        const secs = Math.floor(ms / 1000);
        if (secs < 60) return `${secs}s ago`;
        const mins = Math.floor(secs / 60);
        return `${mins}m ago`;
    };

    return (
        <div class="subagent-pane">
            <div class="subagent-header">
                <div class="subagent-header-left">
                    <span class="subagent-header-icon">
                        <i class="fa-solid fa-diagram-subtask" />
                    </span>
                    <Show when={info()}>
                        <span class="subagent-header-slug">{info()!.slug || info()!.agent_id}</span>
                        <span class="subagent-header-id">({info()!.agent_id.substring(0, 7)})</span>
                    </Show>
                    <Show when={!info()}>
                        <span class="subagent-header-slug">Subagent</span>
                    </Show>
                </div>
                <div class="subagent-header-right">
                    <span
                        class={`subagent-status-badge subagent-status-${status()}`}
                    >
                        {status()}
                    </span>
                    <Show when={info()}>
                        <span class="subagent-header-meta">
                            {info()!.event_count} events
                        </span>
                        <span class="subagent-header-meta">{elapsed()}</span>
                    </Show>
                    <Show when={info()?.model}>
                        <span class="subagent-header-model">{info()!.model}</span>
                    </Show>
                </div>
            </div>
            <div class="subagent-divider" />
            <div
                class="subagent-events"
                ref={(el) => { scrollRef = el; }}
                onScroll={handleScroll}
            >
                <Show when={status() === "loading"}>
                    <div class="subagent-loading">Loading subagent activity...</div>
                </Show>
                <Show when={events().length === 0 && status() !== "loading"}>
                    <div class="subagent-empty">No activity yet</div>
                </Show>
                <For each={events()}>{(event) =>
                    <SubagentEventItem event={event} />
                }</For>
            </div>
            <Show when={!autoScroll()}>
                <button
                    class="subagent-scroll-btn"
                    onClick={() => {
                        props.model.setAutoScroll(true);
                        if (scrollRef) {
                            scrollRef.scrollTop = scrollRef.scrollHeight;
                        }
                    }}
                >
                    Scroll to bottom
                </button>
            </Show>
        </div>
    );
}

// ── Event item rendering ──────────────────────────────────────────────────

function SubagentEventItem(props: { event: SubagentEvent }): JSX.Element {
    const et = props.event.event_type;
    const time = () => {
        const d = new Date(props.event.timestamp);
        return d.toLocaleTimeString(undefined, { hour12: false });
    };

    return (
        <div class={`subagent-event subagent-event-${et.type}`}>
            <span class="subagent-event-time">{time()}</span>
            <EventContent eventType={et} />
        </div>
    );
}

function EventContent(props: { eventType: SubagentEventType }): JSX.Element {
    const et = props.eventType;
    const [expanded, setExpanded] = createSignal(false);

    switch (et.type) {
        case "text":
            return (
                <div class="subagent-event-body">
                    <pre class="subagent-event-text">{et.content}</pre>
                </div>
            );
        case "tool_use":
            return (
                <div class="subagent-event-body">
                    <div
                        class="subagent-event-tool-header"
                        onClick={() => setExpanded(!expanded())}
                    >
                        <i class={`fa-solid fa-${expanded() ? "chevron-down" : "chevron-right"} subagent-expand-icon`} />
                        <span class="subagent-tool-name">{et.name}</span>
                    </div>
                    <Show when={expanded()}>
                        <pre class="subagent-event-input">{et.input_summary}</pre>
                    </Show>
                </div>
            );
        case "tool_result":
            return (
                <div class="subagent-event-body">
                    <div
                        class={`subagent-event-result-header ${et.is_error ? "error" : ""}`}
                        onClick={() => setExpanded(!expanded())}
                    >
                        <i class={`fa-solid fa-${expanded() ? "chevron-down" : "chevron-right"} subagent-expand-icon`} />
                        <span class="subagent-result-label">
                            {et.is_error ? "Error" : "Result"}
                        </span>
                    </div>
                    <Show when={expanded()}>
                        <pre class={`subagent-event-output ${et.is_error ? "error" : ""}`}>
                            {et.preview}
                        </pre>
                    </Show>
                </div>
            );
        case "progress":
            return (
                <div class="subagent-event-body subagent-event-progress">
                    <i class="fa-solid fa-spinner fa-spin subagent-progress-icon" />
                    <span>{et.output}</span>
                </div>
            );
        default:
            return null;
    }
}
