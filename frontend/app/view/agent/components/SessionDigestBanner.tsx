// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SessionDigestBanner — collapsible AI-generated summary of recent session activity.
 *
 * Shown when the user returns to an agent pane that was idle for >1 hour and has
 * accumulated >20 lines of new activity since the last visit.
 */

import { createSignal, Show, type Accessor, type JSX } from "solid-js";
import { Markdown } from "@/app/element/markdown";

interface SessionDigestBannerProps {
    summary: Accessor<string | null>;
    generatedAt: Accessor<number | null>;
    loading: Accessor<boolean>;
    onDismiss: () => void;
    onRegenerate: () => void;
}

function formatAge(ms: number): string {
    const diff = Date.now() - ms;
    const hours = Math.floor(diff / 3600000);
    if (hours < 1) return "just now";
    if (hours < 24) return `${hours}h ago`;
    return `${Math.floor(hours / 24)}d ago`;
}

export const SessionDigestBanner = (props: SessionDigestBannerProps): JSX.Element => {
    const [expanded, setExpanded] = createSignal(true);

    return (
        <Show when={props.summary() != null || props.loading()}>
            <div class="agent-session-digest">
                <div class="agent-session-digest-header">
                    <button
                        class="agent-session-digest-toggle"
                        onClick={() => setExpanded((v) => !v)}
                        title={expanded() ? "Collapse" : "Expand"}
                    >
                        {expanded() ? "\u25BC" : "\u25B6"}
                    </button>
                    <span class="agent-session-digest-title">
                        Session digest
                        <Show when={props.generatedAt()}>
                            {(ts) => (
                                <span class="agent-session-digest-age">
                                    {" \u00B7 "}{formatAge(ts())}
                                </span>
                            )}
                        </Show>
                    </span>
                    <button
                        class="agent-session-digest-action"
                        onClick={props.onRegenerate}
                        title="Regenerate digest"
                        disabled={props.loading()}
                    >
                        {"\u21BB"}
                    </button>
                    <button
                        class="agent-session-digest-action agent-session-digest-dismiss"
                        onClick={props.onDismiss}
                        title="Dismiss"
                    >
                        {"\u00D7"}
                    </button>
                </div>
                <Show when={expanded() && props.loading()}>
                    <div class="agent-session-digest-loading">Generating digest\u2026</div>
                </Show>
                <Show when={expanded() && !props.loading()}>
                    <div class="agent-session-digest-body">
                        <Show
                            when={props.summary()}
                            fallback={<span class="agent-session-digest-empty">No summary available.</span>}
                        >
                            {(s) => <Markdown text={s()} />}
                        </Show>
                    </div>
                </Show>
            </div>
        </Show>
    );
};

SessionDigestBanner.displayName = "SessionDigestBanner";
