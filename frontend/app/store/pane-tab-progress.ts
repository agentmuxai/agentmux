// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent's pane tab pill shows about its work: a moving bar while it
 * works, a still line while it waits for the user, is out of quota or is signed
 * out (docs/specs/SPEC_PANE_TAB_PROGRESS_LINE_2026_10_24.md §3.2, §3.3).
 *
 * The frontend never receives srv's `agent_state` string. It receives the inputs
 * srv's `decide()` reads (`controllerstatus` events and the block's meta) and
 * derives the same answer by the same rule, through the two functions the Swarm
 * already shares (`rankAgentStatus`, `unavailableOfMeta`).
 * `test/fixtures/agent-state-decide.json` holds the rows both sides are tested
 * against, so a change to `decide()` that this does not follow fails CI.
 *
 * One subscription per block that has a pill in this window, shared by every
 * consumer and closed with the last one. The value only notifies when the state
 * or its words change, not on every `controllerstatus` tick.
 */
import { MOS } from "@/app/store/global";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { BlockService } from "@/app/store/services";
import { rankAgentStatus } from "@/app/store/agent-status";
import { META_LAST_FAILURE, unavailableLineText, unavailableOfMeta } from "@/app/store/agent-unavailable";
import { createMemo, createRoot, createSignal, onCleanup, createEffect, type Accessor } from "solid-js";

export type PaneTabProgressState = "working" | "waiting" | "limited" | "signed_out";

export interface PaneTabProgress {
    state: PaneTabProgressState;
    /** The state in the Swarm chip's words: "Working", "Waiting for you", "Out of quota until Sun 7 PM", "Signed out". */
    text: string;
}

/** What the rule reads: the controller's two turn fields and the block's meta. */
export interface ProgressFacts {
    /** `is_agent_pane` of the block's controller status. False for a terminal pane. */
    isAgentPane: boolean;
    turnActive: boolean;
    meta: Record<string, unknown> | null | undefined;
    now: number;
}

/** The META key a question sets while it waits for the user (srv's `META_AWAITING_USER`). */
export const META_AWAITING_USER = "term:awaiting_user";

/** srv's `decide()`, as far as a pill shows it: working, waiting, limited or signed out, else nothing. */
export function progressOf(facts: ProgressFacts): PaneTabProgress | null {
    if (!facts.isAgentPane) return null;
    const live = facts.turnActive;
    const unavailable = unavailableOfMeta(facts.meta, live, facts.now);
    const status = rankAgentStatus({
        live,
        needsYou: facts.meta?.[META_AWAITING_USER] === true,
        unavailable: unavailable?.reason ?? null,
    });
    if (status.kind === "needs-you") return { state: "waiting", text: "Waiting for you" };
    if (status.kind === "working") return { state: "working", text: "Working" };
    if (status.kind === "unavailable" && unavailable) {
        const text = unavailable.reason === "signed_out" ? "Signed out" : unavailableLineText(unavailable, facts.now);
        return { state: unavailable.reason, text };
    }
    return null;
}

/** The reset time of a stored usage-limit failure, when the provider gave one. */
function resetsAtOf(meta: Record<string, unknown> | null | undefined): number | undefined {
    const failure = meta?.[META_LAST_FAILURE] as { resetsAtMs?: unknown } | undefined;
    const at = failure?.resetsAtMs;
    return typeof at === "number" && Number.isFinite(at) && at > 0 ? at : undefined;
}

interface Entry {
    value: Accessor<PaneTabProgress | null>;
    refs: number;
    dispose: () => void;
}

const entries = new Map<string, Entry>();

function open(blockId: string): Entry {
    return createRoot((dispose) => {
        const [controller, setController] = createSignal({ isAgentPane: false, turnActive: false });
        // Bumped when a usage limit's reset passes: nothing publishes at that moment, and
        // srv turns `limited` into `idle` by the clock (decide()).
        const [clock, setClock] = createSignal(0);
        const block = MOS.getMuxObjectAtom<Block>(`block:${blockId}`);

        void BlockService.GetControllerStatus(blockId)
            .then((rts) => setController({ isAgentPane: rts?.is_agent_pane === true, turnActive: rts?.turn_active === true }))
            .catch(() => {
                /* no controller yet: not an agent pane */
            });
        const unsub = onMuxEvent(
            WpsEvent.ControllerStatus,
            (data) => setController({ isAgentPane: data.is_agent_pane === true, turnActive: data.turn_active === true }),
            MOS.makeORef("block", blockId)
        );

        const value = createMemo<PaneTabProgress | null>(
            () => {
                clock();
                return progressOf({ ...controller(), meta: block()?.meta, now: Date.now() });
            },
            null,
            { equals: (a, b) => a?.state === b?.state && a?.text === b?.text }
        );

        createEffect(() => {
            const resets = value()?.state === "limited" ? resetsAtOf(block()?.meta) : undefined;
            if (resets === undefined) return;
            const timer = setTimeout(() => setClock((n) => n + 1), Math.max(0, resets - Date.now()) + 1);
            onCleanup(() => clearTimeout(timer));
        });

        return {
            value,
            refs: 0,
            dispose: () => {
                unsub?.();
                dispose();
            },
        };
    });
}

/**
 * The pill's progress for `blockId`: subscribes for the life of the calling
 * component and shares the subscription with every other pill of the block.
 */
export function usePaneTabProgress(blockId: string): Accessor<PaneTabProgress | null> {
    let entry = entries.get(blockId);
    if (!entry) {
        entry = open(blockId);
        entries.set(blockId, entry);
    }
    entry.refs++;
    const held = entry;
    onCleanup(() => {
        held.refs--;
        if (held.refs > 0) return;
        if (entries.get(blockId) === held) entries.delete(blockId);
        held.dispose();
    });
    return held.value;
}

/** Blocks with an open subscription (tests). */
export function openProgressSubscriptions(): number {
    return entries.size;
}
