// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * When the History reader shows what it has parsed (spec §6.9: at most once a second, never while
 * hidden). Split out of AgentHistoryView.
 */

import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { onCleanup, untrack } from "solid-js";
import { HISTORY_PUBLISH_MIN_INTERVAL_MS, type HistoryReader } from "./history-reader-state";

export function useHistoryPublishing(s: HistoryReader): void {
    const visibility = usePaneTabVisibility(s.props.blockId);
    const hidden = (): boolean => visibility() !== "active";
    s.hidden = hidden;
    const publishNow = (): void => {
        clearTimeout(s.publishTimer);
        s.publishTimer = undefined;
        // Hidden since the timer was armed: keep it for the reveal.
        if (untrack(hidden)) {
            s.dirty = true;
            return;
        }
        s.dirty = false;
        s.lastPublishAt = Date.now();
        s.setRawNodes(s.parser.nodes.slice());
    };
    const publishSoon = (): void => {
        s.dirty = true;
        if (s.publishTimer !== undefined || hidden()) return;
        const wait = Math.max(0, s.lastPublishAt + HISTORY_PUBLISH_MIN_INTERVAL_MS - Date.now());
        s.publishTimer = setTimeout(publishNow, wait);
    };
    onCleanup(() => clearTimeout(s.publishTimer));
    Object.assign(s.act, { publishNow, publishSoon });
}
