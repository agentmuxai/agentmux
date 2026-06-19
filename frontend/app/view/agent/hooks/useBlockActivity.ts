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

        // Clear on session end (process exit) so a subsequent session in the
        // same pane starts without a stale topic. term:activity is persisted in
        // the block store and survives tab-switch remounts, so we must NOT clear
        // in onCleanup — doing so would blank the label every time the pane
        // remounts (tab switch), and Claude Code only emits OSC titles once per
        // session so no new event would restore it.
        const unsubStatus = waveEventSubscribe({
            eventType: WpsEvent.ControllerStatus,
            scope: makeORef("block", opts.blockId),
            handler: (event) => {
                if ((event as any)?.data?.shellprocstatus === "done") {
                    clearActivity(opts.blockId);
                }
            },
        });

        onCleanup(() => {
            unsub();
            unsubStatus();
            clearTimeout(debounceTimer);
        });
    });
}
