// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useCompactionStream — the single per-block WPS subscription for
 * `compaction_started` events, published by the `PreCompact` hook
 * (`agentmux-bashwrap precompact --trigger=manual|auto`) the instant
 * Claude Code begins compacting. See
 * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.2 /
 * Tier 1.
 *
 * Mirrors `useToolChunkStream.ts`'s contract exactly: a single per-block
 * subscription, pushing into the shared `StreamFlushQueue` rather than
 * dispatching its own flush/batch — a second independent RAF/`batch()`
 * here would reintroduce the reconcileArrays/replaceChild crash
 * documented in RETRO_REPLACECHILD_CRASH_2026-06-06.md.
 *
 * Installed at BODY scope by the caller (called directly, not from
 * inside `onMount`) — same early-return-safety rationale as
 * `useToolChunkStream`.
 *
 * This hook only knows about the START signal. The matching completion
 * data (real `trigger`/`preTokens`/`postTokens`/`durationMs`) arrives
 * later as a `compact_boundary` frame on the normal NDJSON stream —
 * handled directly in `useAgentStream.ts`, which also clears the
 * `compacting` pane flag this hook sets.
 *
 * Codex P1 on PR #2378 (two rounds): this event is published with
 * `persist: 0` (`wps_client.rs`) — never retained/replayed — because
 * there is no completion tombstone (`compact_boundary` arrives over
 * the separate NDJSON stream, not WPS), so a replayed "started" ping
 * is indistinguishable from a genuinely active one and a timestamp-
 * age guard alone cannot fix that on the receiving end: a pane
 * reconnecting seconds after a real, already-finished compaction
 * would still fall well within any plausible-duration window. Only a
 * currently-live subscriber ever sees this event now. The `startedAt`
 * staleness check below is kept as defense-in-depth against a
 * malformed/delayed live delivery, not as the mechanism preventing
 * stale replays — that's `persist: 0`'s job.
 *
 * SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md: a ping can
 * ALSO arrive too EARLY — before a freshly-mounted pane's `turnPhase` has
 * been reconciled out of the mount-default `Idle` (`ReconcileTurnActive`
 * is an async RPC round-trip). The reducer's `workingFromPhase` gate
 * correctly rejects that first attempt (round 5's orphan-state guard —
 * see reducer.ts), and since this channel has no replay, the ping would
 * otherwise be lost forever, leaving `compacting` null for the rest of
 * that compaction and eventually letting the liveness watchdog force the
 * "Working…" row off mid-compaction. This hook buffers a rejected ping
 * locally and retries the exact same dispatch once `turnPhase` is next
 * confirmed working — reusing the reducer's own accept/reject gates
 * unchanged rather than adding new staleness logic here.
 */

import { createEffect, onCleanup } from "solid-js";
import type { Accessor } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { workingFromPhase, type AgentPaneEvent, type TurnPhase } from "@/app/store/agent-pane-state/types";
import type { CompactionStartedNode } from "../types";
import type { StreamFlushQueue } from "../stream-flush-queue";

export interface UseCompactionStreamOptions {
    blockId: string;
    model: AgentPaneModel;
    queue: StreamFlushQueue;
    /** In-batch dedup cache shared with the rest of useAgentStream's producers. */
    hasNodeId: (id: string) => boolean;
    addNodeId: (id: string) => void;
    /**
     * Reactive turn phase for this pane — used ONLY to know when to retry a
     * ping the reducer rejected while not yet working (see the module doc
     * comment's SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02
     * note). Threaded down from `useAgentStream.ts`'s own `turnPhaseAtom`.
     */
    turnPhase: Accessor<TurnPhase>;
}

/**
 * A `compaction_started` ping older than this is treated as stale —
 * almost certainly a WPS replay of a compaction that already
 * finished (its `compact_boundary` completion arrived over the
 * separate NDJSON stream while nobody was subscribed to see it), not
 * a real one still in flight. The real captured example in the spec
 * doc (§2) took ~232s; 10 minutes is a generous multiple of that so
 * a genuinely slow compaction is never misclassified as stale.
 */
const MAX_PLAUSIBLE_COMPACTION_MS = 10 * 60 * 1000;

/**
 * Tolerance for `startedAt` appearing slightly in the future relative
 * to this client's clock (delivery jitter / minor clock skew between
 * this machine and wherever the hook process's clock reads from —
 * both are local to the same host today, but this stays defensive).
 * A genuinely stale replay is stale by minutes, not milliseconds, so
 * this tolerance can't mask the bug the age check exists to catch.
 */
const CLOCK_SKEW_TOLERANCE_MS = 60 * 1000;

export type CompactionTrigger = "manual" | "auto";

/**
 * A buffered (rejected-while-not-working) ping is only retried within this
 * long of ITS OWN rejection — not the compaction's own age. codex P2 on
 * this PR: without a bound, a ping that was actually genuinely stale (its
 * turn already ended before the ping arrived) can sit buffered
 * indefinitely if the confirming `ReconcileTurnActive(active: false)`
 * doesn't itself change `turnPhase` (already `Idle` ⇒ a same-ref no-op, no
 * reactive signal this hook can observe) — the retry effect then only
 * ever fires on the NEXT working-phase transition, which could be an
 * entirely unrelated `TurnStart` from the user's next message, minutes
 * later. Re-dispatching a stale ping against that unrelated turn can get
 * falsely accepted (no matching `compact_boundary` was ever recorded to
 * reject it against), setting `compacting` for a turn that was never
 * actually compacting. Bounding the retry to shortly after the ORIGINAL
 * rejection (comfortably longer than the `ReconcileTurnActive` RPC
 * round-trip this fix targets, comfortably shorter than "the user's next
 * message") keeps the fix scoped to the reconciliation race it exists
 * for, without needing to distinguish "which kind of transition" caused
 * the retry effect to re-fire.
 */
export const MISSED_PING_RETRY_WINDOW_MS = 15 * 1000;

/**
 * Validate + resolve a raw `compaction_started` WPS payload into a
 * trigger + clamped `startedAt`, or `null` to reject it outright
 * (malformed shape, unparseable/missing timestamp, or stale replay —
 * see the module doc comment). Pure and exported so the staleness
 * logic has direct unit coverage without spinning up a SolidJS
 * reactive-root harness for the subscription wiring itself.
 */
export function resolveCompactionStart(
    data: unknown,
    now: number,
): { trigger: CompactionTrigger; startedAt: number } | null {
    if (!data || typeof data !== "object") return null;
    const d = data as Record<string, unknown>;
    const trigger: CompactionTrigger | null =
        d.trigger === "auto" ? "auto" :
        d.trigger === "manual" ? "manual" :
        null;
    if (!trigger) return null;

    // Malformed/missing startedAt degrades to "reject" (fail closed),
    // not "treat as fresh" — same "skip rather than guess" philosophy
    // as the backend translator. In practice this binary always sends
    // a valid RFC3339 timestamp (precompact.rs), so a parse failure
    // here would itself indicate something is wrong worth not acting on.
    const rawStartedAt = typeof d.startedAt === "string" ? Date.parse(d.startedAt) : NaN;
    if (Number.isNaN(rawStartedAt)) return null;
    const age = now - rawStartedAt;
    if (age < -CLOCK_SKEW_TOLERANCE_MS || age > MAX_PLAUSIBLE_COMPACTION_MS) return null;
    // Clamp a within-tolerance "future" timestamp to now, so the live
    // elapsed-time display (Date.now() - startedAt) never briefly
    // reads negative.
    return { trigger, startedAt: Math.min(rawStartedAt, now) };
}

/**
 * Whether the reducer actually accepted a dispatched `CompactionStarted`
 * command. The reducer's round-5 fix (`workingFromPhase` gate) makes it a
 * no-op — empty `events`, state unchanged — when this ping arrives after
 * the turn's own `TurnEnd` already fired on the separate NDJSON stream, a
 * real and expected race given `compaction_started`'s separate WPS
 * transport. Pure and exported for direct unit coverage (reagent P1, PR
 * #2378 round 6): without this check, a stray ping the reducer correctly
 * rejected would still get a permanent "Compacting conversation…"
 * transcript node pushed for a compaction that isn't actually happening.
 */
export function wasCompactionStartedAccepted(paneEvents: AgentPaneEvent[]): boolean {
    return paneEvents.some((ev) => ev.type === "compaction-started");
}

export function useCompactionStream(opts: UseCompactionStreamOptions): void {
    // Set only when the reducer rejects a ping while `turnPhase` isn't
    // (yet) working — see the module doc comment. Overwritten by any newer
    // ping; cleared unconditionally after a retry attempt (accepted or
    // not) so this never grows into a retry loop or a replay queue.
    // `bufferedAtMs` is THIS process's local wall-clock at the moment of
    // the original (rejected) live delivery — see `MISSED_PING_RETRY_WINDOW_MS`
    // below for why it's tracked separately from the ping's own `startedAt`.
    let missedPing: { trigger: CompactionTrigger; startedAt: number; bufferedAtMs: number } | null = null;

    // `bufferOnReject` distinguishes the two call sites: a LIVE delivery
    // (fresh WPS ping) should buffer on rejection so the retry effect below
    // gets a chance to re-dispatch once the pane is confirmed working. The
    // RETRY itself must NOT re-buffer on a second rejection (e.g. the ping
    // has since gone genuinely stale — its own `compact_boundary` already
    // arrived) — reagent P1: without this, `missedPing` stayed truthy and
    // the retry effect re-fires on every subsequent `turnPhase` object
    // change (near-constant while Streaming, since most stream events
    // replace the phase object), re-dispatching and re-rejecting forever
    // for the rest of the working session instead of giving up after one
    // retry as documented.
    const attemptStart = (trigger: CompactionTrigger, startedAt: number, bufferOnReject: boolean) => {
        const paneEvents = opts.model.dispatchPane({ type: "CompactionStarted", trigger, at: startedAt });
        if (!wasCompactionStartedAccepted(paneEvents)) {
            missedPing = bufferOnReject ? { trigger, startedAt, bufferedAtMs: Date.now() } : null;
            return;
        }
        missedPing = null;

        const node: CompactionStartedNode = {
            type: "compaction_started",
            id: `compaction-started-${startedAt}`,
            trigger,
            startedAt,
        };
        if (!opts.hasNodeId(node.id)) {
            opts.addNodeId(node.id);
            opts.queue.pushNewNode(node);
            opts.queue.scheduleFlush();
        }
    };

    const unsub = waveEventSubscribe({
        eventType: WpsEvent.CompactionStarted,
        scope: `block:${opts.blockId}`,
        handler: (event: any) => {
            const resolved = resolveCompactionStart(event?.data, Date.now());
            if (!resolved) return;
            attemptStart(resolved.trigger, resolved.startedAt, /* bufferOnReject */ true);
        },
    });

    // Retry a buffered ping the instant this pane is next confirmed
    // working — see SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md
    // §3. Re-dispatches the SAME command; the reducer's existing
    // `workingFromPhase` / `isStaleVsLastBoundary` gates decide accept vs.
    // reject exactly as they would for a live delivery, so a ping that's
    // since gone genuinely stale (its own `compact_boundary` already
    // arrived) is still correctly rejected here, not resurrected. Expired
    // pings (see `MISSED_PING_RETRY_WINDOW_MS`) are dropped silently
    // instead of retried, whichever working-phase transition this is.
    createEffect(() => {
        if (!workingFromPhase(opts.turnPhase()) || !missedPing) return;
        const { trigger, startedAt, bufferedAtMs } = missedPing;
        if (Date.now() - bufferedAtMs > MISSED_PING_RETRY_WINDOW_MS) {
            missedPing = null;
            return;
        }
        attemptStart(trigger, startedAt, /* bufferOnReject */ false);
    });

    // Own the subscription at body scope so it is torn down even if the
    // caller's onMount early-returns (e.g. enabled:false). Without this
    // the global handler would leak one per mount.
    onCleanup(() => { try { unsub(); } catch { /* ignore */ } });
}
