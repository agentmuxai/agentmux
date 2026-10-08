// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useNextPromptSuggestion — ghost-text "predicted next user message" in the
 * agent pane composer.
 *
 * Mirrors Claude Code CLI's own interactive-mode feature (a dimmed
 * suggestion shown in the empty input box after a turn finishes, Tab
 * accepts it): that native mechanism is unreachable from AgentMux because
 * it's unconditionally suppressed in non-interactive (`-p`) mode, which is
 * how AgentMux always drives the CLI. This reimplements the same UX as a
 * second Ambient Model Call gateway purpose. See
 * docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md.
 *
 * Fires exactly once per genuine, backend-confirmed turn completion — see
 * `turnJustEndedAtom` below, NOT on `TurnPhase.kind === "Done"` (mirrors
 * useAgentActivitySummary.ts's identical fix — see that module's doc
 * comment and docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md
 * for the full diagnosis of why `Done` over-triggers). Sends the pane's recent
 * conversation (`ambient-activity.ts`) to the ambient model and asks for a
 * short next user message. The result is written to `term:next_prompt_suggestion`
 * block meta, which the composer reads as ghost text (dimmed, shown only
 * while the input is empty; Tab accepts it into the real input). Typing
 * over it, or accepting it and then deleting the text, does NOT dismiss it
 * from block meta — the composer just stops rendering it while non-empty
 * (native `<textarea placeholder>` behavior) and shows it again once the
 * box is empty, per
 * docs/specs/SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md —
 * see AgentFooter.tsx's placeholder precedence comment.
 *
 * Every write to `term:next_prompt_suggestion` FROM THIS FILE (a fresh
 * suggestion, or guard 1's clear) is paired with a bump to
 * `term:next_prompt_suggestion_gen` (`suggestionGenCounter` below, a
 * monotonic counter) via `writeSuggestionMeta` — AgentFooter.tsx masks a
 * *specific write*, not a specific text value, when it snapshots this pair
 * at send time (§9/§10 of that spec). Text-value comparison alone has a
 * real collision: if a later turn's genuinely fresh suggestion happens to
 * be the exact same string as the one masked at the previous send (a
 * plausible repeat, e.g. "Run the tests"), value-equality would suppress a
 * legitimate current suggestion until the next send. The generation
 * counter can't collide that way — reagentx P1 on #2515.
 *
 * One exception: guard 4's clear (below) is NOT a write from this file —
 * `useBlockActivity.ts`'s `clearActivity` writes
 * `term:next_prompt_suggestion: null` directly, alongside its own
 * `term:osc_title`/`term:ambient_summary` clears, bypassing
 * `writeSuggestionMeta` and never touching the gen key. Harmless today only
 * because the resulting `null` is falsy and short-circuits
 * `AgentFooter.tsx`'s `suggestion && ...` check before the gen comparison
 * is ever reached — not because the invariant above actually holds for
 * that path. Flagged, not fixed: routing guard 4 through this file's
 * counter would need exporting it across a module boundary that was
 * deliberately kept separate (module doc below, guard 4: "NOT by this
 * hook... one ControllerStatus subscription per pane") for a benefit that
 * doesn't exist today — reagentx P2 on #2515 (second re-review).
 *
 * Unlike the read-only activity summary (which persists across turns), a
 * stale suggestion here is a correctness bug, not a cosmetic one — it can
 * put words in the user's mouth. Four independent guards:
 *   1. Cleared the instant a new turn starts (`Submitting`) — a suggestion
 *      from turn N is meaningless (and misleading) once turn N+1 has begun.
 *      `term:ambient_summary` deliberately persists across turns; this key
 *      deliberately does not.
 *   2. `activeTurnId !== myTurnId` at write time — a newer turn has already
 *      started (same belt-and-suspenders pattern as
 *      useAgentActivitySummary.ts, on top of the backend gateway's own
 *      cancellation).
 *   3. `!opts.isComposerEmpty()` at write time — the user has started typing
 *      their own message since this request was issued. Checked when the
 *      response arrives (not just "was the composer empty when we sent the
 *      request") specifically to close the race where the RPC resolves
 *      *after* the user already typed something: without this check, a late
 *      response could silently overwrite what the user is actively typing.
 *      This is the only guard against that race — AgentFooter.tsx's
 *      `handleInput` does NOT also clear the suggestion on typing (it used
 *      to; that was removed as redundant with this guard and was itself the
 *      cause of a bug — see
 *      docs/specs/SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md).
 *   4. Cleared on session end (process exit) — but NOT by this hook.
 *      useBlockActivity.ts's `clearActivity` owns that transition (it
 *      already clears `term:osc_title`/`term:ambient_summary` there) and
 *      clears this key too, so a suggestion from a finished session can't
 *      persist into a brand-new session started in the same pane. Don't
 *      duplicate that listener here — one `ControllerStatus` subscription
 *      per pane for this purpose is enough.
 *
 * Before any of these, `shouldRequestSuggestion` skips the call itself when
 * the user is already typing or the turn failed or was stopped, and the
 * backend skips it when the turn ended waiting for the user. See
 * docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md section 6.2.
 */

import { createEffect, on, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { makeORef } from "@/app/store/mos";
import { ObjectService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";
import { AMBIENT_PULL_TIMEOUT_MS } from "./ambient-rpc";
import { recentActivityEntries } from "../ambient-activity";
import type { DocumentNode } from "../types";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";
import { META_SUGGESTION, META_SUGGESTION_GEN } from "@/app/store/meta-keys";

export interface UseNextPromptSuggestionOptions {
    blockId: string;
    turnPhase: Accessor<TurnPhase>;
    /** See useAgentActivitySummary.ts's identically-named option for what
     *  this is and why it replaces a `TurnPhase.kind === "Done"` trigger. */
    turnJustEndedAtom: Accessor<number>;
    /** Checked at write time — see the module doc comment, guard 3. */
    isComposerEmpty: () => boolean;
    /** The pane's document. Its recent conversation is sent with the request
     *  (`ambient-activity.ts`), so the server need not read it back from the
     *  output file, which it can parse only for Claude. */
    document?: Accessor<DocumentNode[]>;
}

// Shared across every pane's hook instance — module-level, not per-mount.
// Doesn't need to be scoped per block: a strictly-increasing counter is
// still strictly increasing (and still unique per write) when shared, and
// sharing it avoids per-instance bookkeeping for no benefit — AgentFooter.tsx
// only ever compares one block's own gen against its own earlier snapshot,
// never across blocks.
let suggestionGenCounter = 0;

function writeSuggestionMeta(blockId: string, suggestion: string | null): void {
    fireAndForget(() =>
        ObjectService.UpdateObjectMeta(makeORef("block", blockId), {
            [META_SUGGESTION]: suggestion,
            [META_SUGGESTION_GEN]: ++suggestionGenCounter,
        } as any)
    );
}

function clearSuggestion(blockId: string): void {
    writeSuggestionMeta(blockId, null);
}

/**
 * Whether `phase` rules out continuing the turn: it failed, was stopped, or is
 * being stopped. A stop's `Interrupting` can still be the phase when the
 * backend's turn-end edge arrives (#4476). `Idle` and `Streaming` don't rule it
 * out: the edge can also arrive before `TurnEnd`.
 */
function turnWasCutShort(phase: TurnPhase): boolean {
    return phase.kind === "Interrupting" || (phase.kind === "Done" && phase.outcome !== "completed");
}

/**
 * Whether a turn's end is worth a suggestion call. Not while the user is already
 * typing: the reply would be thrown away by guard 3 anyway, after being paid for.
 * Not after a turn that failed or was stopped: there is no finished work to
 * continue. The backend makes the remaining checks that need the transcript
 * (`digest::TurnEnding`).
 */
export function shouldRequestSuggestion(phase: TurnPhase, composerEmpty: boolean): boolean {
    return composerEmpty && !turnWasCutShort(phase);
}

export function useNextPromptSuggestion(opts: UseNextPromptSuggestionOptions): void {
    const { blockId, turnPhase, turnJustEndedAtom, isComposerEmpty } = opts;

    // Scoped to this mount — see useAgentActivitySummary.ts's doc comment for
    // why the wire `generation` is Date.now() instead of this counter.
    let activeTurnId = 0;
    // Remembers whether the MOST RECENT Submitting was a hidden turn
    // (memory-reinjection-controller.ts), read by the turnJustEndedAtom effect
    // below, which has no TurnPhase of its own to inspect. A hidden turn's end
    // makes no call: there is no human turn to continue (#3502). Its content
    // never reaches the activity this hook sends (the
    // document leaves hidden turns out), and the server's fallback read of the
    // output file refuses while a reinjection is in progress
    // (`digest::is_hidden_reinjection_active`).
    let lastTurnWasHidden = false;

    createEffect(on(turnPhase, (phase) => {
        if (phase.kind === "Submitting") {
            activeTurnId++;
            lastTurnWasHidden = phase.hidden === true;
            clearSuggestion(blockId); // guard 1 — see module doc comment
        }
    }));

    // `defer: true` — skip the run at mount, same reasoning as
    // useAgentActivitySummary.ts.
    createEffect(on(turnJustEndedAtom, () => {
        if (lastTurnWasHidden) return; // see lastTurnWasHidden's own doc comment above
        if (!shouldRequestSuggestion(turnPhase(), isComposerEmpty())) return;
        const myTurnId = activeTurnId;

        RpcApi.NextPromptSuggestionCommand(
            TabRpcClient,
            { block_id: blockId, generation: Date.now(), activity: opts.document && recentActivityEntries(opts.document()) },
            { timeout: AMBIENT_PULL_TIMEOUT_MS },
        ).then((result) => {
            if (activeTurnId !== myTurnId) return; // superseded by a newer turn
            // Checked again: the turn's outcome may have settled while the call ran.
            if (result.suggestion && isComposerEmpty() && !turnWasCutShort(turnPhase())) {
                writeSuggestionMeta(blockId, result.suggestion);
            }
        }).catch(() => {
            // Silently ignore — no ghost text shows.
        });
    }, { defer: true }));
}
