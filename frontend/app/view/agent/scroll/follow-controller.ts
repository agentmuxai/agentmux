// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The follow state machine for a transcript scroller: one owner for "are we
 * following". Pure: no DOM, no Solid. The DOM binding that feeds it is
 * follow-scroll.ts. SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md
 * §5 (states §5.2, transitions §5.3, thresholds §5.5).
 *
 * Invariants it enforces:
 *   I1. Only a user gesture leaves FOLLOWING: UserEscape, JumpToNode, or a
 *       user scroll that moved the transcript up. No geometry reading, clamp,
 *       anchoring adjustment or pagination step can.
 *   I2. LoadOlder is emitted only for a user scroll, only while DETACHED, and
 *       never changes the follow state.
 *   I4. Every state change emits exactly one Transition effect with its cause.
 */

/** How close to the true bottom a user scroll must end to follow again (§5.5). */
export const REATTACH_PX = 24;
/** A user scroll this close to the top pages in older history (§5.5). */
export const NEAR_TOP_PX = 50;
/** Moves smaller than this are fractional-zoom noise, not a scroll. */
const EPSILON_PX = 1;

export type FollowMode = "FOLLOWING" | "DETACHED";

export type FollowState =
    | { readonly kind: "FOLLOWING" }
    | { readonly kind: "DETACHED" }
    /** Hidden or zero-size: input is ignored and `prev` comes back once it has size (§5.2). */
    | { readonly kind: "SUSPENDED"; readonly prev: FollowMode };

export const FOLLOWING: FollowState = { kind: "FOLLOWING" };
export const DETACHED: FollowState = { kind: "DETACHED" };
export const suspended = (prev: FollowMode): FollowState => ({ kind: "SUSPENDED", prev });

export interface ScrollGeometry {
    scrollTop: number;
    scrollHeight: number;
    clientHeight: number;
}

/** What a gesture that escapes at once was (§5.4). */
export type EscapeCause = "wheel-up" | "key" | "selection";

export type FollowEvent =
    /** A ResizeObserver saw the content (or the scroller's viewport) change size. */
    | { type: "ContentResized"; geo: ScrollGeometry }
    | { type: "ViewportResized"; geo: ScrollGeometry }
    /**
     * A gesture that means "the user is leaving the bottom", handled before
     * any scroll it causes (a same-frame pin can't override it). `geo` is the
     * last observed geometry: nothing to escape from when it can't scroll up.
     */
    | { type: "UserEscape"; cause: EscapeCause; geo: ScrollGeometry }
    /**
     * A scroll batch. `user` is true inside a user-intent window (§5.4);
     * `prev` is the geometry last observed before it.
     */
    | {
          type: "Scroll";
          user: boolean;
          prev: ScrollGeometry;
          geo: ScrollGeometry;
          /** The gesture was a held scrollbar (telemetry only). */
          scrollbar?: boolean;
          hasOlderHistory: boolean;
          loadingOlder: boolean;
      }
    /** Typing, send, a queued turn starting, the "jump to latest" control. */
    | { type: "JumpToBottom"; reason: string }
    /** Search or a deep link. */
    | { type: "JumpToNode"; nodeId: string }
    | { type: "BecameZeroSize" }
    | { type: "BecameSized" }
    /** `/clear`, or a new session in the same pane. */
    | { type: "Reset" };

export type FollowEffect =
    /** Write scrollTop to the true bottom. */
    | { type: "Pin"; cause: string }
    /** Page in older history (capture the anchor, fetch, restore it). */
    | { type: "LoadOlder" }
    /** I4: one telemetry line. */
    | { type: "Transition"; from: FollowState; to: FollowState; cause: string; detail?: string };

export interface FollowResult {
    state: FollowState;
    effects: FollowEffect[];
}

/**
 * `agent:followcontroller`: an explicit true/false wins; unset means on in dev
 * builds and off in release, until the live soak passes (§8 Phase 2/4).
 */
export function resolveFollowController(setting: unknown, isDevBuild: () => boolean): boolean {
    if (typeof setting === "boolean") return setting;
    try {
        return isDevBuild();
    } catch {
        return false; // no host API (tests, a detached renderer): the release default
    }
}

/** The follow intent: what a SUSPENDED pane goes back to. */
export function intentOf(state: FollowState): FollowMode {
    return state.kind === "SUSPENDED" ? state.prev : state.kind;
}

/** Label for telemetry and `data-follow-state`. */
export function followStateLabel(state: FollowState): "following" | "detached" | "suspended" {
    return state.kind === "FOLLOWING" ? "following" : state.kind === "DETACHED" ? "detached" : "suspended";
}

const gapOf = (g: ScrollGeometry): number => g.scrollHeight - g.clientHeight - g.scrollTop;
const gapDetail = (g: ScrollGeometry): string => `gap=${Math.round(gapOf(g))}px`;

function same(a: FollowState, b: FollowState): boolean {
    return a.kind === b.kind && (a.kind !== "SUSPENDED" || a.prev === (b as { prev: FollowMode }).prev);
}

function go(
    state: FollowState,
    to: FollowState,
    cause: string,
    detail?: string,
    extra: FollowEffect[] = []
): FollowResult {
    if (same(state, to)) return { state, effects: extra };
    return { state: to, effects: [{ type: "Transition", from: state, to, cause, detail }, ...extra] };
}

const stay = (state: FollowState): FollowResult => ({ state, effects: [] });

/** Set the follow intent, keeping a SUSPENDED pane suspended. */
function withIntent(state: FollowState, mode: FollowMode): FollowState {
    if (state.kind === "SUSPENDED") return suspended(mode);
    return mode === "FOLLOWING" ? FOLLOWING : DETACHED;
}

export function followReducer(state: FollowState, event: FollowEvent): FollowResult {
    switch (event.type) {
        case "ContentResized":
        case "ViewportResized": {
            const cause = event.type === "ContentResized" ? "content" : "viewport";
            if (state.kind === "FOLLOWING") return { state, effects: [{ type: "Pin", cause }] };
            // Nothing overflows, so there is no older content being read and no
            // position to keep: follow again, or new content would land below a
            // pane nobody can scroll.
            if (
                state.kind === "DETACHED" &&
                event.geo.clientHeight > 0 &&
                event.geo.scrollHeight <= event.geo.clientHeight + EPSILON_PX
            ) {
                return go(state, FOLLOWING, "no-overflow", undefined, [{ type: "Pin", cause: "no-overflow" }]);
            }
            return stay(state);
        }

        case "UserEscape": {
            if (state.kind !== "FOLLOWING") return stay(state);
            // At the top, or nothing overflows: the gesture scrolls nothing.
            if (event.geo.scrollTop <= EPSILON_PX / 2) return stay(state);
            return go(state, DETACHED, `user-escape:${event.cause}`, gapDetail(event.geo));
        }

        case "Scroll": {
            if (state.kind === "SUSPENDED" || !event.user) return stay(state);
            const { prev, geo } = event;
            const movedUp = geo.scrollTop < prev.scrollTop - EPSILON_PX;
            let result: FollowResult = stay(state);
            if (state.kind === "FOLLOWING") {
                // Up with the same scrollHeight is the user. Up with a shorter
                // one is a clamp after a shrink, which the user didn't do.
                if (movedUp && Math.abs(geo.scrollHeight - prev.scrollHeight) <= EPSILON_PX) {
                    result = go(
                        state,
                        DETACHED,
                        event.scrollbar ? "user-scroll:scrollbar" : "user-scroll",
                        gapDetail(geo)
                    );
                } else {
                    return result; // down never detaches (§5.4)
                }
            } else if (!movedUp && gapOf(geo) <= REATTACH_PX) {
                return go(state, FOLLOWING, "user-scroll-to-bottom", gapDetail(geo), [
                    { type: "Pin", cause: "user-scroll-to-bottom" },
                ]);
            }
            // DETACHED now (I2): page in older history from a user scroll near the top.
            if (geo.scrollTop < NEAR_TOP_PX && event.hasOlderHistory && !event.loadingOlder) {
                result = { state: result.state, effects: [...result.effects, { type: "LoadOlder" }] };
            }
            return result;
        }

        case "JumpToBottom": {
            const to = withIntent(state, "FOLLOWING");
            // A hidden pane can't be pinned; BecameSized pins it.
            const pin: FollowEffect[] =
                to.kind === "FOLLOWING" ? [{ type: "Pin", cause: `jump-to-bottom:${event.reason}` }] : [];
            return go(state, to, `jump-to-bottom:${event.reason}`, undefined, pin);
        }

        case "JumpToNode":
            return go(state, withIntent(state, "DETACHED"), "jump-to-node", `node=${event.nodeId.slice(0, 8)}`);

        case "BecameZeroSize":
            if (state.kind === "SUSPENDED") return stay(state);
            return go(state, suspended(state.kind), "zero-size");

        case "BecameSized": {
            if (state.kind !== "SUSPENDED") return stay(state);
            const to = state.prev === "FOLLOWING" ? FOLLOWING : DETACHED;
            // DETACHED: the reader's scrollTop is left alone.
            return go(state, to, "sized", undefined, to.kind === "FOLLOWING" ? [{ type: "Pin", cause: "sized" }] : []);
        }

        case "Reset":
            return go(state, withIntent(state, "FOLLOWING"), "reset");
    }
}
