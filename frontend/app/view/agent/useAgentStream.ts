// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentStream — SolidJS hook that subscribes to a block's subprocess output,
 * pipes it through the provider translator + stream parser, and feeds
 * the resulting DocumentNodes into SolidJS signals.
 *
 * This hook bundles several producers that all write into the same agent
 * document: the core NDJSON transform pipeline below (byte decode → line
 * buffering → JSON parse → token extraction → translate → parse → node),
 * tool-chunk streaming (`hooks/useToolChunkStream.ts`), persistent-shell
 * streaming (`hooks/useShellNodeStream.ts`), turn-lifecycle finalization
 * and watchdogs (`hooks/useTurnLifecycle.ts`), and pending-message
 * acceptance (`hooks/usePendingMessageAcceptance.ts`).
 *
 * IMPORTANT — crash history: `tool_chunk` MPS events used to call
 * dispatchDoc(ToolChunkAppend) directly, one immediate signal write per
 * chunk. During active tool streaming that meant many independent signal
 * writes, each triggering its own Solid reactive flush. When a chunk write
 * raced with a concurrent RAF-scheduled document flush (both live in the
 * same browser task), two separate runUpdates frames could interleave,
 * leaving the <Index> reconciler holding a stale `current` array →
 * replaceChild NotFoundError. (Retro: RETRO_REPLACECHILD_CRASH_2026-06-06.md;
 * see also SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md §3.1.)
 *
 * The fix — and the reason this file is split the way it is — is
 * `stream-flush-queue.ts`'s `StreamFlushQueue`: ONE shared
 * `requestAnimationFrame` call site and ONE shared `batch()` call site for
 * every producer below. EVERY producer (this file's own NDJSON loop, the
 * tool-chunk hook, the shell hook, and turn-lifecycle's "Interrupted by
 * user" row) pushes into that SAME queue instance instead of scheduling
 * its own flush or calling its own `batch()`. If you are adding a new
 * event-source producer, give it a `pushXxx` method on `StreamFlushQueue`
 * — do NOT give it its own RAF or `batch()` call.
 */

import { getFileSubject } from "@/app/store/mps";
import { batch, createEffect, onCleanup, onMount, type Accessor } from "solid-js";
import { createTranslator } from "./providers/translator-factory";
import { readsMainAgentUsage } from "./main-agent-usage";
import { createSessionStartDetector } from "./session-start";
import { contextReaderFor } from "./context-reader";
import type { PendingMessage } from "../../agent-model/state";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { DocumentNode } from "../../agent-model/types";
import { createThinkingHeadlineTracker } from "./status/thinking-headline";
import { CompactionSummaryTracker } from "./context-delivery";
import { workingFromPhase, type CompactionState, type TurnPhase } from "@/app/store/agent-pane-state/types";
import { getNodeIdSet } from "@/app/store/agent-document-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { createStreamFlushQueue } from "./stream-flush-queue";
import { createHidingStreamFlushQueue } from "./hiding-stream-flush-queue";
import { getBlockMetaKeyAtom } from "@/app/store/global";
import { CATCH_UP_META_KEY, clearCatchUpRequest, parseCatchUpRequest } from "./catch-up-request";
import { markTitleStale } from "@/app/store/title-stale";
import { createStreamMemoryReinjection } from "./stream-memory-reinjection";
import { createToolOutputNoter, startSharedZonePoll } from "./stream-side-channels";
import { FALLBACK_CONTEXT_WINDOW } from "./memory-reinjection";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import { contextWindowForModel } from "@/app/store/agent-pane-state/context-window";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { useToolChunkStream } from "./hooks/useToolChunkStream";
import { useShellNodeStream } from "./hooks/useShellNodeStream";
import { useCompactionStream } from "./hooks/useCompactionStream";
import { useDockClearStream } from "./hooks/useDockClearStream";
import { useResumeRetryStream } from "./hooks/useResumeRetryStream";
import { useBackgroundTaskRegistry } from "./hooks/useBackgroundTaskRegistry";
import { useTurnLifecycle } from "./hooks/useTurnLifecycle";
import { usePendingMessageAcceptance } from "./hooks/usePendingMessageAcceptance";
import type { BackgroundTaskView } from "@/app/store/rpc-api";
import { agentPerfStore } from "../../agent-model/perf-probe";
import { parseHistoryLines } from "./parseHistoryLines";
import { EchoLedger, TranscriptCursor, type TranscriptFileEvent, type TranscriptSettleLatch } from "./transcript-cursor";
import { createStreamPipeline, createStreamState, parseRecords, pushReleasedJekts, resetStream } from "./stream-pipeline";

export { fillCompactionCard, pushContextCompactedNodes } from "./compaction-cards";

const OutputFileName = "output";
/** Longest live records wait for the history load before being placed anyway. */
const HISTORY_HOLD_MAX_MS = 15_000;

interface UseAgentStreamOpts {
    blockId: string;
    /**
     * Per-pane model handle returned by `registerPane`. Threaded in so
     * the hook can dispatch via `model.dispatchPane` / `model.dispatchDoc`
     * — default-safe against post-unmount races (the model's `disposed`
     * flag is flipped before the underlying stores unregister). PR-4
     * of the cascade follow-up sequence. See `agent-pane-model.ts`.
     */
    model: AgentPaneModel;
    outputFormat: string;
    /** Reactive accessor for the pane document nodes (`model.document`). */
    documentNodes: Accessor<DocumentNode[]>;
    /**
     * The reducer turn-phase signal — read by the hook to detect
     * "was this a user-initiated stop?" at session_end so the
     * "⏹ Interrupted by user" markdown row can be appended for
     * durable visual confirmation. Replaces the legacy
     * `turnActiveAtom` / `stoppingAtom` props dropped in PR G; the
     * predicate is `turnPhase.kind === "Interrupting"`.
     */
    turnPhase: Accessor<TurnPhase>;
    /**
     * The reducer's live "compaction in progress" signal — read by
     * `useCompactionStream` to push the "Compacting conversation…"
     * transcript node whenever `state.compacting` transitions from `null`
     * to set, REGARDLESS of which dispatch caused it (a live-accepted
     * `CompactionStarted`, or a buffered `pendingCompactionPing` promoted
     * later by `ReconcileTurnActive`/`StreamFlushObserved` — see
     * SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md). Reacting
     * to this signal, rather than inspecting each dispatch call site's own
     * returned events, is what lets the promotion paths (dispatched from
     * `agent-view.tsx` and `stream-flush-queue.ts`, neither of which has
     * access to this hook's document-node infrastructure) still get their
     * transcript node pushed from one unified place.
     */
    compacting: Accessor<CompactionState | null>;
    /**
     * Pending queue shared with the composer's `sendMessage` path. On
     * `agent-message-accepted` events, the hook removes the matching
     * entry and promotes it to a `user_message` document node — this is
     * the visible "accepted" transition for the user.
     */
    pendingMessages?: Accessor<PendingMessage[]>;
    enabled: boolean;
    /**
     * Provider id (from CLI_CATALOG — "claude", "codex", "gemini", …).
     * Used to attribute completed-turn tokens to the right row in the
     * status-bar token-usage store. Optional for back-compat; missing
     * provider means tokens aren't aggregated (the per-pane stats still
     * work). Per SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §5.1.
     */
    provider?: string;
    /**
     * This pane's agent name (`block.meta.agentName`). Threaded to
     * `parser.setAgentId` so jekt direction detection works: an echoed
     * outgoing jekt has FROM == this agent and must render as an outgoing
     * bubble, not incoming (stream-parser `tryParseJekt`). Optional —
     * missing name means direction falls back to "incoming", the only
     * pre-echo behavior. SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2.
     */
    agentName?: string;
    /**
     * Forwarded verbatim to `usePendingMessageAcceptance` — see that hook's
     * `onTurnStartFromQueue` doc comment. Re-engages message-list auto-scroll
     * for a turn that starts from the queue-drain path (a queued message the
     * backend picked up), not just from this pane's own composer send.
     */
    onTurnStartFromQueue?: () => void;
    /** Every queued message the backend accepts, mid-turn included — see
     *  usePendingMessageAcceptance's `onPendingLeft`. */
    onPendingLeft?: () => void;
    /**
     * Where this pane's history load ended (Phase 5a-4,
     * `transcript-cursor.ts`). Live events are held until it settles, then
     * placed by line: duplicates of history dropped, gaps read. Absent: no
     * history load to wait for; events are placed from the first one.
     */
    transcriptSettle?: TranscriptSettleLatch;
}

/** A resumed conversation is a new stretch of work: the session title is judged afresh at the next message. */
function retitleThen<T>(blockId: string, run: () => T): T {
    markTitleStale(blockId);
    return run();
}

/**
 * Subscribe to subprocess output and parse it into styled DocumentNodes.
 */
export function useAgentStream({
    blockId,
    model,
    outputFormat,
    documentNodes,
    turnPhase,
    compacting,
    pendingMessages,
    enabled,
    provider,
    agentName,
    onTurnStartFromQueue,
    onPendingLeft,
    transcriptSettle,
}: UseAgentStreamOpts): Accessor<BackgroundTaskView[]> {
    // Mutable state that doesn't trigger re-renders: the line buffer, translator and parser accumulate
    // per-byte parse state across fileSubject callbacks (stream-pipeline.ts), and the node id set is
    // this hook's in-batch dedup cache that the producer hooks also consult via hasNodeId/addNodeId.
    const st = createStreamState(outputFormat, agentName);
    const hasNodeId = (id: string) => st.nodeIdSet.has(id);
    const addNodeId = (id: string) => { st.nodeIdSet.add(id); };

    // The single shared RAF-batching queue every producer in this hook pushes into (see this file's
    // top doc comment and stream-flush-queue.ts for why there must be exactly one). It is wrapped in
    // createHidingStreamFlushQueue BEFORE anything below sees it: `memoryReinjectionController.isHiding`
    // then gates every push uniformly, for every producer, with no change to any of them
    // (SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md §3.3).
    const rawQueue = createStreamFlushQueue(model);

    // The pane's context window for memory sizing: the meter's own reading when it has one; the
    // last-seen model's table entry next; FALLBACK_CONTEXT_WINDOW (shared with memory-reinjection.ts's
    // replay-time reconstruction) only before any model is known.
    const currentContextWindow = (): number =>
        paneSnapshot(blockId)?.context?.window ?? contextWindowForModel(st.lastSeenModelId) ?? FALLBACK_CONTEXT_WINDOW;
    // Fed every line in stream order: true for a new CLI session's `init`.
    const isNewSession = createSessionStartDetector();
    // The live status's "Thinking: …" (status/thinking-headline.ts).
    const readThinkingHeadline = createThinkingHeadlineTracker();
    // Only Claude Code's stream carries per-call usage in the shape the meter reads (main-agent-usage.ts).
    const readsUsage = readsMainAgentUsage(outputFormat);

    const memoryReinjectionController = createStreamMemoryReinjection({ blockId, agentName, model, contextWindow: currentContextWindow });
    const queue = createHidingStreamFlushQueue(rawQueue, memoryReinjectionController.isHiding);

    // A deferred reinjection or catch-up fires when the pane goes quiet, by whatever route: a turn
    // the controller dropped (an account switch) ends with no session_end, and nothing else would drain it.
    createEffect(() => {
        if (!enabled || !blockId) return;
        if (!workingFromPhase(turnPhase())) memoryReinjectionController.maybeFireDeferred();
    });

    // The user resumed this conversation (catch-up-request.ts): the agent says where it is picking
    // up. The request is cleared whether or not it is acted on, so it cannot fire again.
    createEffect(() => {
        const stored = blockId ? getBlockMetaKeyAtom(blockId, CATCH_UP_META_KEY as never)() : undefined;
        if (!enabled || !blockId || stored === undefined || stored === null) return;
        const reason = parseCatchUpRequest(stored, Date.now());
        void clearCatchUpRequest(blockId).catch(() => {});
        if (reason) void retitleThen(blockId, () => memoryReinjectionController.catchUp(null, reason));
    });

    // Tool-chunk and persistent-shell streaming subscriptions, installed at body scope (not inside
    // onMount) so they tear down even if onMount early-returns (enabled:false). Both push into `queue`
    // rather than scheduling their own flush.
    useToolChunkStream({ blockId, queue, onOutput: createToolOutputNoter(model) });
    useShellNodeStream({ blockId, queue });
    useCompactionStream({ blockId, model, queue, hasNodeId, addNodeId, compacting });
    // dock:clear is a rare, out-of-band mutation of one existing node, not a streaming producer. It
    // uses model.dispatchDoc (disposal-safe), since this MPS handler can fire after the pane unregisters.
    useDockClearStream({ blockId, model });
    useResumeRetryStream({ blockId, model });
    // Seeds/refreshes attachedTask from the durable db_background_tasks registry (additive-only
    // relative to the transcript-derived path) and returns the raw task list, forwarded to the caller
    // so the dock can render registry-known rows the transcript has no record of
    // (docs/reports/REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md Tier 1).
    const backgroundTasksAtom = useBackgroundTaskRegistry({ blockId, model });

    onMount(() => {
        if (!enabled || !blockId) return;

        // Reset state on new subscription
        st.lineBuffer = "";
        st.translator = createTranslator(outputFormat);
        st.contextReader = contextReaderFor(outputFormat);
        st.compactionSummaries = new CompactionSummaryTracker();
        st.parser = new ClaudeCodeStreamParser();
        if (agentName) st.parser.setAgentId(agentName);
        st.nodeIdSet = new Set();
        queue.resetNodeQueues();

        // Turn-lifecycle finalization (session_end / Esc-fallback / crash grace timer) and the
        // stuck-stream watchdog. `finalizeTurn` is called on the real `session_end` StreamEvent.
        const { finalizeTurn } = useTurnLifecycle({
            blockId,
            model,
            turnPhase,
            provider,
            queue,
            // Turn end without session_end (Esc / crash): release held jekts too.
            flushParserPending: () => {
                st.parser.flushPending();
                pushReleasedJekts(st, queue);
            },
            hasNodeId,
            addNodeId,
        });

        // Promotes accepted pending messages into user_message document nodes. A sent message's row
        // shows at once under a client id; when its record is placed, the row takes the id history
        // gives that record, so a reload finds the same row (collapsed state, a scroll anchor).
        // Retried a few frames while the row still waits in the flush queue.
        const rekeyAccepted = (id: string, record: string, at: { gen: string; line: number }, tries = 0): void => {
            const replayed = parseHistoryLines([record], outputFormat, agentName, undefined, { idFirstLine: at.line, idGen: at.gen })
                .nodes.find((n) => n.type === "user_message");
            if (!replayed || replayed.id === id) return;
            const events = batch(() => model.dispatchDoc({ type: "RekeyNode", from: id, to: replayed.id }));
            if (events.length > 0) addNodeId(replayed.id);
            else if (tries < 5 && queue.pendingNewNodes().some((n) => n.id === id)) {
                requestAnimationFrame(() => rekeyAccepted(id, record, at, tries + 1));
            }
        };
        const echoLedger = new EchoLedger(rekeyAccepted);
        usePendingMessageAcceptance({
            blockId,
            model,
            pendingMessages,
            queue,
            hasNodeId,
            addNodeId,
            onTurnStartFromQueue,
            onPendingLeft,
            onAccepted: (text, nodeId) => echoLedger.accepted(text, nodeId),
        });

        // Seed the in-batch dedup cache from the reducer-maintained index, which is always current.
        st.nodeIdSet = new Set(getNodeIdSet(blockId));

        // The parser's skip-set is a callback (NOT a static snapshot) that tracks the live document's
        // `nodeIdSet`: resumed-session snapshots are restored asynchronously via `useHistoryPagination`
        // -> `HistoryLoaded`, so a snapshot taken at mount would be empty and the parser's first
        // `node_0` would collide with the restored `node_0`.
        st.parser = new ClaudeCodeStreamParser({
            skipIds: () => getNodeIdSet(blockId),
        });
        if (agentName) st.parser.setAgentId(agentName);

        // Two reducers signaled in lockstep: pane-state owns the streaming metadata (active flag),
        // agent-document owns the session phase gate that drives truncate suppression.
        const subscribedAt = Date.now();
        model.dispatchPane({ type: "StreamSubscribe", at: subscribedAt });
        model.dispatchDoc({ type: "SessionStart", at: subscribedAt });

        const fileSubject = getFileSubject(blockId, OutputFileName);

        console.debug(`[useAgentStream] subscribed blockId=${blockId} format=${outputFormat}`);
        const pipeline = createStreamPipeline(st, {
            blockId,
            agentName,
            model,
            documentNodes,
            queue,
            rawQueue,
            memory: memoryReinjectionController,
            currentContextWindow,
            isNewSession,
            readThinkingHeadline,
            readsUsage,
            finalizeTurn,
        });

        // Places each transcript record by its line (Phase 5a-4, transcript-cursor.ts): duplicates
        // dropped, gaps read, and nothing parsed before the history it follows. An echo (the user
        // message the controller wrote to the agent's stdin, whose node the pane already shows) is not
        // parsed; the ledger pairs it with that node.
        const cursor = new TranscriptCursor({
            deliver: (text, from) => parseRecords(pipeline, text, from),
            echo: (text, from) => echoLedger.echoed(text, from),
            isOwnEcho: (line, at) => echoLedger.isOwnEcho(line, at),
            reset: (fileop) => resetStream(pipeline, fileop),
            readRange: async (offset, limit, expectGen) => {
                const resp = await RpcApi.BlockfileReadRangeCommand(
                    TabRpcClient,
                    { block_id: blockId, filename: OutputFileName, offset, limit, expect_gen: expectGen },
                    { timeout: 15_000 },
                );
                return { lines: resp?.lines ?? [], stream: resp?.stream, gen: resp?.gen, genMismatch: resp?.gen_mismatch };
            },
            log: (message, level) =>
                level === "warn" ? console.warn(`[useAgentStream] ${message}`) : console.debug(`[useAgentStream] ${message}`),
        });
        const unregisterCursorStats = agentPerfStore.registerTranscriptCursor(blockId, cursor.stats);
        const subscription = fileSubject.subscribe((msg: TranscriptFileEvent) => cursor.push(msg));

        // Live records wait for the history load (the pane is covered until then). A load that never
        // reports doesn't hold them for good: after HISTORY_HOLD_MAX_MS they are placed from the first event on.
        const stopWaiting = transcriptSettle
            ? transcriptSettle.onSettle((outcome) => cursor.settle(outcome))
            : (cursor.settle(null), () => {});
        const holdTimer = setTimeout(() => {
            if (cursor.isSettled()) return;
            console.warn(`[useAgentStream] no history outcome after ${HISTORY_HOLD_MAX_MS}ms; placing live records from the next one`);
            cursor.settle(null);
        }, HISTORY_HOLD_MAX_MS);

        const stopPoll = startSharedZonePoll(blockId, OutputFileName, cursor);

        onCleanup(() => {
            clearTimeout(holdTimer);
            stopPoll();
            stopWaiting();
            cursor.dispose();
            unregisterCursorStats();
            queue.cancelScheduledFlush();
            subscription.unsubscribe();
            // (the tool_chunk subscription is torn down by its own body-scope onCleanup, so it is
            // cleaned up even when this onMount early-returns.)
            // StreamUnsubscribe moves a working turn into the Disconnected phase, so a crash or exit
            // without session_end doesn't leave "Working…" stuck.
            const at = Date.now();
            model.dispatchPane({ type: "StreamUnsubscribe", at });
            // Defer SessionEnd to a microtask so it fires AFTER the synchronous disposal chain
            // completes: during error-boundary cleanup the <Key> streaming-buffer scope is still
            // partially live, and a synchronous document-nodes publish here re-triggers
            // reconcileArrays on a half-torn-down DOM -> replaceChild NotFoundError
            // (SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md §3.1). model.dispatchDoc is
            // the soft dispatchIfRegistered variant, so a gone slot is a silent no-op rather than a throw.
            queueMicrotask(() => model.dispatchDoc({ type: "SessionEnd", at }));
        });
    });

    return backgroundTasksAtom;
}
