// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentActivitySummary — drives the session-goal title in the agent pane header.
 *
 * Fires when a turn ENTERS `Submitting` — i.e. right when the user submits a
 * new message — not on turn completion. The goal a session is working toward
 * can only change at the point the user says something new; the agent's own
 * tool calls afterward never change it, so there's no reason to wait for a
 * (possibly long, multi-tool-call) turn to finish before re-evaluating the
 * title. This also means the backend no longer needs to read a FileStore
 * output tail for this call — `TurnPhase.Submitting.pendingContent` already
 * carries the literal text just submitted (threaded through via
 * `TurnStart.content`, see agent-pane-state/reducer.ts).
 *
 * Sends that text, plus the CURRENTLY DISPLAYED title (already in block
 * meta), to the ambient model and asks it to maintain a stable,
 * PR-title-style summary of the session's OVERALL GOAL: it replies SKIP while
 * the current title still fits, and writes a new one only for a genuinely
 * new or expanded goal (`ambient::prompt::build_session_title_prompt`). This replaces the previous "what is currently being
 * worked on" per-turn micro-activity phrasing, which regenerated from a
 * blank slate every call and had no way to recognize "this is still the same
 * task." See docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md.
 *
 * The backend stores the result in the `term:ambient_summary` block meta key
 * itself, keeping the current title unless the new one names a different goal
 * (`crates/srv/src/ambient/title.rs`, shared with its empty-title recovery).
 * agent-model.ts and swarm-model.ts read that key (preferring it over the free
 * `term:osc_title` signal — see
 * docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md §3.4).
 *
 * Word target is derived from pane width so narrow panes get ~5 words and wide
 * panes can accommodate up to 12.
 *
 * This call is routed through the backend's Ambient Model Call gateway
 * (`crate::ambient`), keyed by block_id, which cancels a call superseded by a
 * newer message. The gateway's per-block generation outlives this hook's mount
 * (a tab switch remounts it), so the wire `generation` is `Date.now()`, which
 * only increases, not a per-mount counter, which would restart lower than the
 * gateway's last one and be refused as stale.
 *
 * The summary is never cleared on our own — it persists across turns so the
 * header always shows the last known title. It's cleared elsewhere
 * (useBlockActivity.ts) when the underlying session ends.
 */

import { createEffect, on, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { makeORef } from "@/app/store/mos";
import { ObjectService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";
import { AMBIENT_PULL_TIMEOUT_MS } from "./ambient-rpc";
import { isUsableTitle } from "@/app/store/ambient-title";
import { lastPromptToStore, META_LAST_PROMPT } from "@/app/store/swarm-line";
import { META_HUMAN_TURNS, nextHumanTurn, shouldRequestTitle } from "@/app/store/title-schedule";
import { MOS } from "@/app/store/global";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";
import { META_TITLE } from "@/app/store/meta-keys";

export interface UseAgentActivitySummaryOptions {
    blockId: string;
    turnPhase: Accessor<TurnPhase>;
    getRootWidth: () => number | undefined;
}

export function useAgentActivitySummary(opts: UseAgentActivitySummaryOptions): void {
    const { blockId, turnPhase, getRootWidth } = opts;

    // `defer: true` — skip the run at mount. A freshly-opened pane whose
    // live turnPhase happens to already be Submitting (e.g. reattaching
    // mid-turn) shouldn't immediately fire; the next genuine submission will.
    createEffect(on(turnPhase, (phase) => {
        if (phase.kind !== "Submitting") return;
        // A hidden turn (memory-reinjection-controller.ts) must never
        // reach an ambient LLM call whose result becomes the human-visible
        // pane title — reagentx P0 on PR #3502: this effect was forwarding
        // `pendingContent` unconditionally, regardless of source, which for
        // a hidden reinjection turn meant the full Global+Personal memory
        // bodies got sent to Haiku and could surface a summary of them in
        // the header. See TurnStart's `hidden` field doc comment
        // (agent-pane-state/types.ts) for why this is defense-in-depth on
        // top of, not instead of, never putting real content in
        // `pendingContent` for a hidden turn in the first place.
        if (phase.hidden) return;
        // Remember what the user asked, for the swarm row's fallback line when no
        // generated title exists (store/swarm-line.ts). Only a message with a goal
        // in it is kept, so "u there" never overwrites the real one. Written now,
        // not after the model call, so it is there even if that call fails.
        const lastPrompt = lastPromptToStore(phase.pendingContent);
        // Count this human message, then decide whether it is a turn on which the
        // title is (re)computed: every message while there is no usable title, and
        // only turns 2, 5, 8, then every third once there is one
        // (store/title-schedule.ts). Read from block meta, not a local counter, so
        // a remount (tab switch) does not restart the schedule.
        const meta = MOS.getMuxObjectAtom<Block>(`block:${blockId}`)()?.meta;
        const turn = nextHumanTurn(blockId, meta?.[META_HUMAN_TURNS]);
        const currentTitle = meta?.[META_TITLE];
        const hasTitle = typeof currentTitle === "string" && isUsableTitle(currentTitle);
        fireAndForget(() =>
            ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
                [META_HUMAN_TURNS]: turn,
                ...(lastPrompt ? { [META_LAST_PROMPT]: lastPrompt } : {}),
            } as any)
        );
        if (!shouldRequestTitle(hasTitle, turn)) return;
        const rootWidth = getRootWidth() ?? 400;
        const textWidth = Math.max(0, rootWidth - 280);
        const wordTarget = Math.max(5, Math.min(12, Math.floor(textWidth / 48)));

        // The backend stores the title and reports the spend (ambient/spend.rs).
        RpcApi.AgentActivitySummaryCommand(
            TabRpcClient,
            {
                block_id: blockId,
                word_target: wordTarget,
                generation: Date.now(),
                user_message: phase.pendingContent,
            },
            { timeout: AMBIENT_PULL_TIMEOUT_MS },
        ).catch(() => {
            // Silently ignore — the header just stays on its last title.
        });
    }, { defer: true }));
}
