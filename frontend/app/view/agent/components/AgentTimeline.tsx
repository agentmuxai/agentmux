// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentTimeline — compact vertical minimap shown in the right gutter of the
 * agent document view.
 *
 * - Divides the session time range into ~30 activity buckets.
 * - Renders a proportional bar for each bucket (height = message density).
 * - Overlays a thin horizontal line at the current scroll position.
 * - Clicking anywhere on the track calls onJump(fraction) so the parent can
 *   scroll the document to the equivalent position.
 */

import { createMemo, For, Show, type Accessor, type JSX } from "solid-js";
import type { DocumentNode } from "../types";

interface AgentTimelineProps {
    document: Accessor<DocumentNode[]>;
    /** session:start_ts_ms from block meta (ms since epoch) */
    startTsMs: Accessor<number | null>;
    /** session:last_activity_ms from block meta (ms since epoch) */
    endTsMs: Accessor<number | null>;
    /** Current scroll fraction in the document container, 0..1 */
    scrollPosition: Accessor<number>;
    /** Called with 0..1 fraction when the user clicks the track */
    onJump: (fraction: number) => void;
}

const BUCKET_COUNT = 30;

/** Format a ms-epoch timestamp as HH:MM (locale-aware). */
function formatTime(ms: number): string {
    const d = new Date(ms);
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export const AgentTimeline = (props: AgentTimelineProps): JSX.Element => {
    // Divide the session time range into BUCKET_COUNT slices and count nodes
    // per slice. Nodes that carry an explicit `timestamp` field are placed into
    // their true bucket; those without one are distributed uniformly by index.
    const buckets = createMemo(() => {
        const start = props.startTsMs();
        const end = props.endTsMs();
        const nodes = props.document();
        if (!start || !end || end <= start || nodes.length === 0) return [];

        const range = end - start;
        const bucketSize = range / BUCKET_COUNT;
        const counts = new Array<number>(BUCKET_COUNT).fill(0);

        for (let i = 0; i < nodes.length; i++) {
            const n = nodes[i];
            let ts: number;
            if (typeof (n as any).timestamp === "number") {
                ts = (n as any).timestamp as number;
            } else {
                // Fall back to linear interpolation by node index
                ts = start + (range * i) / nodes.length;
            }
            const bucket = Math.min(
                BUCKET_COUNT - 1,
                Math.max(0, Math.floor((ts - start) / bucketSize)),
            );
            counts[bucket]++;
        }

        const maxCount = Math.max(...counts, 1);
        return counts.map((c, i) => ({
            index: i,
            count: c,
            // Height as percentage of the tallest bucket (never 0 — give empty
            // buckets a 1px minimum so the track still looks anchored)
            heightPct: c > 0 ? Math.max(8, (c / maxCount) * 100) : 0,
            tsMs: start + i * bucketSize,
        }));
    });

    const handleClick = (e: MouseEvent) => {
        const target = e.currentTarget as HTMLDivElement;
        const rect = target.getBoundingClientRect();
        const fraction = (e.clientY - rect.top) / rect.height;
        props.onJump(Math.max(0, Math.min(1, fraction)));
    };

    return (
        <Show when={props.startTsMs() != null && props.endTsMs() != null}>
            <div class="agent-timeline" title="Session timeline — click to jump">
                <div class="agent-timeline-label agent-timeline-label--top">
                    {formatTime(props.startTsMs()!)}
                </div>
                <div class="agent-timeline-track" onClick={handleClick}>
                    {/* Activity density bars */}
                    <div class="agent-timeline-bars">
                        <For each={buckets()}>
                            {(b) => (
                                <div
                                    class="agent-timeline-bar"
                                    classList={{ "agent-timeline-bar--empty": b.count === 0 }}
                                    style={{ height: `${b.heightPct}%` }}
                                    title={`${b.count} message${b.count === 1 ? "" : "s"} around ${formatTime(b.tsMs)}`}
                                />
                            )}
                        </For>
                    </div>
                    {/* Current scroll position indicator */}
                    <div
                        class="agent-timeline-scroll-indicator"
                        style={{ top: `${props.scrollPosition() * 100}%` }}
                    />
                </div>
                <div class="agent-timeline-label agent-timeline-label--bottom">
                    {formatTime(props.endTsMs()!)}
                </div>
            </div>
        </Show>
    );
};

AgentTimeline.displayName = "AgentTimeline";
