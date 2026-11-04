// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The transcript's follow plumbing that doesn't depend on its layout: the
 * `[scroll-follow]` transition log (I4) and the shape of the user scroll input
 * the follow controller provides. Used by
 * virtualization/AgentDocumentVirtualList.tsx.
 * SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md.
 */

import { createEffect, untrack, type Accessor } from "solid-js";

import { followStateLabel, intentOf, type FollowState } from "./follow-controller";

/** What the transcript asks about the user's scroll input. */
export interface ScrollInput {
    /** Inside the user-input window, or holding the scrollbar. */
    windowOpen(): boolean;
    /** A pointer is held anywhere on the transcript (a selection or scrollbar drag). */
    pointerHeld(): boolean;
    scrollbarHeld(): boolean;
}

/** Before the scroller mounts: no input yet. */
export const NO_SCROLL_INPUT: ScrollInput = {
    windowOpen: () => false,
    pointerHeld: () => false,
    scrollbarHeld: () => false,
};

export interface FollowLog {
    /** Publish `cause`, then apply a stickToBottom change; logs it once. */
    transitionFollow(cause: string, detail: string | undefined, apply: () => void): void;
    /** Project a follow-controller transition onto stickToBottom (and log it). */
    project(from: FollowState, to: FollowState, cause: string, detail?: string): void;
}

/**
 * ── Follow-state transition log (spec invariant I4) ──────────────────
 * Every stickToBottom change emits exactly one line:
 *   [scroll-follow] pane=abc1234 following→detached cause=user-scroll gap=412px
 * so a pane that stopped following always says why in
 * `muxlog fe grep scroll-follow`. Bug A (spec §2.1) was invisible for
 * months precisely because its disengage path logged nothing.
 *
 * Call sites go through transitionFollow(), which publishes its cause
 * BEFORE applying the change. Solid may run the effect below either
 * synchronously inside apply() (a setter called outside an update) or
 * later (a setter called from inside another effect); whichever of the
 * two observes the change first logs it, with that cause, exactly once —
 * `lastLoggedFollow` is the dedupe. A change that did NOT come through
 * transitionFollow() has no published cause and logs as cause=external,
 * so no path can be silent. `data-follow-state` on the scroller mirrors
 * the state for CDP / tests. Call during component setup (it creates an effect).
 */
export function createFollowLog(
    paneTag: () => string,
    viewState: { stickToBottom: Accessor<boolean>; engageStickToBottom(): void; disengageStickToBottom(): void }
): FollowLog {
    const followLabel = (on: boolean): string => (on ? "following" : "detached");
    let lastLoggedFollow = untrack(viewState.stickToBottom);
    let pendingFollowCause: { cause: string; detail?: string } | null = null;
    const line = (change: string, cause: string, detail?: string): void =>
        console.info("[scroll-follow]", `pane=${paneTag()}`, change, `cause=${cause}${detail ? ` ${detail}` : ""}`);
    const logFollowIfChanged = (now: boolean): void => {
        if (now === lastLoggedFollow) return;
        const { cause, detail } = pendingFollowCause ?? { cause: "external" };
        line(`${followLabel(lastLoggedFollow)}→${followLabel(now)}`, cause, detail);
        lastLoggedFollow = now;
    };
    const transitionFollow = (cause: string, detail: string | undefined, apply: () => void): void => {
        pendingFollowCause = { cause, detail };
        try {
            apply();
            logFollowIfChanged(untrack(viewState.stickToBottom));
        } finally {
            pendingFollowCause = null;
        }
    };
    console.info("[scroll-follow]", `pane=${paneTag()}`, `mount ${followLabel(lastLoggedFollow)}`);
    createEffect(() => logFollowIfChanged(viewState.stickToBottom()));
    return {
        transitionFollow,
        project(from, to, cause, detail) {
            if (intentOf(from) !== intentOf(to)) {
                const following = intentOf(to) === "FOLLOWING";
                transitionFollow(cause, detail, () =>
                    following ? viewState.engageStickToBottom() : viewState.disengageStickToBottom()
                );
            } else {
                // Hidden / shown: the intent (stickToBottom) is unchanged.
                line(`${followStateLabel(from)}→${followStateLabel(to)}`, cause, detail);
            }
        },
    };
}
