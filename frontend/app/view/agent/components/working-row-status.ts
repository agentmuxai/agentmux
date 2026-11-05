// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Working row's live status line: what it says now, and how it types out. */

import { createEffect, createMemo, on, onCleanup, untrack } from "solid-js";

import { shouldReduceMotion } from "@/app/motion/motion";
import { turnOpen, turnSettling } from "@/app/store/agent-pane-state/turn-ledger";
import { presentStatus, TIMING } from "../status/present-status";
import { rowStatusWords } from "../status/row-status";
import { leadInText } from "./working-row-model";
import type { RowState } from "./working-row-state";

/**
 * The line and the signals it comes from. Between two passes of one turn no pass runs and the next is
 * expected: the row keeps its live form (dimmed) rather than flash "Worked" and take it back a moment later.
 * Display only: the composer's busy gate is not this.
 *
 * First display of a new left-zone string (a phrase, summary, or status change) reveals
 * character-by-character. The gradient highlight that used to sweep over it afterwards is gone: the text is
 * one solid color now (SPEC_AGENT_WORKING_ROW_MONO_SUMMARY_2026_10_02.md); the ASCII spinner beside it carries
 * the "still working" signal. Originally SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md §2.
 *
 * When the motion policy reduces `transition` (motion/motion.ts; dormant today), skips the one-shot type-out
 * and jumps straight to the full text.
 */
export function useRowStatus(s: RowState) {
    const props = s.props;
    const reducedMotion = () => shouldReduceMotion("transition");
    const settling = createMemo(() => turnSettling(props.turnLedger, (s.tick(), Date.now())));
    const live = createMemo(() => props.loading || settling());
    const status = createMemo(() => {
        const now = (s.tick(), Date.now());
        if (live() && !s.wasLive) s.statusMemory = null;
        s.wasLive = live();
        const l = props.turnLedger;
        const words = rowStatusWords(
            {
                live: live() || !!props.compacting || !!props.reconnecting,
                needsYou: props.needsYou ?? null,
                reconnecting: !!props.reconnecting,
                compacting: !!props.compacting,
                stopping: !!props.stopping,
                waitingReason: props.waitingReason,
                retryAfterMs: props.retryAtMs != null ? Math.max(0, props.retryAtMs - now) : null,
                launchPhase: props.launchPhase,
            },
            now,
        );
        const r = presentStatus(
            {
                nowMs: now,
                ...words,
                leadIn: leadInText(props, now),
                activity: props.activity ?? null,
                turnStartedAt: l && turnOpen(l, now) ? l.startedAtMs : untrack(s.loadStartMs),
                goal: props.activitySummary?.trim() || null,
                phrase: s.phrase(),
                instantReveal: untrack(reducedMotion),
            },
            s.statusMemory,
        );
        s.statusMemory = r.memory;
        return r.line;
    });
    const leftText = () => status().text;
    // Deduplicated: `on()` re-runs whenever its source fires, and `status` is a
    // new object every tick, so `on(() => status().key)` re-typed it each second.
    const statusKey = createMemo(() => status().key);

    // Every new line types out, and so does the line on screen when the row
    // goes live; a moved counter (same key) never re-types it. The presenter's
    // dwell counts from the end of this type-out.
    createEffect(
        on([statusKey, live], () => {
            const text = untrack(leftText);
            if (untrack(reducedMotion) || !text || !untrack(live)) {
                s.setRevealed(Number.POSITIVE_INFINITY);
                return;
            }
            s.setRevealed(0);
            // By the clock, so a throttled timer catches up (the dwell's clock too).
            const startedAt = Date.now();
            const id = setInterval(() => {
                const n = Math.floor((Date.now() - startedAt) / TIMING.revealCharMs);
                if (n < untrack(leftText).length) return void s.setRevealed(n);
                clearInterval(id);
                s.setRevealed(Number.POSITIVE_INFINITY);
            }, TIMING.revealCharMs);
            onCleanup(() => clearInterval(id));
        }),
    );

    return { settling, live, leftText };
}
