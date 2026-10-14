// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Orchestrates one hidden memory-reinjection turn — trigger through
 * completion. See SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md
 * §3.3. Deliberately has ZERO Solid/RPC dependencies of its own — every
 * side effect (fetching entries, sending the message, dispatching turn
 * state) is injected, so the actual orchestration logic (when to hide, when
 * to stop hiding, what to do on failure) is testable without a live app or
 * a mocked reactive framework. The live wiring (real RPC clients, real
 * `model.dispatchPane`) lives at the `useAgentStream.ts` call site, kept as
 * thin as possible on top of this.
 *
 * State-machine correctness (turnPhase/busy-vs-idle) is deliberately NOT
 * reimplemented here — `dispatchTurnStart`/`dispatchTurnStartFailed` are expected
 * to be the real `TurnStart`/`TurnStartFailed` reducer commands, reused exactly
 * as-is. Only the RENDERING is different, which is this module's actual
 * job — that and the DELIVERY path, which bypasses the normal pending-zone
 * (`sendRpc` is expected to be the raw `RpcApi.AgentInputCommand` call, not
 * the full `sendMessage` pending-zone/promotion path) so no transient
 * "sending…" row or promoted `UserMessageNode` is ever created for this
 * turn — see the module's sibling `hiding-stream-flush-queue.ts` for the
 * other half (suppressing the resulting ASSISTANT response).
 *
 * **Two fixes below landed from reagentx P0 review on PR #3502 — recorded
 * here because both correct a claim this module's own original doc comment
 * used to make:**
 *
 * 1. **Busy-pane deferral — TWO rounds.** Round 1: the original version
 *    dispatched `TurnStart` unconditionally, on the (wrong) assumption
 *    that reusing `TurnStart`/`TurnEnd` alone was enough to "handle both
 *    the manual-compact and auto-compact timing cases with no new logic."
 *    It is not: for the common auto-compaction case, `compact_boundary`
 *    lands MID a real, still-streaming turn, and dispatching a fresh
 *    `TurnStart` on top of an active one regresses `turnPhase` from
 *    `Streaming` back to `Submitting` — exactly the flicker
 *    `agent-view.tsx`'s real `handleSendMessage` guards against via
 *    `if (!wasAlreadyWorking)`, a guard this module bypassed entirely by
 *    never calling `handleSendMessage` in the first place. First fix:
 *    check `opts.isPaneWorking()` once, before fetching, and DEFER (via
 *    `deferred`) when busy.
 *
 *    **Round 2 (reagentx P1, same PR, next re-review): that single
 *    check wasn't enough — it left a real, narrower race.**
 *    `fetchEntries()` is a genuine async RPC round trip (widened further
 *    by `memory-reinjection-fetch.ts` reading personal-memory files
 *    sequentially, not in parallel); a real turn can start DURING that
 *    await (a queued message auto-promoted, or a fresh user send into an
 *    apparently-idle pane), and the original round-1 fix still dispatched
 *    `TurnStart` unconditionally once the fetch resolved — reintroducing
 *    the exact same corruption through a narrower window instead of
 *    closing it. Fixed by moving the authoritative check to `doTrigger()`
 *    itself, immediately before `dispatchTurnStart()` (after the fetch,
 *    with no further `await` in between) — the ONE place close enough to
 *    the actual dispatch to matter, rather than checked once at a call
 *    site that can go stale. `trigger()`'s own pre-fetch check is now
 *    explicitly just an optimization (skips a pointless fetch when
 *    obviously busy already), not the correctness guarantee.
 *    `maybeFireDeferred()` needs no check of its own for the same reason —
 *    it calls the same `doTrigger()`, which re-verifies regardless of
 *    caller. A deferred trigger fires once the in-flight turn's own
 *    `session_end` has been fully processed (`maybeFireDeferred()`,
 *    called by the `useAgentStream.ts` call site AFTER `finalizeTurn()`),
 *    but if something else raced it busy again in that instant, it simply
 *    re-defers rather than firing incorrectly.
 * 2. **No real content in `TurnStart.content`.** The original version
 *    passed the full composed message (real Global + Personal memory
 *    bodies) as `TurnStart.content`/`pendingContent` — visible to ANY
 *    consumer that watches `turnPhase`, not just this hook's own
 *    suppression mechanism. `useAgentActivitySummary.ts` does exactly
 *    that: it forwards `pendingContent` to an ambient LLM call whose
 *    result becomes the human-visible pane title, regardless of what
 *    dispatched the turn. Fixed at the source, not just patched at that
 *    one call site: `trigger()` now dispatches a fixed, content-free
 *    placeholder string and the `hidden: true` flag (propagated onto
 *    `TurnPhase.Submitting.hidden` — see that field's doc comment in
 *    `agent-pane-state/types.ts`), so a consumer that forgets to check
 *    `hidden` still never sees real content in the first place — defense
 *    in depth, not reliant on every current and future consumer
 *    remembering to opt out.
 */

import { buildMemoryReinjectionNode, composeReinjectionMessage, shouldReinject } from "./memory-reinjection";
import type { CatchUpReason, MemoryEntryInput, ReinjectionReason } from "./memory-reinjection";
import type { ContextDeliveryNode, MemoryReinjectionNode } from "./types";

/**
 * A re-delivery srv composed (`memorydelivery:compose`, CD2b): the hidden
 * message, the id to send it with, and the card srv writes when it arrives.
 */
export interface ComposedDelivery {
    /** Absent for a catch-up that carries no memory: there is no card to write. */
    deliveryId?: string;
    text: string;
    node?: ContextDeliveryNode;
}

/**
 * Deliberately content-free — see this module's doc comment, fix 2. Never
 * changes based on what's actually being reinjected; a consumer inspecting
 * `pendingContent` learns nothing about the real memory content from it.
 */
export const HIDDEN_TURN_PLACEHOLDER_CONTENT = "[agentmux: hidden memory reinjection]";

/**
 * srv's answer to a claim. `foreign`: the event was another block's (a
 * compaction this block's CLI did not write), not the hook's, so a catch-up
 * turn has no business here either.
 */
export interface ClaimAnswer {
    deliver: boolean;
    foreign?: boolean;
}

export interface MemoryReinjectionControllerOpts {
    /** The pane's current context window (tokens) — threaded to buildMemoryReinjectionNode's size-band computation, §3.4.2. */
    contextWindow: () => number;
    /** Wall-clock fallback for the node's `at` field when frameTimestamp can't be parsed. */
    now: () => number;
    /** True if a real turn is currently in flight (Submitting/Streaming/Interrupting) — expected to be `workingFromPhase(paneSnapshot(blockId)?.turnPhase)`, read fresh at call time. See doc comment fix 1. */
    isPaneWorking: () => boolean;
    /** Fetches Global + Personal memory entries with full bodies. May reject (e.g. RPC failure) — handled by trigger(). */
    fetchEntries: () => Promise<MemoryEntryInput[]>;
    /** Delivers the composed message to the live CLI process. Expected to be the raw send RPC, NOT the pending-zone/promotion path — see module doc comment. May reject. */
    sendRpc: (message: string, deliveryId?: string) => Promise<void>;
    /**
     * srv composes the message and keeps its item list, so the card shows
     * every item and replays (SPEC_CONTEXT_DELIVERY_2026_09_30.md §3.4 step
     * 2). Resolves `null` when there's no memory. Absent, or a rejection (an
     * older srv), falls back to `fetchEntries` and composing here.
     */
    compose?: (reason: ReinjectionReason) => Promise<ComposedDelivery | null>;
    /**
     * srv's catch-up message: the memory re-delivery when `memory` (else the
     * hook already delivered it, or the phase has none), then the block that
     * has the agent say where it is picking up
     * (`memorydelivery:compose` with `catch_up`). Absent, or a rejection, means
     * no catch-up turn.
     */
    composeCatchUp?: (reason: CatchUpReason, memory: boolean) => Promise<ComposedDelivery | null>;
    /** Starts real turn-state bookkeeping — expected to be the actual `TurnStart` dispatch, reused unmodified, called ONLY with `HIDDEN_TURN_PLACEHOLDER_CONTENT` and `hidden: true` (the controller enforces this — the caller's implementation should just forward both args verbatim). */
    dispatchTurnStart: (content: string, hidden: boolean) => void;
    /** Reverts turn-state bookkeeping on a send failure — expected to be the actual `TurnStartFailed` dispatch
     *  (phase only: a failed send must not wipe the pane's context reading or session totals, #2318). */
    dispatchTurnStartFailed: () => void;
    /**
     * Asks the sidecar whether this fallback should deliver (`true`) or stand
     * down because Claude Code's `SessionStart` hook already delivered the
     * same memory for this event (`false`) —
     * docs/specs/SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2. Called
     * right before anything is hidden or dispatched, so standing down leaves
     * the pane untouched. The sidecar decides atomically: whichever of the hook
     * and this fallback claims the event first delivers. Absent, or a
     * rejection, means deliver: a duplicate beats no memory.
     *
     * A compaction is claimed when its boundary arrives, with the boundary's
     * `uuid` (srv answers `false` unless this block's own CLI wrote it); a
     * fresh session when it is about to fire, with `eventAtMs`, its
     * session-outcome frame's time. That fire is often only after the
     * session's first turn, long after the hook delivered; the time lets srv
     * still match the hook's delivery to this session.
     */
    claimFallback?: (
        reason: ReinjectionReason,
        boundaryUuid?: string,
        eventAtMs?: number,
    ) => Promise<boolean | ClaimAnswer>;
}

export interface MemoryReinjectionController {
    /** True for the entire duration of a hidden turn — from `trigger()`'s successful send until `onSessionEnd()` observes its completion. Read this to gate the hiding queue wrapper. */
    isHiding: () => boolean;
    /**
     * Call once per real `CompactionBoundary` (`reason: "compaction"`) or
     * per fresh-session `agentmux_session_outcome` (`reason: "fresh_session"`
     * — a persistent identity whose prior session could not be resumed, so
     * the model has none of its prior context at all; see
     * SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md §3.3's
     * "fresh session" addendum, §3.3a). If the pane is idle, fetches current
     * memory and sends it hidden immediately. If a real turn is already in
     * flight (the common case for BOTH triggers — auto-compaction lands
     * mid-turn, and a fresh-session outcome is typically discovered while
     * processing the very turn the user just sent), DEFERS instead — see
     * doc comment fix 1 — and does nothing else until `maybeFireDeferred()`
     * is called. No-ops entirely (does not fetch, does not defer) if a
     * hidden turn is already in flight — re-entrancy guard, never stacks
     * two.
     *
     * `boundaryUuid`: the compaction boundary's own `uuid`. A boundary seen
     * again (a re-read, a reconnect) is ignored, and it keys the node id.
     */
    trigger: (frameTimestamp: string | null, reason: ReinjectionReason, boundaryUuid?: string | null) => Promise<void>;
    /**
     * The turn after a resume phase (SPEC_RESUME_PHASE_CATCH_UP_TURN_2026_10_13.md
     * §3.3): AgentMux hands the agent what it needs (memory the hook didn't
     * deliver, then an instruction to say where it is picking up), the pane
     * shows a card for the input, and the agent's reply is shown, unlike a
     * reinjection's. Fires only on a quiet pane: with the user already
     * talking to the agent there is nothing to catch up on, so a busy pane
     * makes this a no-op. Waits out a turn already in flight when
     * `deferIfBusy` (a manual compaction, found mid-way through its own turn).
     */
    catchUp: (
        frameTimestamp: string | null,
        reason: CatchUpReason,
        opts?: { boundaryUuid?: string | null; deferIfBusy?: boolean },
    ) => Promise<void>;
    /**
     * Call at every `session_end`, unconditionally — including for
     * perfectly ordinary, unrelated turns. Returns the completed
     * `MemoryReinjectionNode` (and clears hiding) ONLY if a hidden turn was
     * actually in flight; returns `null` and does nothing otherwise, so
     * calling this for a normal turn's session_end is always safe.
     */
    onSessionEnd: () => MemoryReinjectionNode | ContextDeliveryNode | null;
    /**
     * Call AFTER `finalizeTurn()` has run for ANY session_end (real or
     * hidden) — i.e. once `turnPhase` has genuinely settled to `Done` for
     * whatever turn just ended. If a reinjection was deferred because the
     * pane was busy when `trigger()` was called, this is where it actually
     * fires — turnPhase is now legitimately idle, so a fresh `TurnStart` is
     * safe. No-op if nothing is deferred.
     */
    maybeFireDeferred: () => void;
}

interface DeferredTrigger {
    frameTimestamp: string | null;
    reason: CatchUpReason;
    boundaryUuid: string | null;
    /** srv already answered "deliver" for this event: don't ask again. */
    claimed: boolean;
    /** A catch-up turn rather than a bare reinjection. */
    catchUp: boolean;
    /** A catch-up only: carry the memory too (false once the hook delivered it). */
    memory: boolean;
}

/** Boundary uuids a controller remembers, to ignore one seen twice. */
const SEEN_BOUNDARIES_MAX = 64;

/** A fresh-session outcome older than this is not a session starting now. */
const FRESH_SESSION_MAX_AGE_MS = 60_000;

export function createMemoryReinjectionController(opts: MemoryReinjectionControllerOpts): MemoryReinjectionController {
    let hiding = false;
    // A catch-up turn is in flight, from its send until its own session_end.
    // Unlike `hiding` it hides nothing.
    let catchingUp = false;
    let pendingNode: MemoryReinjectionNode | ContextDeliveryNode | null = null;
    let deferred: DeferredTrigger | undefined = undefined; // undefined = nothing deferred
    const seenBoundaries = new Set<string>();

    function firstSighting(uuid: string): boolean {
        if (seenBoundaries.has(uuid)) return false;
        seenBoundaries.add(uuid);
        if (seenBoundaries.size > SEEN_BOUNDARIES_MAX) {
            seenBoundaries.delete(seenBoundaries.values().next().value as string);
        }
        return true;
    }

    async function claimAnswer(t: DeferredTrigger): Promise<ClaimAnswer> {
        // Only a fresh session's frame is srv's own, so only it dates the claim.
        const at = t.reason === "fresh_session" && t.frameTimestamp ? Date.parse(t.frameTimestamp) : NaN;
        const eventAtMs = Number.isNaN(at) ? undefined : at;
        // A failed claim means deliver: a duplicate beats no memory.
        const answer = await opts
            .claimFallback!(t.reason as ReinjectionReason, t.boundaryUuid ?? undefined, eventAtMs)
            .catch(() => true);
        return typeof answer === "boolean" ? { deliver: answer } : answer;
    }

    async function claim(t: DeferredTrigger): Promise<boolean> {
        return (await claimAnswer(t)).deliver;
    }

    /**
     * What to send and what to show: srv's composition when it can (CD2b),
     * else this side's own from `fetchEntries`. `null` = nothing to deliver.
     */
    async function prepare(
        frameTimestamp: string | null,
        reason: ReinjectionReason,
        boundaryUuid: string | null,
    ): Promise<{ message: string; deliveryId?: string; node: MemoryReinjectionNode | ContextDeliveryNode } | null> {
        if (opts.compose) {
            const composed = await opts.compose(reason).then(
                (c) => ({ ok: true as const, c }),
                () => ({ ok: false as const }),
            );
            if (composed.ok) {
                return composed.c ? { message: composed.c.text, deliveryId: composed.c.deliveryId, node: composed.c.node } : null;
            }
        }
        let entries: MemoryEntryInput[];
        try {
            entries = await opts.fetchEntries();
        } catch {
            // Fetch failed before anything was sent or dispatched — nothing
            // to revert. Best-effort: skip this reinjection rather than
            // block or surface an error for a hidden, unsolicited turn.
            return null;
        }
        if (!shouldReinject(entries)) return null; // §3.1 suppression rule
        return {
            message: composeReinjectionMessage(entries, reason),
            node: buildMemoryReinjectionNode(entries, {
                frameTimestamp,
                boundaryUuid,
                now: opts.now(),
                contextWindow: opts.contextWindow(),
            }),
        };
    }

    /**
     * A catch-up turn: claim the memory event first (the hook may have
     * delivered it), ask srv for the message with or without memory, then send
     * it with nothing hidden. Reads `isPaneWorking()` after every await, as
     * `doTrigger` does, and re-defers if a turn started.
     */
    async function doCatchUp(t: DeferredTrigger): Promise<void> {
        if (!opts.composeCatchUp) return;
        if (opts.isPaneWorking()) {
            deferred = t;
            return;
        }
        if (!t.claimed && opts.claimFallback) {
            const answer = await claimAnswer(t);
            // Another block's compaction: this block's agent was not compacted.
            if (answer.foreign) return;
            t.memory = answer.deliver;
            t.claimed = true;
            if (opts.isPaneWorking()) {
                deferred = t;
                return;
            }
        }
        const composed = await opts.composeCatchUp(t.reason as CatchUpReason, t.memory).catch(() => null);
        if (!composed) return;
        if (opts.isPaneWorking() || catchingUp || hiding) {
            // A turn started while srv composed. The message stays unsent; it is
            // composed again, from fresh state, when the pane next goes quiet.
            deferred = t;
            return;
        }
        pendingNode = composed.node ?? null;
        catchingUp = true;
        opts.dispatchTurnStart(HIDDEN_TURN_PLACEHOLDER_CONTENT, true);
        try {
            await opts.sendRpc(composed.text, composed.deliveryId);
        } catch {
            catchingUp = false;
            pendingNode = null;
            opts.dispatchTurnStartFailed();
        }
    }

    async function doTrigger(t: DeferredTrigger): Promise<void> {
        if (t.catchUp) return doCatchUp(t);
        const prepared = await prepare(t.frameTimestamp, t.reason as ReinjectionReason, t.boundaryUuid);
        if (!prepared) return;

        // Re-check busy-ness HERE, after the async fetchEntries() await —
        // reagentx P1, second review round: checking isPaneWorking() only
        // once at the top of trigger() (before this await) left a real
        // race. fetchEntries() is a genuine RPC round trip
        // (memory-reinjection-fetch.ts reads personal-memory files
        // sequentially, not in parallel, widening the window further) — a
        // real turn can start DURING it (a queued message auto-promoted,
        // or the user sending into an apparently-idle pane), and firing
        // TurnStart unconditionally after the fetch resolves reintroduces
        // the exact turnPhase-corruption bug "fix 1" (doc comment above)
        // was built to eliminate, just through a narrower window. Also why
        // maybeFireDeferred() no longer needs its own isPaneWorking()
        // check — it calls this same function, and this is the ONE place
        // busy-ness is verified immediately before the dispatch that
        // matters, not scattered across multiple call sites that could
        // drift out of sync.
        if (opts.isPaneWorking()) {
            deferred = t;
            return;
        }

        // Only now — about to fire, pane idle — ask whether the SessionStart
        // hook already delivered this event. Standing down here touches
        // nothing: no hiding, no pending node, no TurnStart (Codex on #3926:
        // a skip must never strand the pane busy). A claim is only made when
        // the fallback is really about to fire, never while deferred.
        if (opts.claimFallback && !t.claimed) {
            if (!(await claim(t))) return;
            // The claim was a round trip too: a real turn may have started.
            if (opts.isPaneWorking()) {
                deferred = { ...t, claimed: true };
                return;
            }
        }

        const { message, deliveryId, node } = prepared;

        // Mirrors the real send path's own optimistic-TurnStart-before-RPC
        // ordering (agent-view.tsx's handleSendMessage) — turnPhase must
        // read "busy" before the RPC call, not after, so a real message
        // sent by the user in the gap between dispatch and RPC resolution
        // still correctly queues behind this turn instead of racing it.
        // Synchronous from the isPaneWorking() check immediately above
        // (no further await in between), so this is as close to atomic as
        // this async architecture allows.
        hiding = true;
        pendingNode = node;
        // Placeholder content, not `message` — see doc comment fix 2 and
        // HIDDEN_TURN_PLACEHOLDER_CONTENT. The REAL content only ever
        // travels through opts.sendRpc below, never through reducer state.
        opts.dispatchTurnStart(HIDDEN_TURN_PLACEHOLDER_CONTENT, true);

        try {
            await opts.sendRpc(message, deliveryId);
        } catch {
            // RPC outright failed — mirror useAgentCommands.ts's own
            // catch-path philosophy (never leave the pane stuck showing
            // "Working…" for a turn the backend never received).
            hiding = false;
            pendingNode = null;
            opts.dispatchTurnStartFailed();
        }
    }

    async function trigger(
        frameTimestamp: string | null,
        reason: ReinjectionReason,
        boundaryUuid: string | null = null,
    ): Promise<void> {
        if (boundaryUuid && !firstSighting(boundaryUuid)) return; // once per boundary
        if (hiding || catchingUp) return; // re-entrancy guard — see doc comment
        if (deferred !== undefined) return; // already have one queued — first wins, arbitrary but simple

        const t: DeferredTrigger = { frameTimestamp, reason, boundaryUuid, claimed: false, catchUp: false, memory: true };
        // A compaction is claimed now, not when it fires: the SessionStart
        // hook delivered moments ago, inside srv's claim window. Claimed at
        // the end of a long turn, that window has passed and both would send.
        if (reason === "compaction" && opts.claimFallback) {
            if (!(await claim(t))) return;
            t.claimed = true;
            if (hiding || deferred !== undefined) return;
        }

        // Early-exit OPTIMIZATION only — skips an unnecessary fetchEntries()
        // round trip when the pane is obviously already busy right now.
        // NOT load-bearing for correctness: doTrigger() re-checks
        // isPaneWorking() itself, immediately before dispatching, which is
        // what actually closes the race a stale check here could reopen.
        if (opts.isPaneWorking()) {
            deferred = t;
            return;
        }

        await doTrigger(t);
    }

    async function catchUp(
        frameTimestamp: string | null,
        reason: CatchUpReason,
        catchUpOpts: { boundaryUuid?: string | null; deferIfBusy?: boolean } = {},
    ): Promise<void> {
        const boundaryUuid = catchUpOpts.boundaryUuid ?? null;
        if (boundaryUuid && !firstSighting(boundaryUuid)) return; // once per boundary
        // One per fresh session: the same outcome seen twice (a re-read, a second
        // writer) must not start a second turn.
        if (reason === "fresh_session" && frameTimestamp && !firstSighting(`fresh:${frameTimestamp}`)) return;
        if (hiding || catchingUp || deferred !== undefined) return;
        // An outcome that is not recent (a re-read after a reconnect, a line
        // another writer appended) is not a session starting now.
        if (reason === "fresh_session" && frameTimestamp) {
            const at = Date.parse(frameTimestamp);
            if (!Number.isNaN(at) && opts.now() - at > FRESH_SESSION_MAX_AGE_MS) return;
        }
        // A user already talking to the agent has nothing to be caught up on.
        if (opts.isPaneWorking() && !catchUpOpts.deferIfBusy) return;
        const t: DeferredTrigger = {
            frameTimestamp,
            reason,
            boundaryUuid,
            // A resumed conversation re-delivers no memory, so there is no event to claim.
            claimed: reason === "resumed",
            catchUp: true,
            memory: reason !== "resumed",
        };
        // A compaction is claimed now, not when the turn fires: the hook
        // delivered moments ago, inside srv's claim window, and a manual
        // compaction's own turn can outlast it.
        if (reason === "compaction" && opts.claimFallback) {
            const answer = await claimAnswer(t);
            if (answer.foreign) return;
            t.memory = answer.deliver;
            t.claimed = true;
            if (hiding || catchingUp || deferred !== undefined) return;
        }
        await doCatchUp(t);
    }

    function onSessionEnd(): MemoryReinjectionNode | ContextDeliveryNode | null {
        if (catchingUp) {
            // The catch-up turn is over. Its card was written to the pane when
            // srv accepted the message; this copy is the backstop, deduped by id.
            const node = pendingNode;
            catchingUp = false;
            pendingNode = null;
            return node;
        }
        if (!hiding) return null;
        const node = pendingNode;
        hiding = false;
        pendingNode = null;
        return node;
    }

    function maybeFireDeferred(): void {
        if (hiding) return; // shouldn't be reachable (trigger() already guards), but never stack regardless
        if (deferred === undefined) return;
        const t = deferred;
        deferred = undefined;
        void doTrigger(t);
    }

    return {
        isHiding: () => hiding,
        trigger,
        catchUp,
        onSessionEnd,
        maybeFireDeferred,
    };
}
