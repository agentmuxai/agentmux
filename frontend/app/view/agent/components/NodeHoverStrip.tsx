// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NodeHoverStrip — row-level hover strip with timestamp and action buttons.
 * Visibility is pure CSS (.agent-document-node-wrapper:hover and :focus-within).
 * No JS signals for show/hide.
 *
 * SolidJS reactivity: props are never destructured. See AgentDocumentView.tsx
 * (comment above DocumentNodeRenderer) for the reactivity rule and rationale.
 */

import { Show, type JSX } from "solid-js";

interface NodeHoverStripProps {
    timestamp?: number; // Unix ms
    nodeId: string;
    isBookmarked?: boolean;
    onBookmark?: () => void;
}

interface StripButtonProps {
    icon: string;
    label: string;
    active?: boolean;
    onClick?: () => void;
}

const StripButton = (props: StripButtonProps): JSX.Element => {
    const disabled = () => props.onClick == null;
    return (
        <button
            type="button"
            class="node-strip-btn"
            classList={{
                "node-strip-btn--active": props.active === true,
                "node-strip-btn--disabled": disabled(),
            }}
            disabled={disabled()}
            onClick={(e) => {
                e.stopPropagation();
                props.onClick?.();
            }}
            title={props.label}
            aria-label={props.label}
        >
            {props.icon}
        </button>
    );
};

export const NodeHoverStrip = (props: NodeHoverStripProps): JSX.Element => (
    <Show when={props.timestamp != null || props.onBookmark != null}>
        <div
            class="node-strip"
            data-node-strip-for={props.nodeId}
        >
            <Show when={props.timestamp != null}>
                <time
                    class="node-strip-time"
                    dateTime={new Date(props.timestamp!).toISOString()}
                >
                    {formatLocalized(props.timestamp!)}
                </time>
            </Show>
            <Show when={props.onBookmark}>
                <StripButton
                    icon="🔖"
                    label={props.isBookmarked ? "Remove bookmark" : "Bookmark"}
                    active={props.isBookmarked === true}
                    onClick={props.onBookmark}
                />
            </Show>
        </div>
    </Show>
);

NodeHoverStrip.displayName = "NodeHoverStrip";

/**
 * Localized weekday + date + 12-hour AM/PM time. Adds year when ≥7 days old.
 */
function formatLocalized(ms: number): string {
    const d = new Date(ms);
    const ageDays = (Date.now() - ms) / 86_400_000;
    return new Intl.DateTimeFormat(undefined, {
        weekday: "short",
        month: "short",
        day: "numeric",
        year: ageDays >= 7 ? "numeric" : undefined,
        hour: "numeric",
        minute: "2-digit",
        second: "2-digit",
        hour12: true,
    }).format(d);
}
