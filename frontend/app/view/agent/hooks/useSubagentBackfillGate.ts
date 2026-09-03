// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useSubagentBackfillGate — tracks whether THIS block's own subagent/
 * dispatch history is still cold-backfilling on pane (re)open, so
 * `block.tsx`'s `ready()` gate (the BrainSpinner overlay) can stay up until
 * it's genuinely done, instead of fading as soon as this block's own
 * `blockData`/`viewModel` resolve — which happens well before the Activity
 * Dock's data (a separate, app-wide singleton source) has settled. See
 * docs/retro/retro-activity-dock-flicker-survives-debounce-fix-2026-08-24.md
 * §5, options 1-2.
 *
 * Generic over every block type by design (called unconditionally from
 * `block.tsx`, same as `ready()` itself) — resolves immediately for any
 * block whose `viewType()` isn't `"agent"`, since only agent panes ever
 * have subagent history to backfill in the first place.
 *
 * reagentx P1 + codex P1 (PR #2781, round 2): an earlier version defaulted
 * `settled` to `true` (only flipping `false` once a "started" status was
 * actually observed). That was backwards — `blockData()`/`viewModel()`
 * (block.tsx) typically resolve before this hook's async subscribe+RPC
 * round trip completes, so `ready()` would go true→false→true, and because
 * content is wrapped in `<Show when={ready()}>`, that unmounted/remounted
 * the real pane content instead of merely covering it with the spinner.
 * Fixed (that round) by defaulting to NOT settled for agent blocks and
 * resolving non-agent blocks to `true` immediately.
 *
 * reagentx P0 (PR #2781, round 4): that fix was still unsound for the
 * COMMON case — an agent block WITH a persisted session id (i.e. every
 * ordinary reopen of a previously-used agent). On a genuine first mount,
 * `scan_session_subagents` (agentmux-srv/src/server/reactive.rs) hasn't
 * necessarily started broadcasting yet by the time this hook's
 * mount-time history read resolves, so the read comes back EMPTY — the
 * previous version treated "empty history" as "nothing will ever happen
 * here" and resolved `settled=true` immediately, exactly as unsound as the
 * round-2 bug it was meant to fix. The REAL "started" then arrived moments
 * later once the backend's registration-triggered scan actually ran,
 * flipping `settled` back to `false` — unmounting the just-mounted pane,
 * which (per round 2's own finding) re-registers and re-triggers ANOTHER
 * backfill scan, repeating indefinitely on every reopen of any agent with
 * a persisted session.
 *
 * Fixed by never inferring "will a backfill happen at all" from the
 * (inherently racy) history read. The caller now passes
 * `hasPersistedSession` — read synchronously from this block's own
 * `blockData().meta["agent:sessionid"]`, the EXACT same condition
 * `server/reactive.rs` gates `scan_session_subagents` on — so this hook
 * knows definitively, with no async round trip involved, whether a
 * backfill is coming. Only a block with NO persisted session (a fresh
 * agent — `scan_session_subagents` is never even called for it) resolves
 * immediately; every persisted-session agent block waits for a REAL
 * "done" (live or historical), never short-circuiting on an empty read.
 * A generous safety-net timeout still guards against getting stuck forever
 * if "done" never arrives for some unrelated reason (e.g. a dropped WS
 * connection) — failing open, not stuck, same posture as every other
 * safety net in this hook.
 *
 * Mirrors `useResumeRetryStream.ts`'s live-subscribe + mount-time
 * `EventReadHistoryCommand` read + discard-on-live-race shape.
 *
 * 2026-09-02 (docs/retro/retro-activitydock-appears-on-agent-pane-load-2026-09-02.md,
 * following up on docs/reports/REPORT_AGENT_PANE_LOAD_RENDER_ARCHITECTURE_2026_08_27.md §2-3):
 * the settle decision on a "done" event used to be a blind
 * `DOCK_SETTLE_BUFFER_MS` (250ms) timer, guessed to give
 * `subagent-source.ts`/`dispatch-source.ts`'s own independently-triggered,
 * debounced refresh (`backfill-tracker.ts`'s `onNextBackfillSettle` →
 * fire-and-forget `refreshNow()`) time to land before revealing the pane.
 * There was no actual relationship between that guess and the real refresh
 * — for a heavy agent (verified live: ~20 subagents replayed on reopen),
 * the refresh's own RPC round trip could still be in flight well past
 * 250ms, so the spinner faded and revealed a stale Activity Dock snapshot
 * moments before the real data caught up and visibly corrected it — a
 * flash on load, confirmed live via a CDP `MutationObserver` trace. Fixed
 * by directly awaiting `refreshSubagentsNow()`/`refreshDispatchesNow()`
 * (an explicit, non-debounced refresh of the exact data the dock renders)
 * instead of guessing how long some OTHER refresh might take.
 */

import { createEffect, createSignal, onCleanup, type Accessor } from "solid-js";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { refreshSubagentsNow } from "../activity/subagent-source";
import { refreshDispatchesNow } from "../activity/dispatch-source";

/**
 * Resolve a raw `subagent:backfill_status` WPS payload into `"started"` /
 * `"done"`, or `null` to ignore it (malformed shape). Pure and exported for
 * direct unit coverage, same rationale as `resolveResumeRetryEvent`.
 */
export function resolveBackfillStatus(data: unknown): "started" | "done" | null {
    if (!data || typeof data !== "object") return null;
    const status = (data as Record<string, unknown>).status;
    return status === "started" || status === "done" ? status : null;
}

/**
 * Safety net (PR #2781 round 4): fail open rather than get stuck forever
 * if "done" never arrives for a block that genuinely has a backfill
 * pending — comfortably above the ~14s worst-case total settle time
 * measured for a very heavy real backfill in
 * docs/reports/REPORT_AGENT_PANE_REOPEN_SUBAGENT_STORM_2026_08_23.md §1.
 */
const SETTLE_SAFETY_TIMEOUT_MS = 20_000;

export function useSubagentBackfillGate(
    blockId: string,
    viewType: Accessor<string | undefined>,
    hasPersistedSession: Accessor<boolean>,
): Accessor<boolean> {
    const [settled, setSettled] = createSignal(false);
    // reagentx P1 (PR #2781, round 8): whether "does this block have a
    // backfill coming at all" has already been decided for the CURRENT
    // "agent" stay. `hasPersistedSession()` is only read below while
    // `!decided` — so once it's been consulted once, it's no longer a
    // tracked dependency of this effect, and a LATER change to it can't
    // trigger a re-run that reconsiders the decision. This matters because
    // `hasPersistedSession()` genuinely DOES flip false→true for the most
    // common flow in the whole app: a brand-new (non-continued) agent
    // conversation is created with `agent:sessionid: ""` (agent-model.ts),
    // so at mount there's correctly nothing to backfill — but the instant
    // the CLI's first turn captures a real session id, `persist_session_id`
    // (core.rs) writes it to block meta and broadcasts `waveobj:update`,
    // which is EXACTLY the same signal this hook reads to decide
    // `hasPersistedSession()`. Without freezing the decision, that ordinary
    // mid-conversation write re-triggered this effect, re-closed the gate
    // over already-live, already-streaming content, and — since
    // `scan_session_subagents` is a ONE-TIME, registration-time-only check
    // (server/reactive.rs) that is never retroactively re-run once a
    // session id shows up later — no "started"/"done" event could ever
    // arrive to reopen it, so the BrainSpinner incorrectly reappeared for
    // the full 20s safety-timeout on essentially every first message of
    // every new agent conversation.
    let decided = false;
    let safetyTimer: ReturnType<typeof setTimeout> | undefined;
    // Bumped on every "started" and on unmount — guards the async settle
    // refresh below against resolving into a stale cycle (a new "started"
    // reopening the gate, or the hook tearing down, while the refresh from
    // a PRIOR "done" is still in flight).
    let settleGeneration = 0;
    let disposed = false;

    createEffect(() => {
        const vt = viewType();
        if (vt === undefined) return; // not yet resolved — stay gated, nothing to decide yet
        if (vt !== "agent") {
            // A genuine view-type change (e.g. "Replace With...") — allow a
            // later re-entry into "agent" to decide fresh (round 6).
            decided = false;
            setSettled(true);
            return;
        }
        if (decided) return;
        decided = true;

        if (!hasPersistedSession()) {
            // Only an agent block WITH a persisted session id ever gets a
            // backfill_status event at all (server/reactive.rs gates
            // scan_session_subagents on the exact same condition) —
            // resolve immediately, rather than via the same async path.
            // See this module's own doc comment (round 4) for why this
            // must be known synchronously, not inferred from an empty
            // history read. Deliberately NOT re-evaluated later (see
            // `decided`'s own doc comment, round 8) — a session id
            // appearing after this point is an ordinary mid-conversation
            // write, not evidence a backfill is now pending.
            setSettled(true);
            return;
        }

        disposed = false; // fresh decide cycle — see round-6's re-entry note above
        setSettled(false);

        // reagentx P2 (PR #2781, round 7): re-armed on EVERY "started", not
        // just this initial wiring — `backfill_generation` (scan.rs)
        // explicitly supports a later, legitimate overlapping
        // re-registration for the same block re-closing this gate (a fresh
        // "started" arriving well after the first cycle already settled).
        // A safety net that only ever fires once per mount would leave
        // that LATER cycle with no rescue at all if its own "done" is
        // somehow never observed (e.g. a dropped WS connection) — settled
        // would then stay false, and the BrainSpinner gated, for the rest
        // of this mount's life.
        const armSafetyTimer = () => {
            clearTimeout(safetyTimer);
            safetyTimer = setTimeout(() => {
                console.log(
                    `[useSubagentBackfillGate] block ${blockId}: no backfill "done" after ${SETTLE_SAFETY_TIMEOUT_MS}ms, revealing anyway`
                );
                setSettled(true);
            }, SETTLE_SAFETY_TIMEOUT_MS);
        };
        armSafetyTimer();

        let receivedLiveEvent = false;
        // Wait for the dock's ACTUAL data (subagents + dispatches) to catch
        // up with the settled backend state, rather than guessing a fixed
        // duration — see this module's doc comment for why a blind timer
        // raced ahead of the real refresh on a heavy agent. `myGeneration`
        // makes this resolution a no-op if superseded by a later "started"
        // or the hook unmounting before the refresh lands.
        const scheduleSettle = () => {
            const myGeneration = ++settleGeneration;
            void Promise.all([refreshSubagentsNow(), refreshDispatchesNow()])
                .catch(() => {
                    // Best-effort — fall through to settle anyway rather than
                    // stay gated forever; the safety-net timer above still
                    // covers a hang in either RPC.
                })
                .then(() => {
                    if (disposed || myGeneration !== settleGeneration) return;
                    clearTimeout(safetyTimer);
                    setSettled(true);
                });
        };
        const applyStatus = (data: unknown) => {
            const status = resolveBackfillStatus(data);
            if (status === "started") {
                settleGeneration++; // invalidate any in-flight settle refresh from a prior cycle
                setSettled(false);
                armSafetyTimer();
            } else if (status === "done") {
                scheduleSettle();
            }
        };

        const unsub = waveEventSubscribe({
            eventType: WpsEvent.SubagentBackfillStatus,
            scope: `block:${blockId}`,
            handler: (event: any) => {
                receivedLiveEvent = true;
                applyStatus(event?.data);
            },
        });

        // Explicit current-history read on mount — same rationale as
        // useResumeRetryStream.ts's identical read: subscribe-time replay
        // alone doesn't cover a same-connection remount, and here the
        // backend's scan can even outrace a genuinely first-ever mount's
        // subscription. `maxitems: 2` reads the latest started/done pair
        // (backend publishes with `persist: 2`); the last element is
        // current truth. An EMPTY result here does NOT mean "resolve
        // settled" (see this module's own doc comment, round 4) — it just
        // means the backend's scan hasn't started broadcasting yet; the
        // live subscribe above (already registered) will catch the real
        // pair once it does, and the safety-net timer covers the case
        // where it somehow never does.
        void RpcApi.EventReadHistoryCommand(TabRpcClient, {
            event: WpsEvent.SubagentBackfillStatus,
            scope: `block:${blockId}`,
            maxitems: 2,
        })
            .then((history) => {
                if (receivedLiveEvent) return;
                const latest = history?.[history.length - 1];
                if (latest) applyStatus(latest.data);
            })
            .catch((e) => {
                console.log("[useSubagentBackfillGate] failed to load initial backfill status", e);
            });

        onCleanup(() => {
            // reagentx P2 (PR #2781, round 6): a genuine view-type change
            // away from "agent" tears this down via the effect's own
            // re-run — `decided` is reset in the `vt !== "agent"` branch
            // above, so a LATER change back to "agent" re-wires from
            // scratch rather than staying silently disabled forever.
            disposed = true;
            // codex P2 + reagentx P1 (PR #2937): bump the generation here
            // too, not just on a live "started". Without this, a "done"-
            // triggered settle refresh still in flight when the view
            // switches away from "agent" and back (before the NEW cycle
            // gets its own "started"/"done") would resolve into the new
            // cycle — `disposed` is reset to `false` on re-entry and
            // `myGeneration` still equals `settleGeneration` (nothing else
            // bumped it), so the stale refresh would pass both guards and
            // prematurely settle the just-re-entered pane, reproducing the
            // exact bug this PR fixes.
            settleGeneration++;
            clearTimeout(safetyTimer);
            try { unsub(); } catch { /* ignore */ }
        });
    });

    return settled;
}
