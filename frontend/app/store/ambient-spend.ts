// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Records what AgentMux's own model calls (titles, names, suggestions,
 * previews, narration, running summaries) spend, in the status bar's token
 * totals under `ambient:<purpose>`.
 *
 * srv publishes every call's spend once, as an `ambient:spent` event
 * (`crates/srv/src/ambient/spend.rs`), including the calls it makes on its own
 * that no pane asked for. This is the only place that records it; the panes
 * that ask for a title, a name or a suggestion don't, so nothing is counted twice.
 */

import { muxEventSubscribe } from "./mps";
import { recordTurn, type ServiceUsage } from "./token-usage";

import { WpsEvent } from "@/app/store/mps-events";
export const EVENT_AMBIENT_SPENT = WpsEvent.AmbientSpent;

let installed = false;

export function installAmbientSpendListener(): void {
    if (installed) return;
    installed = true;
    muxEventSubscribe({
        eventType: EVENT_AMBIENT_SPENT,
        handler: (event: MuxEvent) => recordAmbientSpend(event?.data),
    });
}

/** Record one `ambient:spent` payload. Exported for tests. */
export function recordAmbientSpend(data: unknown): void {
    const d = data as { purpose?: unknown; tokens?: ServiceUsage } | undefined;
    if (typeof d?.purpose !== "string" || !d.purpose || !d.tokens) return;
    recordTurn(`ambient:${d.purpose}`, d.tokens);
}
