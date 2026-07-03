// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useBlockActivity — subscribes to `block:activity` WPS events and writes
 * the payload to `term:osc_title` block metadata so the agent-pane tab
 * label shows the Claude Code session topic.
 *
 * The backend emits `block:activity` when it extracts an OSC 0/2
 * window-title sequence from the agent PTY stream (osc_extractor.rs).
 * The topic is a session-level LLM-derived label (e.g. "auth refactor"),
 * not a per-tool-call status — it updates infrequently, and unlike
 * useAgentActivitySummary.ts this is free (no LLM call of our own; the CLI
 * emits the title itself). Owns a distinct meta key from the Haiku-derived
 * `term:ambient_summary` — the two used to share `term:activity` with no
 * ownership protocol, which is what agent-model.ts / swarm-model.ts's
 * precedence logic now resolves. See
 * docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md §3.4 and
 * docs/specs/SPEC_AGENT_OSC_TITLE_ACTIVITY_2026_06_18.md.
 *
 * Terminal panes write the same key via termosc.ts.
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
            "term:osc_title": null,
            "term:ambient_summary": null,
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
                            "term:osc_title": activity,
                        } as any)
                    );
                }, 300);
            },
        });

        // Clear both term:osc_title and term:ambient_summary on session end
        // (process exit) so a subsequent session in the same pane starts
        // without a stale topic/summary from the finished one. These keys
        // are persisted in the block store and survive tab-switch remounts,
        // so we must NOT clear in onCleanup — doing so would blank the label
        // every time the pane remounts (tab switch), and Claude Code only
        // emits OSC titles once per session so no new event would restore it.
        const unsubStatus = waveEventSubscribe({
            eventType: WpsEvent.ControllerStatus,
            scope: makeORef("block", opts.blockId),
            handler: (event) => {
                if ((event as any)?.data?.shellprocstatus === "done") {
                    clearTimeout(debounceTimer);
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
