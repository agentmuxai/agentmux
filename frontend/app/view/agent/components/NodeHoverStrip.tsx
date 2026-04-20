// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NodeHoverStrip — row-level hover strip with timestamp and (later) action
 * buttons. Visibility is pure CSS (.agent-document-node-wrapper:hover and
 * :focus-within). No JS signals for show/hide.
 *
 * SolidJS reactivity: props are never destructured. See AgentDocumentView.tsx
 * (comment above DocumentNodeRenderer) for the reactivity rule and rationale.
 */

import { Show, type JSX } from "solid-js";

interface NodeHoverStripProps {
    timestamp?: number; // Unix ms
    nodeId: string;
}

export const NodeHoverStrip = (props: NodeHoverStripProps): JSX.Element => (
    <Show when={props.timestamp != null}>
        <div
            class="node-strip"
            data-node-strip-for={props.nodeId}
        >
            <time
                class="node-strip-time"
                dateTime={new Date(props.timestamp!).toISOString()}
            >
                {formatLocalized(props.timestamp!)}
            </time>
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
