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
 * as-is. A hidden reinjection is a completely real turn from the state
 * machine's perspective (§2 of the spec: this is what lets the existing
 * queued-while-busy machinery handle both the manual-compact and
 * auto-compact timing cases with no new logic). Only the RENDERING is
 * different, which is this module's actual job — that and the DELIVERY
 * path, which bypasses the normal pending-zone (`sendRpc` is expected to be
 * the raw `RpcApi.AgentInputCommand` call, not the full `sendMessage`
 * pending-zone/promotion path) so no transient "sending…" row or promoted
 * `UserMessageNode` is ever created for this turn — see the module's
 * sibling `hiding-stream-flush-queue.ts` for the other half (suppressing
 * the resulting ASSISTANT response).
 */

import { buildMemoryReinjectionNode, composeReinjectionMessage, shouldReinject } from "./memory-reinjection";
import type { MemoryEntryInput } from "./memory-reinjection";
import type { MemoryReinjectionNode } from "./types";

export interface MemoryReinjectionControllerOpts {
    /** The pane's current context window (tokens) — threaded to buildMemoryReinjectionNode's size-band computation, §3.4.2. */
    contextWindow: () => number;
    /** Wall-clock fallback for the node's `at` field when frameTimestamp can't be parsed. */
    now: () => number;
    /** Fetches Global + Personal memory entries with full bodies. May reject (e.g. RPC failure) — handled by trigger(). */
    fetchEntries: () => Promise<MemoryEntryInput[]>;
    /** Delivers the composed message to the live CLI process. Expected to be the raw send RPC, NOT the pending-zone/promotion path — see module doc comment. May reject. */
    sendRpc: (message: string) => Promise<void>;
    /** Starts real turn-state bookkeeping — expected to be the actual `TurnStart` dispatch, reused unmodified. */
    dispatchTurnStart: (content: string) => void;
    /** Reverts turn-state bookkeeping on a send failure — expected to be the actual `TurnReset` dispatch. */
    dispatchTurnReset: () => void;
}

export interface MemoryReinjectionController {
    /** True for the entire duration of a hidden turn — from `trigger()`'s successful send until `onSessionEnd()` observes its completion. Read this to gate the hiding queue wrapper. */
    isHiding: () => boolean;
    /**
     * Call once per real `CompactionBoundary`. Fetches current memory,
     * sends it hidden if there's anything to send, and enters hiding.
     * No-ops (does not fetch, does not send) if a hidden turn is already
     * in flight — re-entrancy guard, never stacks two.
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
}

export function createMemoryReinjectionController(opts: MemoryReinjectionControllerOpts): MemoryReinjectionController {
    let hiding = false;
    let pendingNode: MemoryReinjectionNode | null = null;

    async function trigger(frameTimestamp: string | null): Promise<void> {
        if (hiding) return; // re-entrancy guard — see doc comment

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
        hiding = true;
        pendingNode = node;
        opts.dispatchTurnStart(message);

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

    function onSessionEnd(): MemoryReinjectionNode | null {
        if (!hiding) return null;
        const node = pendingNode;
        hiding = false;
        pendingNode = null;
        return node;
    }

    return {
        isHiding: () => hiding,
        trigger,
        onSessionEnd,
    };
}
