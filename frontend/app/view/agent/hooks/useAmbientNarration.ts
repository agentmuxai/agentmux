// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Ambient narration — short lines describing something AgentMux did on its own,
 * shown in the pane's conversation.
 *
 * First consumer: a tool call the harness detached to the background. The pane
 * goes quiet at that moment and the user is told nothing about why, or about
 * what is still running.
 *
 * WHY THESE ARE MARKED, NOT DISGUISED. The request was for a line that "appears
 * like it is coming from the model". These are rendered in the conversation
 * flow and in the model's voice — but visibly tagged, because the model did not
 * say them and its transcript does not contain them. An unmarked line would
 * read back later as something the model claimed, to a user or to an agent
 * debugging a transcript, and there would be nothing to distinguish it. The tag
 * is one word; the alternative is a small lie told by the UI on every
 * occurrence.
 *
 * Best-effort by construction. The backend caps concurrency and may cancel or
 * fail, in which case nothing arrives. Nothing here gates a UI state change on
 * a narration — the action being narrated has already happened and the pane
 * already reflects it.
 *
 * View-only: these never enter the CLI transcript, so they cannot alter what
 * the model sees on its next turn, and they vanish on reload. See
 * docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md
 * §8.3.
 */

import { createSignal, onCleanup, type Accessor } from "solid-js";

import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";

export interface AmbientNarration {
    /** Which narrated action this was — the backend's prompt selector. */
    kind: string;
    text: string;
    at: number;
}

/**
 * Subscribe to this block's ambient narrations.
 *
 * Bounded: only the most recent few are kept. These are transient asides, not a
 * log — an unbounded list on a long-lived pane would grow without anything ever
 * pruning it, and older entries describe work that has long since finished.
 */
const MAX_RETAINED = 5;

export function useAmbientNarration(blockId: string): Accessor<AmbientNarration[]> {
    const [narrations, setNarrations] = createSignal<AmbientNarration[]>([]);

    const unsub = waveEventSubscribe({
        eventType: WpsEvent.AmbientNarration,
        scope: `block:${blockId}`,
        handler: (event: unknown) => {
            const data = (event as { data?: { text?: unknown; kind?: unknown } } | undefined)?.data;
            const text = typeof data?.text === "string" ? data.text.trim() : "";
            if (!text) return;
            const kind = typeof data?.kind === "string" ? data.kind : "unknown";
            setNarrations((prev) => [...prev, { kind, text, at: Date.now() }].slice(-MAX_RETAINED));
        },
    });
    onCleanup(() => {
        try {
            unsub();
        } catch {
            /* ignore */
        }
    });

    return narrations;
}
