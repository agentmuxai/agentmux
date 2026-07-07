// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import type { SubagentViewModel, SubagentEvent, SubagentEventType } from "./subagent-model";
import { BrainSpinner } from "@/app/element/BrainSpinner";
import { scheduleOnSettle } from "@/app/util/settle-detector";
import "./subagent-view.scss";

export function SubagentView(props: ViewComponentProps<SubagentViewModel>): JSX.Element {
    const info = props.model.infoAtom;
    const events = props.model.eventsAtom;
    const status = props.model.statusAtom;
    const autoScroll = props.model.autoScrollAtom;

    // Loading overlay — mirrors agent-view.tsx's brain-spinner treatment
    // (docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md):
    // shown from mount, cross-fades out once loadHistory's two RPCs
    // (GetHistory + GetInfo) have resolved AND the resulting DOM has
    // actually painted. #1992 wired the brain-spinner treatment into
    // block.tsx's generic stage-one fallback (before the block/viewModel
    // resolve) and into agent-view.tsx's stage-two window, but SubagentView
    // resolves its viewModel near-instantly — its own stage-two window
    // (loadHistory's RPC round trip) was left showing only a static
    // "Loading subagent activity..." text. See
    // docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
    // Finding 4.
    const [historyLoaded, setHistoryLoaded] = createSignal(false);
    const [showLoadingOverlay, setShowLoadingOverlay] = createSignal(true);
    let cancelSettleWait: (() => void) | undefined;
    let loadingOverlayFadeTimeout: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => {
        cancelSettleWait?.();
        clearTimeout(loadingOverlayFadeTimeout);
    });
    // status() starts "loading" and flips to "active"/"completed" once
    // loadHistory resolves (subagent-model.ts) — that transition is this
    // view's equivalent of agent-view.tsx's onHistoryReady.
    createEffect((wasLoading: boolean) => {
        const isLoading = status() === "loading";
        if (wasLoading && !isLoading) {
            cancelSettleWait = scheduleOnSettle(() => {
                setHistoryLoaded(true);
                loadingOverlayFadeTimeout = setTimeout(() => setShowLoadingOverlay(false), 220);
            });
        }
        return isLoading;
    }, true);

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
            <Show when={showLoadingOverlay()}>
                <div class="subagent-pane-loading-overlay">
                    <BrainSpinner fading={historyLoaded()} />
                </div>
            </Show>
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
        case "result":
            return (
                <div class="subagent-event-body">
                    <pre class="subagent-event-text">{et.content}</pre>
                </div>
            );
        default:
            return null;
    }
}
