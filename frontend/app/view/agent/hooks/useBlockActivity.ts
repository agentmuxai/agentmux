// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useBlockActivity — subscribes to `block:activity` WPS events and writes
 * the payload to `term:activity` block metadata so the agent-pane tab
 * label shows the Claude Code session topic.
 *
 * The backend emits `block:activity` when it extracts an OSC 0/2
 * window-title sequence from the agent PTY stream (osc_extractor.rs).
 * The topic is a session-level LLM-derived label (e.g. "auth refactor"),
 * not a per-tool-call status — it updates infrequently.
 *
 * Reuses the existing `term:activity` metadata key so no new tab-bar
 * plumbing is needed — terminal panes write the same key via termosc.ts.
 *
 * See: docs/specs/SPEC_AGENT_OSC_TITLE_ACTIVITY_2026_06_18.md
 */

import { onCleanup, onMount } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { makeORef } from "@/app/store/wos";
import { ObjectService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";

export interface UseBlockActivityOptions {
    blockId: string;
}

function clearActivity(blockId: string): void {
    fireAndForget(() =>
        ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
            "term:activity": null,
        } as any)
    );
}

export function useBlockActivity(opts: UseBlockActivityOptions): void {
    onMount(() => {
        let debounceTimer: ReturnType<typeof setTimeout> | undefined;

        const unsub = waveEventSubscribe({
            eventType: WpsEvent.BlockActivity,
            scope: makeORef("block", opts.blockId),
            handler: (event) => {
                const activity = (event as any)?.data?.activity as string | undefined;
                if (!activity) return;
                clearTimeout(debounceTimer);
                debounceTimer = setTimeout(() => {
                    fireAndForget(() =>
                        ObjectService.UpdateObjectMeta(makeORef("block", opts.blockId), {
                            "term:activity": activity,
                        } as any)
                    );
                }, 300);
            },
        });

        // Clear on pane unmount only. The topic label intentionally persists
        // across turns within a single session — it is a session-level label,
        // not a per-turn status. Claude Code does not emit a title-restore
        // sequence on exit (GitHub #27197), so we clear here to prevent the
        // previous session's topic from appearing when the pane is re-used.
        onCleanup(() => {
            unsub();
            clearTimeout(debounceTimer);
            clearActivity(opts.blockId);
        });
    });
}
