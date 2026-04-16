// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";

interface NodeTimestampProps {
    timestamp?: number; // Unix ms
}

function formatTime(ms: number): string {
    const d = new Date(ms);
    const h = String(d.getHours()).padStart(2, "0");
    const m = String(d.getMinutes()).padStart(2, "0");
    const s = String(d.getSeconds()).padStart(2, "0");
    const t = Math.floor(d.getMilliseconds() / 100); // tenths
    return `${h}:${m}:${s}.${t}`;
}

/**
 * Floating timestamp pill shown on node row hover.
 * Visibility is controlled entirely by CSS (.doc-node:hover .node-ts).
 * No JS state is involved — zero overhead when not hovered.
 */
export const NodeTimestamp = (props: NodeTimestampProps): JSX.Element => (
    <Show when={props.timestamp != null}>
        <span class="node-ts">{formatTime(props.timestamp!)}</span>
    </Show>
);

NodeTimestamp.displayName = "NodeTimestamp";
