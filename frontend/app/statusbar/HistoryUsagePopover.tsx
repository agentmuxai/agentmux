// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * HistoryUsagePopover: where the agent history on this machine is kept, from
 * the status-bar total. Read-only. Nothing here deletes history, because
 * AgentMux keeps it forever. The sibling of DiskVolumesPopover.
 */

import { createSignal, For, Show, type JSX } from "solid-js";
import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import type { HistoryUsage } from "@/types/rpc/HistoryUsage";
import { formatBytes } from "@/util/format-bytes";

interface HistoryUsagePopoverProps {
    anchor: PopoverAnchor;
    /** The last measurement, or null before the first arrives. */
    usage: HistoryUsage | null;
    onRefresh: () => Promise<void>;
    onClose: () => void;
}

/** "5 min ago", "just now": when the numbers were measured. */
export function measuredAgo(computedAtMs: number, nowMs: number): string {
    const min = Math.floor(Math.max(0, nowMs - computedAtMs) / 60_000);
    if (min < 1) return "just now";
    if (min < 60) return `${min} min ago`;
    return `${Math.floor(min / 60)} h ago`;
}

export const HistoryUsagePopover = (props: HistoryUsagePopoverProps): JSX.Element => {
    const [refreshing, setRefreshing] = createSignal(false);
    const refresh = async () => {
        setRefreshing(true);
        try {
            await props.onRefresh();
        } finally {
            setRefreshing(false);
        }
    };
    const share = (bytes: number): number => {
        const total = props.usage?.total_bytes ?? 0;
        return total > 0 ? Math.min(100, (bytes / total) * 100) : 0;
    };

    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-end"
            onDismiss={props.onClose}
            class="history-usage-popover"
            role="dialog"
            aria-label="Agent history kept on this machine"
            style={{ width: "340px" }}
        >
            <div class="history-usage-header">
                <span class="history-usage-title">Agent history</span>
                <span class="history-usage-note">kept forever</span>
            </div>
            <Show when={props.usage} fallback={<div class="history-usage-empty">Measuring…</div>}>
                {(usage) => (
                    <>
                        <div class="history-usage-total">{formatBytes(usage().total_bytes)}</div>
                        <div class="history-usage-list">
                            <For each={usage().stores}>
                                {(s) => (
                                    <div class="history-usage-row" data-store={s.key}>
                                        <span class="history-usage-label">{s.label}</span>
                                        <span class="history-usage-bar" aria-hidden="true">
                                            <span class="history-usage-bar-fill" style={{ width: `${share(s.bytes)}%` }} />
                                        </span>
                                        <span class="history-usage-bytes">{formatBytes(s.bytes)}</span>
                                    </div>
                                )}
                            </For>
                        </div>
                        <Show when={usage().agents.length > 0}>
                            <div class="history-usage-subtitle">Largest pane histories</div>
                            <div class="history-usage-list">
                                <For each={usage().agents}>
                                    {(a) => (
                                        <div class="history-usage-row" data-agent={a.definition_id}>
                                            <span class="history-usage-label">{a.name}</span>
                                            <span class="history-usage-bytes">{formatBytes(a.bytes)}</span>
                                        </div>
                                    )}
                                </For>
                            </div>
                        </Show>
                        <div class="history-usage-footer">
                            <span class="history-usage-note">Measured {measuredAgo(usage().computed_at_ms, Date.now())}</span>
                            <button type="button" class="history-usage-refresh" disabled={refreshing()} onClick={() => void refresh()}>
                                {refreshing() ? "Measuring…" : "Measure again"}
                            </button>
                        </div>
                    </>
                )}
            </Show>
        </AnchoredPopover>
    );
};

HistoryUsagePopover.displayName = "HistoryUsagePopover";
