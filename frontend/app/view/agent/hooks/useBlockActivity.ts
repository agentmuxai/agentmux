// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useBlockActivity — subscribes to `block:activity` MPS events and writes
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
 *
 * This is also the one place that reacts to session end (`shellprocstatus
 * === "done"`), so `clearActivity` doubles as the session-end clear point
 * for `term:next_prompt_suggestion` (useNextPromptSuggestion.ts) too —
 * without it, a ghost-text suggestion from a finished session would persist
 * into a brand-new session started in the same pane and render as the
 * composer placeholder before any new turn completes. That key gets its own
 * mid-session clears (new turn start, superseded turn) from
 * useNextPromptSuggestion.ts directly; this hook only owns the
 * session-boundary case, matching term:osc_title/term:ambient_summary.
 */

import { onCleanup, onMount } from "solid-js";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { makeORef } from "@/app/store/mos";
import { ObjectService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";
import { MOS } from "@/app/store/global";
import { isUsableTitle } from "@/app/store/ambient-title";
import { META_AWAITING_USER, META_LAST_PROMPT, META_RESTORED } from "@/app/store/swarm-line";
import { META_HUMAN_TURNS, resetHumanTurns } from "@/app/store/title-schedule";
import { META_OSC_TITLE, META_SUGGESTION, META_TITLE } from "@/app/store/meta-keys";

export interface UseBlockActivityOptions {
    blockId: string;
}

function clearActivity(blockId: string): void {
    // The live title is cleared so the next session does not start under the
    // finished one's topic, but it is not thrown away: it moves to
    // `term:restored_summary`, which the swarm row shows (muted, labelled) until
    // a fresh title exists. Without this a restart opened onto a blank row.
    // Block meta persists across a restart, so this needs no new store.
    // docs/specs/SPEC_AMBIENT_SWARM_SUMMARY_HARDENING_2026_10_02.md section 5.5.
    const ended = MOS.getMuxObjectAtom<Block>(`block:${blockId}`)()?.meta?.[META_TITLE];
    const restored = typeof ended === "string" && isUsableTitle(ended) ? ended.trim() : null;
    resetHumanTurns(blockId);
    fireAndForget(() =>
        ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
            [META_OSC_TITLE]: null,
            [META_TITLE]: null,
            [META_SUGGESTION]: null,
            // The session is over: what it was asked and whether it was waiting
            // belong to it. Left in place, a new session in this pane, or one
            // after a restart, would open under the old session's last message
            // (ReAgent P2 on #4234).
            [META_LAST_PROMPT]: null,
            [META_AWAITING_USER]: null,
            // The title schedule counts this session's human turns from one.
            [META_HUMAN_TURNS]: null,
            // Only replaced by a title that was actually worth keeping: a session
            // that never got one leaves the previous restored title in place.
            ...(restored ? { [META_RESTORED]: restored } : {}),
        } as any)
    );
}

export function useBlockActivity(opts: UseBlockActivityOptions): void {
    onMount(() => {
        let debounceTimer: ReturnType<typeof setTimeout> | undefined;

        const unsub = muxEventSubscribe({
            eventType: WpsEvent.BlockActivity,
            scope: makeORef("block", opts.blockId),
            handler: (event) => {
                const activity = (event as any)?.data?.activity as string | undefined;
                if (!activity) return;
                clearTimeout(debounceTimer);
                debounceTimer = setTimeout(() => {
                    fireAndForget(() =>
                        ObjectService.UpdateObjectMeta(makeORef("block", opts.blockId), {
                            [META_OSC_TITLE]: activity,
                        } as any)
                    );
                }, 300);
            },
        });

        // Clear term:osc_title, term:ambient_summary, and
        // term:next_prompt_suggestion on session end (process exit) so a
        // subsequent session in the same pane starts without a stale
        // topic/summary/ghost-text-suggestion from the finished one. These
        // keys are persisted in the block store and survive tab-switch
        // remounts, so we must NOT clear in onCleanup — doing so would blank
        // the label every time the pane remounts (tab switch), and Claude
        // Code only emits OSC titles once per session so no new event would
        // restore it.
        const unsubStatus = muxEventSubscribe({
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
