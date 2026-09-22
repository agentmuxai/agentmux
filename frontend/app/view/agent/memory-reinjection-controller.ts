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
 * 1. **Busy-pane deferral.** The original version dispatched `TurnStart`
 *    unconditionally, on the (wrong) assumption that reusing `TurnStart`/
 *    `TurnEnd` alone was enough to "handle both the manual-compact and
 *    auto-compact timing cases with no new logic." It is not: for the
 *    common auto-compaction case, `compact_boundary` lands MID a real,
 *    still-streaming turn, and dispatching a fresh `TurnStart` on top of an
 *    active one regresses `turnPhase` from `Streaming` back to
 *    `Submitting` — exactly the flicker `agent-view.tsx`'s real
 *    `handleSendMessage` guards against via `if (!wasAlreadyWorking)`, a
 *    guard this module bypassed entirely by never calling
 *    `handleSendMessage` in the first place. Fixed: `trigger()` now checks
 *    `opts.isPaneWorking()` and DEFERS (via `deferredFrameTimestamp`)
 *    rather than firing immediately when the pane is busy; the deferred
 *    trigger fires once the in-flight turn's own `session_end` has been
 *    fully processed (`maybeFireDeferred()`, called by the
 *    `useAgentStream.ts` call site AFTER `finalizeTurn()` — turnPhase is
 *    then genuinely idle, so a fresh `TurnStart` is safe).
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
import type { MemoryEntryInput } from "./memory-reinjection";
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
     * Call once per real `CompactionBoundary`. If the pane is idle, fetches
     * current memory and sends it hidden immediately. If a real turn is
     * already in flight (the common auto-compaction case), DEFERS instead
     * — see doc comment fix 1 — and does nothing else until
     * `maybeFireDeferred()` is called. No-ops entirely (does not fetch,
     * does not defer) if a hidden turn is already in flight — re-entrancy
     * guard, never stacks two.
     */
    trigger: (frameTimestamp: string | null) => Promise<void>;
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

export function createMemoryReinjectionController(opts: MemoryReinjectionControllerOpts): MemoryReinjectionController {
    let hiding = false;
    let pendingNode: MemoryReinjectionNode | null = null;
    let deferredFrameTimestamp: string | null | undefined = undefined; // undefined = nothing deferred; null is a valid frameTimestamp value

    async function doTrigger(frameTimestamp: string | null): Promise<void> {
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

        const node = buildMemoryReinjectionNode(entries, {
            frameTimestamp,
            now: opts.now(),
            contextWindow: opts.contextWindow(),
        });
        const message = composeReinjectionMessage(entries);

        // Mirrors the real send path's own optimistic-TurnStart-before-RPC
        // ordering (agent-view.tsx's handleSendMessage) — turnPhase must
        // read "busy" before the RPC call, not after, so a real message
        // sent by the user in the gap between dispatch and RPC resolution
        // still correctly queues behind this turn instead of racing it.
        // See doc comment fix 1 for why this is now only ever reached when
        // the pane is confirmed idle (checked in trigger()/maybeFireDeferred()).
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

    async function trigger(frameTimestamp: string | null): Promise<void> {
        if (hiding) return; // re-entrancy guard — see doc comment
        if (deferredFrameTimestamp !== undefined) return; // already have one queued — first wins, arbitrary but simple

        if (opts.isPaneWorking()) {
            // Doc comment fix 1: a real turn is still in flight. Wait for
            // ITS session_end (maybeFireDeferred()) rather than dispatching
            // TurnStart on top of it.
            deferredFrameTimestamp = frameTimestamp;
            return;
        }

        await doTrigger(frameTimestamp);
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
        if (deferredFrameTimestamp === undefined) return;
        const ts = deferredFrameTimestamp;
        deferredFrameTimestamp = undefined;
        void doTrigger(ts);
    }

    return {
        isHiding: () => hiding,
        trigger,
        onSessionEnd,
        maybeFireDeferred,
    };
}
