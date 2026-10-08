// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { healthSignals } from "@/app/store/health-signals";
import { formatSince, healthLabel, messageFor } from "@/app/store/health-signals-text";
import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

/**
 * The active performance signals as notices at the top of the backend status
 * panel, worst first, the way the banners used to stack at the top of the
 * window. They come and go live while the panel is open.
 * REPORT_PERFORMANCE_INDICATORS_TO_STATUS_BAR_2026_10_08 §3.5.
 */
export const HealthNotices = (): JSX.Element => {
    const [now, setNow] = createSignal(Date.now());
    onMount(() => {
        const timer = setInterval(() => setNow(Date.now()), 15_000);
        onCleanup(() => clearInterval(timer));
    });

    return (
        <Show when={healthSignals().length > 0}>
            <div class="status-bar-health-notices" role="list" aria-label="Performance">
                <For each={healthSignals()}>
                    {(s) => (
                        <div class={`status-bar-health-notice status-bar-health-notice--${s.level}`} role="listitem">
                            <div class="status-bar-health-notice-head">
                                <span aria-hidden="true">{s.level === "critical" ? "⚠" : "▲"}</span>
                                <span class="status-bar-health-notice-label">{healthLabel(s.kind, s.payload)}</span>
                                <span class="status-bar-health-notice-since">{formatSince(now() - s.since)}</span>
                            </div>
                            <div class="status-bar-health-notice-text">{messageFor(s.kind, s.level, s.payload)}</div>
                        </div>
                    )}
                </For>
            </div>
            <div class="status-bar-popover-divider" />
        </Show>
    );
};

HealthNotices.displayName = "HealthNotices";
