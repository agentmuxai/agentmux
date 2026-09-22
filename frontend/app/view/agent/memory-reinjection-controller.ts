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
 * reimplemented here — `dispatchTurnStart`/`dispatchTurnReset` are expected
 * to be the real `TurnStart`/`TurnReset` reducer commands, reused exactly
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
 *    `deferredFrameTimestamp`) when busy.
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
import type { MemoryEntryInput, ReinjectionReason } from "./memory-reinjection";
import type { MemoryReinjectionNode } from "./types";

/**
 * Deliberately content-free — see this module's doc comment, fix 2. Never
 * changes based on what's actually being reinjected; a consumer inspecting
 * `pendingContent` learns nothing about the real memory content from it.
 */
export const HIDDEN_TURN_PLACEHOLDER_CONTENT = "[agentmux: hidden memory reinjection]";

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
    sendRpc: (message: string) => Promise<void>;
    /** Starts real turn-state bookkeeping — expected to be the actual `TurnStart` dispatch, reused unmodified, called ONLY with `HIDDEN_TURN_PLACEHOLDER_CONTENT` and `hidden: true` (the controller enforces this — the caller's implementation should just forward both args verbatim). */
    dispatchTurnStart: (content: string, hidden: boolean) => void;
    /** Reverts turn-state bookkeeping on a send failure — expected to be the actual `TurnReset` dispatch. */
    dispatchTurnReset: () => void;
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
     */
    trigger: (frameTimestamp: string | null, reason: ReinjectionReason) => Promise<void>;
    /**
     * Call at every `session_end`, unconditionally — including for
     * perfectly ordinary, unrelated turns. Returns the completed
     * `MemoryReinjectionNode` (and clears hiding) ONLY if a hidden turn was
     * actually in flight; returns `null` and does nothing otherwise, so
     * calling this for a normal turn's session_end is always safe.
     */
    onSessionEnd: () => MemoryReinjectionNode | null;
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
    reason: ReinjectionReason;
}

export function createMemoryReinjectionController(opts: MemoryReinjectionControllerOpts): MemoryReinjectionController {
    let hiding = false;
    let pendingNode: MemoryReinjectionNode | null = null;
    let deferred: DeferredTrigger | undefined = undefined; // undefined = nothing deferred

    async function doTrigger(frameTimestamp: string | null, reason: ReinjectionReason): Promise<void> {
        let entries: MemoryEntryInput[];
        try {
            entries = await opts.fetchEntries();
        } catch {
            // Fetch failed before anything was sent or dispatched — nothing
            // to revert. Best-effort: skip this reinjection rather than
            // block or surface an error for a hidden, unsolicited turn.
            return;
        }

        if (!shouldReinject(entries)) return; // §3.1 suppression rule

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
            deferred = { frameTimestamp, reason };
            return;
        }

        const node = buildMemoryReinjectionNode(entries, {
            frameTimestamp,
            now: opts.now(),
            contextWindow: opts.contextWindow(),
        });
        const message = composeReinjectionMessage(entries, reason);

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
            await opts.sendRpc(message);
        } catch {
            // RPC outright failed — mirror useAgentCommands.ts's own
            // catch-path philosophy (never leave the pane stuck showing
            // "Working…" for a turn the backend never received).
            hiding = false;
            pendingNode = null;
            opts.dispatchTurnReset();
        }
    }

    async function trigger(frameTimestamp: string | null, reason: ReinjectionReason): Promise<void> {
        if (hiding) return; // re-entrancy guard — see doc comment
        if (deferred !== undefined) return; // already have one queued — first wins, arbitrary but simple

        // Early-exit OPTIMIZATION only — skips an unnecessary fetchEntries()
        // round trip when the pane is obviously already busy right now.
        // NOT load-bearing for correctness: doTrigger() re-checks
        // isPaneWorking() itself, immediately before dispatching, which is
        // what actually closes the race a stale check here could reopen.
        if (opts.isPaneWorking()) {
            deferred = { frameTimestamp, reason };
            return;
        }

        await doTrigger(frameTimestamp, reason);
    }

    function onSessionEnd(): MemoryReinjectionNode | null {
        if (!hiding) return null;
        const node = pendingNode;
        hiding = false;
        pendingNode = null;
        return node;
    }

    function maybeFireDeferred(): void {
        if (hiding) return; // shouldn't be reachable (trigger() already guards), but never stack regardless
        if (deferred === undefined) return;
        const { frameTimestamp, reason } = deferred;
        deferred = undefined;
        void doTrigger(frameTimestamp, reason);
    }

    return {
        isHiding: () => hiding,
        trigger,
        onSessionEnd,
        maybeFireDeferred,
    };
}
