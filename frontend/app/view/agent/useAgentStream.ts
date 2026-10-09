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
import { getObjectValue, makeORef } from "@/app/store/mos";
import { blockRoleColor, isLightThemeActive } from "@/app/block/pane-identity";
import { noteToolCall, noteToolResult } from "@/app/store/touched-files";
import { onCleanup, onMount, type Accessor } from "solid-js";
import { createTranslator } from "./providers/translator-factory";
import { modelTurnCommand } from "./model-turn-signal";
import { mainAgentRequestStarted, mainAgentStreamedChars, mainAgentUsage, readsMainAgentUsage } from "./main-agent-usage";
import { createSessionStartDetector } from "./session-start";
import type { PendingMessage } from "./state";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { ContextCompactedNode, DocumentNode, SessionOutcomeNode } from "./types";
import { noteTaskFrame } from "./activity/task-outcomes";
import { parseCompactBoundaryFrame, contextCompactedNodeId, contextCompactedLiveTimestamp } from "./compact-boundary";
import { createTaskWakeDetector } from "./task-wake";
import { compactionModelKey, parseCompactionSample, recordCompactionSample } from "./compaction-estimate";
import { CompactionSummaryTracker } from "./context-delivery";
import { parseSessionOutcomeFrame, sessionOutcomeNodeId, sessionOutcomeLiveTimestamp } from "./session-outcome";
import { workingFromPhase, type AgentPaneEvent, type CompactionState, type TurnPhase } from "@/app/store/agent-pane-state/types";
import { getNodeIdSet } from "@/app/store/agent-document-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { createStreamFlushQueue, type StreamFlushQueue } from "./stream-flush-queue";
import { createHidingStreamFlushQueue } from "./hiding-stream-flush-queue";
import { createMemoryReinjectionController } from "./memory-reinjection-controller";
import { FALLBACK_CONTEXT_WINDOW } from "./memory-reinjection";
import { buildMemoryInjectedNode, isMemoryInjectedFrame } from "./memory-injected";
import { parseCliNoticeFrame } from "./cli-notice";
import { MemoryDeliveryApi } from "@/app/store/rpc-api/memory-delivery";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import { fetchMemoryReinjectionEntries } from "./memory-reinjection-fetch";
import { contextWindowForModel } from "@/app/store/agent-pane-state/context-window";
import { reportedContextWindowsFromResult } from "@/app/store/agent-pane-state/context-reading";
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
import { agentPerfStore } from "./virtualization/perf-probe";
import { toolActivityArg } from "./tool-meta/tool-descriptors";
import { EchoLedger, TranscriptCursor, type TranscriptFileEvent, type TranscriptSettleLatch } from "./transcript-cursor";

const OutputFileName = "output";
/** Longest live records wait for the history load before being placed anyway. */
const HISTORY_HOLD_MAX_MS = 15_000;
/** How often a pane on an agent's shared zone checks for lines no event brought. */
const SHARED_ZONE_POLL_MS = 5_000;

/**
 * Shared node-construction for the reducer's `context-compacted` event —
 * used by BOTH the real `CompactionBoundary` path and the `TokensIn`
 * heuristic fallback, so the two never drift into different node shapes.
 * `source`/`trigger`/`durationMs` come straight from the event: "real"
 * carries all three, "heuristic" carries only `source`. See
 * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.3.
 */
export function pushContextCompactedNodes(
    paneEvents: AgentPaneEvent[] | undefined,
    queue: StreamFlushQueue,
    hasNodeId: (id: string) => boolean,
    addNodeId: (id: string) => void,
): ContextCompactedNode | null {
    // The last real card pushed: its `contextAfter` is filled in by the next
    // main-agent call (`fillCompactionCard`).
    let realCard: ContextCompactedNode | null = null;
    for (const ev of paneEvents ?? []) {
        if (ev.type !== "context-compacted") continue;
        // Codex P2, PR #2378 round 7 (id) / round 12 (timestamp + shared
        // helpers): a "real" event (source: "real", i.e. a genuine
        // compact_boundary frame) is keyed and timestamped via the SAME
        // functions parseHistoryLines.ts uses — see compact-boundary.ts's
        // doc comments for why sharing them, not independently
        // reimplementing the fallback logic in each file, is what actually
        // prevents the two consumers from drifting. The heuristic path has
        // no frame to key or time on, so it keeps its own Date.now()-based
        // id/timestamp — nothing in history replay can ever produce a
        // competing node for a heuristic-sourced detection to collide
        // with, so a live-only id is fine there.
        const id =
            ev.source === "real"
                ? contextCompactedNodeId({
                      trigger: ev.trigger,
                      preTokens: ev.tokensBefore,
                      postTokens: ev.tokensAfter,
                      durationMs: ev.durationMs,
                      uuid: ev.boundaryUuid,
                      frameTimestamp: ev.frameTimestamp,
                  })
                : `context-compacted-${Date.now()}`;
        const timestamp =
            ev.source === "real" ? contextCompactedLiveTimestamp(ev.frameTimestamp) : Date.now();
        const compactNode: ContextCompactedNode = {
            type: "context_compacted",
            id,
            tokensBefore: ev.tokensBefore,
            tokensAfter: ev.tokensAfter,
            timestamp,
            source: ev.source,
            trigger: ev.trigger,
            durationMs: ev.durationMs,
        };
        if (!hasNodeId(compactNode.id)) {
            addNodeId(compactNode.id);
            queue.pushNewNode(compactNode);
            queue.scheduleFlush();
            if (compactNode.source === "real") realCard = compactNode;
        }
    }
    return realCard;
}

/** `card` with the context's real size after it — the first main-agent call
 *  since the boundary — queued as an in-place update. */
export function fillCompactionCard(card: ContextCompactedNode, contextAfter: number, queue: StreamFlushQueue): void {
    queue.pushUpdatedNode({ ...card, contextAfter });
    queue.scheduleFlush();
}

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

/**
 * Subscribe to subprocess output and parse it into styled DocumentNodes.
 */
/** The colour an agent pane is drawn in (its focused border), if it has one. */
function agentColorOf(blockId: string): string | undefined {
    return blockRoleColor(getObjectValue<Block>(makeORef("block", blockId))?.meta, isLightThemeActive(), "identity");
}

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
    // Mutable state that doesn't trigger re-renders. Kept here (not
    // extracted) because it's tightly coupled to the NDJSON parse loop
    // below: lineBuffer/translator/parser accumulate per-byte parse state
    // across fileSubject callbacks, and nodeIdSet is this hook's in-batch
    // dedup cache that the extracted producer hooks also need to consult
    // via the hasNodeId/addNodeId closures passed to them.
    let lineBuffer = "";
    let translator = createTranslator(outputFormat);
    // Pairs each compact_boundary with Claude Code's summary frame, which
    // becomes a card instead of a user message (context-delivery.ts).
    let compactionSummaries = new CompactionSummaryTracker();
    let parser = new ClaudeCodeStreamParser();
    if (agentName) parser.setAgentId(agentName);
    // nodeIdSet remains for fast in-batch dedup (the reducer also dedups,
    // but checking here avoids enqueuing already-seen nodes into pendingNew).
    let nodeIdSet = new Set<string>();
    const hasNodeId = (id: string) => nodeIdSet.has(id);
    const addNodeId = (id: string) => { nodeIdSet.add(id); };

    // The single shared RAF-batching queue every producer in this hook
    // pushes into — see this file's top doc comment and
    // stream-flush-queue.ts's module doc for why there must be exactly one.
    //
    // Wrapped in createHidingStreamFlushQueue BEFORE anything below (or any
    // of the producer hooks it's handed to) ever sees it — see
    // docs/specs/SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md
    // §3.3. `memoryReinjectionController.isHiding` gates every push
    // uniformly, for every producer this hook has, with no changes needed
    // to any of them individually — exactly the point of there being one
    // choke point already.
    const rawQueue = createStreamFlushQueue(model);

    // The pane's context window for memory sizing: the meter's own reading
    // (reported by Claude Code, else the model table) when it has one; the
    // last-seen model's table entry next; FALLBACK_CONTEXT_WINDOW only
    // before any model is known. The fallback is shared with
    // memory-reinjection.ts's own replay-time reconstruction — see that
    // constant's doc comment for why 200K — rather than each picking the
    // number independently.
    let lastSeenModelId: string | undefined;
    const currentContextWindow = (): number =>
        paneSnapshot(blockId)?.context?.window ?? contextWindowForModel(lastSeenModelId) ?? FALLBACK_CONTEXT_WINDOW;
    // The API message the last TokensIn was for: one call is reported by its
    // message_start and again by its assistant frame(s) (main-agent-usage.ts),
    // and must count once.
    let lastUsageMessageId: string | undefined;
    // A real compaction card still waiting for the context's size after it.
    let awaitingCompactionSize: ContextCompactedNode | null = null;
    // Fed every line in stream order: true for a new CLI session's `init`.
    const isNewSession = createSessionStartDetector();
    // A pass the CLI starts for a finished background task (task-wake.ts):
    // the same detector parseHistoryLines uses, so live and replay agree.
    const detectTaskWake = createTaskWakeDetector();
    // Only Claude Code's stream carries per-call usage in the shape the meter
    // reads (main-agent-usage.ts); other providers' panes show no reading.
    const readsUsage = readsMainAgentUsage(outputFormat);
    // Output characters the main agent streamed since the last dispatch, for
    // the working row's counter: one OutputStreamed per batch of lines, not
    // per delta. Flushed before any call boundary so a call's characters are
    // never credited to the next one.
    let pendingStreamedChars = 0;
    const flushStreamedChars = () => {
        if (pendingStreamedChars <= 0) return;
        model.dispatchPane({ type: "OutputStreamed", chars: pendingStreamedChars });
        pendingStreamedChars = 0;
    };

    const memoryReinjectionController = createMemoryReinjectionController({
        contextWindow: currentContextWindow,
        now: () => Date.now(),
        // reagentx P0, PR #3502: a real turn is genuinely in flight for the
        // common auto-compaction case (compact_boundary lands mid an
        // ongoing turn) — dispatching TurnStart on top of it regresses
        // turnPhase from Streaming back to Submitting. Same read
        // agent-view.tsx's own handleSendMessage captures as
        // wasAlreadyWorking before deciding whether to dispatch TurnStart
        // at all. See memory-reinjection-controller.ts's module doc
        // comment, "fix 1".
        isPaneWorking: () => workingFromPhase(paneSnapshot(blockId)?.turnPhase ?? { kind: "Idle" }),
        fetchEntries: () => fetchMemoryReinjectionEntries(TabRpcClient, agentName ?? "", blockId),
        // Deliberately the raw send RPC, not the full sendMessage/pending-
        // zone path (§3.3 — that path is what creates the visible
        // UserMessageNode this feature must never produce). This DOES skip
        // sendMessage's own auth-guard/cmd:args-reapply preamble
        // (useAgentCommands.ts) — acceptable here because a reinjection
        // only ever fires immediately after a real compact_boundary, i.e.
        // strictly mid-session on an already-authenticated, already-running
        // process, not a cold user-initiated send where that preamble
        // matters. Flagged, not silently assumed.
        // `hidden: true` — reagentx P0, PR #3502, seventh review round:
        // extract_digest_text's marker-text suppression can only see inside
        // its own 32 KB / ~30-line tail window, which large Personal memory
        // content (measured up to 76 KB in this PR's own §3.4.1 numbers)
        // routinely scrolls past. This flag is the authoritative gate
        // (session::set_hidden_reinjection_active) — set here, not
        // reconstructed from transcript bytes later.
        sendRpc: (message, deliveryId) =>
            RpcApi.AgentInputCommand(TabRpcClient, {
                blockid: blockId,
                message,
                message_id: `memreinject_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
                hidden: true,
                // srv writes this delivery's card to the pane (CD2b).
                ...(deliveryId ? { delivery_id: deliveryId } : {}),
            }).then(() => undefined),
        // srv composes the message and the card (CD2b); an older srv rejects
        // and the controller composes here instead.
        compose: async (reason) => {
            const r = await MemoryDeliveryApi.ComposeCommand(TabRpcClient, { block_id: blockId, reason });
            if (!r.text || !r.delivery_id || !r.frame) return null;
            const node = buildMemoryInjectedNode(r.frame, {
                contextWindow: currentContextWindow(),
                now: Date.now(),
            });
            return node ? { deliveryId: r.delivery_id, text: r.text, node } : null;
        },
        // Claude Code's SessionStart hook now delivers the same memory at a
        // session start and after a compaction; this fallback fires only when
        // the hook didn't (SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2).
        claimFallback: (reason, boundaryUuid, eventAtMs) =>
            MemoryDeliveryApi.ClaimFallbackCommand(TabRpcClient, {
                block_id: blockId,
                reason,
                ...(boundaryUuid ? { boundary_uuid: boundaryUuid } : {}),
                ...(eventAtMs !== undefined ? { event_at_ms: eventAtMs } : {}),
            }).then((r) => r.deliver),
        // Reuses the REAL TurnStart/TurnStartFailed commands unmodified — a hidden
        // reinjection is a completely genuine turn state-machine-wise; only
        // its rendering differs. See memory-reinjection-controller.ts's
        // module doc comment. `content`/`hidden` are always exactly what the
        // controller itself passes (HIDDEN_TURN_PLACEHOLDER_CONTENT, true) —
        // this callback forwards verbatim rather than deciding anything, per
        // "fix 2" in that module's doc comment: the REAL memory content must
        // never enter reducer state, only sendRpc's own argument above.
        dispatchTurnStart: (content, hidden) => {
            model.dispatchPane({ type: "TurnStart", at: Date.now(), content, hidden }, "system");
        },
        dispatchTurnStartFailed: () => {
            model.dispatchPane({ type: "TurnStartFailed" }, "system");
        },
    });

    const queue = createHidingStreamFlushQueue(rawQueue, memoryReinjectionController.isHiding);

    // Tool-chunk and persistent-shell streaming subscriptions, installed at
    // body scope (not inside onMount) so they tear down even if onMount
    // below early-returns (e.g. enabled:false). Both push into `queue`
    // rather than scheduling their own flush.
    useToolChunkStream({ blockId, queue });
    useShellNodeStream({ blockId, queue });
    useCompactionStream({ blockId, model, queue, hasNodeId, addNodeId, compacting });
    // dock:clear doesn't push into `queue` — it's a rare, out-of-band
    // mutation of one existing node, not a streaming producer. Uses
    // model.dispatchDoc (disposal-safe), not the raw dispatch, since this
    // MPS handler can fire after the pane unregisters.
    useDockClearStream({ blockId, model });
    useResumeRetryStream({ blockId, model });
    // Seeds/refreshes attachedTask from the durable db_background_tasks
    // registry — see that hook's own module doc comment for why this is
    // additive-only (never clears) relative to the transcript-derived path.
    // Returns the raw task list too, forwarded to the caller so the dock can
    // render registry-known rows the transcript has no record of (Tier 1 of
    // docs/reports/REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md).
    const backgroundTasksAtom = useBackgroundTaskRegistry({ blockId, model });

    onMount(() => {
        if (!enabled || !blockId) return;

        // Reset state on new subscription
        lineBuffer = "";
        translator = createTranslator(outputFormat);
        compactionSummaries = new CompactionSummaryTracker();
        parser = new ClaudeCodeStreamParser();
        if (agentName) parser.setAgentId(agentName);
        nodeIdSet = new Set();
        queue.resetNodeQueues();

        // Jekts the parser held while a text/thinking block streamed, released
        // now that it ended: pushed as new nodes, BEFORE whatever node the
        // releasing event produced. SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md §2.2.
        const pushReleasedJekts = () => {
            for (const released of parser.drainReleased()) {
                if ((released as { timestamp?: number }).timestamp == null) {
                    (released as { timestamp?: number }).timestamp = Date.now();
                }
                if (hasNodeId(released.id)) continue;
                addNodeId(released.id);
                queue.pushNewNode(released);
            }
        };
        // Turn-lifecycle finalization (session_end / Esc-fallback / crash
        // grace timer) and the stuck-stream watchdog. `finalizeTurn` is
        // called below on the real `session_end` StreamEvent.
        const { finalizeTurn } = useTurnLifecycle({
            blockId,
            model,
            turnPhase,
            provider,
            queue,
            // Turn end without session_end (Esc / crash): release held jekts too.
            flushParserPending: () => {
                parser.flushPending();
                pushReleasedJekts();
            },
            hasNodeId,
            addNodeId,
        });

        // Promotes accepted pending messages into user_message document
        // nodes. No-ops internally if pendingMessages was not provided.
        const echoLedger = new EchoLedger();
        usePendingMessageAcceptance({
            blockId,
            model,
            pendingMessages,
            queue,
            hasNodeId,
            addNodeId,
            onTurnStartFromQueue,
            onPendingLeft,
            onAccepted: (text) => echoLedger.accepted(text),
        });

        // Seed the in-batch dedup cache from the reducer-maintained
        // index. Issue #728 gap 4 — replaces the per-mount scan of
        // `doc()` that could miss in-flight events arriving between
        // mount and scan. The reducer keeps `nodeIdSet` in lockstep
        // with `nodes[]` so this read is always current.
        nodeIdSet = new Set(getNodeIdSet(blockId));

        // Pass a callback (NOT a static snapshot) so the parser's
        // skip-set tracks the live document's `nodeIdSet`.
        //
        // Why a callback: resumed-session snapshots are restored
        // **asynchronously** via `useHistoryPagination` →
        // `HistoryLoaded`. A static snapshot captured at mount
        // would be empty (history hasn't landed yet), and the
        // parser's first `node_0` would collide with the
        // restored `node_0` from the snapshot — the very bug
        // this PR fixes. The callback reads the reducer's live
        // index at id-generation time, so by the time the agent
        // emits its first text event, the snapshot's ids are
        // already in the index and get skipped.
        //
        // Codex P1 #2 on PR #1101.
        parser = new ClaudeCodeStreamParser({
            skipIds: () => getNodeIdSet(blockId),
        });
        if (agentName) parser.setAgentId(agentName);

        // Two reducers signaled in lockstep: pane-state owns the streaming
        // metadata (active flag), agent-document owns the session phase
        // gate that drives truncate suppression.
        const subscribedAt = Date.now();
        model.dispatchPane({ type: "StreamSubscribe", at: subscribedAt });
        model.dispatchDoc({ type: "SessionStart", at: subscribedAt });

        const fileSubject = getFileSubject(blockId, OutputFileName);

        console.debug(`[useAgentStream] subscribed blockId=${blockId} format=${outputFormat}`);
        // A reset of the stream. `truncate` is today's: the reducer decides
        // whether to honor it — late truncates after a socket-reconnect race
        // are suppressed. Only reset the hook-local parser/stats/etc. when the
        // truncate is actually honored; if suppressed, the live stream is
        // still flowing and resetting would corrupt in-flight parse state.
        // `replace` and `delete` (restore, archive) only move the cursor:
        // they published nothing before Phase 5a-3b, and the pane keeps what
        // it shows, as it did then.
        const resetStream = (fileop: "truncate" | "replace" | "delete") => {
            if (fileop !== "truncate") {
                // Archive (`delete`) and restore (`replace`) swap the
                // conversation under the pane: the meter's reading measured
                // the old one. srv sends no `fresh` outcome for an archive,
                // so this is the only signal. The next call reports the new
                // size; without this, its smaller reading would also look
                // like a compaction to the TokensIn heuristic.
                model.dispatchPane({ type: "ContextInvalidated", reason: "transcript_replaced" });
                awaitingCompactionSize = null;
                return;
            }
            const events = model.dispatchDoc({
                type: "StreamTruncate",
                reason: "fileop",
            });
            const honored = events.some((e) => e.type === "truncate-applied");
            if (!honored) return;
            queue.resetAll();
            lineBuffer = "";
            translator.reset();
            parser.reset();
            nodeIdSet = new Set();
            // Reducer clears sessionStats/currentTool/turnTokens
            // and transitions turnPhase to Idle in one shot.
            model.dispatchPane({ type: "TurnReset" });
        };

        // Records the transcript cursor (below) has placed, parsed as live
        // input.
        const parseRecords = (text: string, from?: { stream: string; gen: string; line: number }) => {
            // Where each complete line sits in the transcript, so a tool node
            // records its result's line (SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_
            // 2026_10_01.md §3.2). Only when this text starts a fresh line: a
            // partial line held from before would shift every number.
            let sourceLine = from && lineBuffer === "" ? from.line : null;
            // Accumulate into line buffer and process complete lines
            lineBuffer += text;
            const lines = lineBuffer.split("\n");
            lineBuffer = lines.pop() || ""; // Keep incomplete line
            // Safety: drop the buffer if it grows absurdly large without a
            // newline. This should never happen in well-formed stream-json
            // output, but protects against runaway memory if something
            // upstream is sending garbage.
            if (lineBuffer.length > 10_000_000) {
                console.warn(`[useAgentStream] line buffer exceeded 10MB, dropping`);
                lineBuffer = "";
            }

            for (const line of lines) {
                // Every element is one transcript line, blank or not.
                parser.setSourceLine(sourceLine != null && from ? { stream: from.stream, gen: from.gen, line: sourceLine } : null);
                if (sourceLine != null) sourceLine++;
                const trimmed = line.trim();
                if (!trimmed) continue;

                // Fast path: non-JSON lines (subprocess echoes, CLI warnings).
                // Skip without the cost of a try/catch on a huge string.
                if (!trimmed.startsWith("{")) continue;

                // Try to parse as JSON
                let rawEvent: any;
                try {
                    rawEvent = JSON.parse(trimmed);
                } catch {
                    // Don't log the full line — it may be 100KB+ (e.g. a Write
                    // tool call with a long file content) and forwarding it
                    // through the IPC log pipe stalls the main thread.
                    // See docs/analysis/v0-33-91-ndjson-parse-crash-2026-04-12.md
                    console.warn(`[useAgentStream] JSON parse failed, len=${trimmed.length}`);
                    continue;
                }

                // Handle stderr events from subprocess
                if (rawEvent.type === "stderr" && rawEvent.text) {
                    const text = rawEvent.text.trim();
                    if (text.includes("Fast mode is not available") ||
                        text.includes("[WARN]") && text.length < 200) {
                        continue;
                    }
                    queue.pushNewNode({
                        id: `stderr-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
                        type: "markdown",
                        content: `**stderr:** ${text}`,
                        timestamp: Date.now(),
                    });
                    queue.scheduleFlush();
                    continue;
                }

                // Observe only — the frame keeps flowing below. A background
                // task's end is on this stream (`system/task_notification`);
                // the dock reads it from here so it never depends on srv's
                // live registry having watched the task (activity/task-outcomes.ts).
                if (rawEvent.type === "system") noteTaskFrame(blockId, rawEvent, Date.now());

                // A new CLI session, not a compaction's own `init`
                // (session-start.ts): a turn still holding live tokens ended
                // without a `result`, so its tokens go.
                if (isNewSession(rawEvent)) model.dispatchPane({ type: "StreamSessionStarted" });

                // Real compaction-boundary completion data (Tier 1/2 —
                // docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md).
                // Claude Code's `system`/`compact_boundary` frame arrives on this
                // same raw stdout stream (mirrors agentmux-srv's
                // `translator/claude.rs::handle_system_message`); intercepted here
                // directly — like the message_start/message_delta token
                // extraction below — rather than routed through the provider
                // translator, which has no StreamEvent shape for it. Parsing is
                // shared with `parseHistoryLines.ts`'s replay path via
                // `compact-boundary.ts` (Codex P1, PR #2378 round 2) so the two
                // can't drift on what counts as a valid frame.
                {
                    const wake = detectTaskWake(rawEvent);
                    if (wake && !hasNodeId(wake.id)) {
                        addNodeId(wake.id);
                        queue.pushNewNode(wake);
                        queue.scheduleFlush();
                    }
                }
                if (rawEvent.type === "system" && rawEvent.subtype === "compact_boundary") {
                    // Feeds only the compaction-time ESTIMATE (the working row's
                    // progress bar) with its own parser — it reads the frame's
                    // real stdout shape, and changes nothing below. See
                    // SPEC_COMPACTION_ESTIMATED_PROGRESS_AND_STREAM_FRAMES_2026_10_01.md §3/§5.
                    recordCompactionSample(
                        parseCompactionSample(
                            rawEvent,
                            compactionModelKey(getObjectValue<Block>(makeORef("block", blockId))?.meta, lastSeenModelId)
                        )
                    );
                    // Compaction happens MID-turn — flushParserPending() is
                    // only called at finalizeTurn (useTurnLifecycle.ts), so
                    // without an explicit flush here the parser's
                    // currentTextNode/currentThinkingNode accumulator never
                    // sees this line and keeps accumulating text from AFTER
                    // the compaction onto the SAME node id as text from
                    // BEFORE it — silently merging content across the
                    // boundary and rendering it before the compaction
                    // marker, live, not just on history replay (same root
                    // cause as the parseHistoryLines.ts fix). Flushed
                    // unconditionally, even when the metadata below fails
                    // to parse — it's still a real boundary in the
                    // underlying conversation.
                    parser.flushPending();
                    pushReleasedJekts();
                    const compactBoundary = parseCompactBoundaryFrame(rawEvent);
                    if (compactBoundary) {
                        compactionSummaries.noteBoundary(compactBoundary);
                        const paneEvents = model.dispatchPane({
                            type: "CompactionBoundary",
                            trigger: compactBoundary.trigger,
                            preTokens: compactBoundary.preTokens,
                            postTokens: compactBoundary.postTokens,
                            durationMs: compactBoundary.durationMs,
                            at: Date.now(),
                            frameTimestamp: compactBoundary.frameTimestamp,
                            boundaryUuid: compactBoundary.uuid,
                        });
                        {
                            const card = pushContextCompactedNodes(paneEvents, queue, hasNodeId, addNodeId);
                            if (card) awaitingCompactionSize = card;
                        }
                        // Fire-and-forget: trigger() handles its own
                        // fetch/send failures internally (never throws) and
                        // its own re-entrancy guard, so nothing here needs
                        // to await or catch. See
                        // SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_
                        // 2026_09_22.md §1.2/§3.3.
                        void memoryReinjectionController.trigger(
                            compactBoundary.frameTimestamp,
                            "compaction",
                            compactBoundary.uuid,
                        );
                    }
                    continue;
                }

                // AgentMux's own resume-outcome marker (not a provider frame —
                // see docs/specs/SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md
                // §2). Intercepted the same way as `compact_boundary` just
                // above: no `StreamEvent` shape in the translator, shared
                // parsing with `parseHistoryLines.ts` via `session-outcome.ts`
                // so the two can't drift. No `dispatchPane` round-trip needed —
                // unlike compaction, this has no live token-meter side effect,
                // it's purely a transcript marker — so the node is pushed
                // directly.
                if (rawEvent.type === "system" && rawEvent.subtype === "agentmux_session_outcome") {
                    parser.flushPending();
                    pushReleasedJekts();
                    const sessionOutcome = parseSessionOutcomeFrame(rawEvent);
                    // `resumed` is demoted out of the working transcript —
                    // same rule and rationale as parseHistoryLines.ts
                    // (SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_
                    // HISTORY_VIEW_2026_08_09.md §3.5). A `fresh` node that
                    // does get pushed ends the old session's in-progress rows
                    // inside the same StreamFlush that lands it (reducer.ts,
                    // StreamFlush handler); the rows themselves stay.
                    if (sessionOutcome && sessionOutcome.outcome !== "resumed") {
                        const node: SessionOutcomeNode = {
                            type: "session_outcome",
                            id: sessionOutcomeNodeId(sessionOutcome),
                            outcome: sessionOutcome.outcome,
                            attemptedSid: sessionOutcome.attemptedSid,
                            actualSid: sessionOutcome.actualSid,
                            continued: sessionOutcome.continued,
                            timestamp: sessionOutcomeLiveTimestamp(sessionOutcome.frameTimestamp),
                        };
                        if (!hasNodeId(node.id)) {
                            addNodeId(node.id);
                            queue.pushNewNode(node);
                            queue.scheduleFlush();
                            // A "fresh" outcome means AgentMux could not resume
                            // this PERSISTENT identity's prior session — the
                            // model has none of its previous conversation at
                            // all, the same "just lost prior context"
                            // situation compact_boundary's reinjection exists
                            // for (arguably more total loss than compaction,
                            // which at least leaves a summary). The identity
                            // (Global/Personal memory) is unchanged by the
                            // session swap, so it still needs to be back in
                            // front of the model. Fire the same hidden-
                            // reinjection turn here too — the controller's own
                            // busy-pane defer (fix 1/2 above) handles this
                            // landing while the triggering turn (the message
                            // that discovered the resume failure) is itself
                            // still in flight, which is the common case here.
                            // See SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_
                            // COMPACTION_2026_09_22.md §3.3a, "fresh session"
                            // addendum.
                            if (sessionOutcome.outcome === "fresh") {
                                // The meter's reading measured the conversation
                                // that is now gone.
                                model.dispatchPane({ type: "ContextInvalidated", reason: "fresh_session" });
                                awaitingCompactionSize = null;
                                void memoryReinjectionController.trigger(sessionOutcome.frameTimestamp, "fresh_session");
                            }
                        }
                    }
                    continue;
                }

                // Claude Code's compaction summary: a card, not a user message
                // (context-delivery.ts, shared with parseHistoryLines.ts).
                // SPEC_CONTEXT_DELIVERY_2026_09_30.md §3.3.
                {
                    const summaryNode = compactionSummaries.take(rawEvent, Date.now());
                    if (summaryNode) {
                        parser.flushPending();
                        pushReleasedJekts();
                        if (!hasNodeId(summaryNode.id)) {
                            addNodeId(summaryNode.id);
                            queue.pushNewNode(summaryNode);
                            queue.scheduleFlush();
                        }
                        continue;
                    }
                }

                // CLI install / version-change notices (cli-notice.ts,
                // shared with parseHistoryLines.ts). An install's later frame
                // carries the same id, so it updates the "installing" row.
                {
                    const cliNode = parseCliNoticeFrame(rawEvent, Date.now());
                    if (cliNode) {
                        parser.flushPending();
                        pushReleasedJekts();
                        if (hasNodeId(cliNode.id)) {
                            queue.pushUpdatedNode(cliNode);
                        } else {
                            addNodeId(cliNode.id);
                            queue.pushNewNode(cliNode);
                        }
                        queue.scheduleFlush();
                        continue;
                    }
                }

                // The notice for memory the `SessionStart` hook delivered
                // (memory-injected.ts, shared with parseHistoryLines.ts). The
                // model already has the content — this is only the label.
                // SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2.
                if (isMemoryInjectedFrame(rawEvent)) {
                    // A fallback re-delivery's own card arrives while its
                    // hidden turn is in flight; the controller shows the same
                    // card (same id) when the turn ends, so skip it here
                    // rather than mark the id seen behind the hiding queue.
                    if ((rawEvent as { fallback?: unknown }).fallback === true && memoryReinjectionController.isHiding()) {
                        continue;
                    }
                    parser.flushPending();
                    pushReleasedJekts();
                    const node = buildMemoryInjectedNode(rawEvent, {
                        contextWindow: currentContextWindow(),
                        now: Date.now(),
                    });
                    if (node && !hasNodeId(node.id)) {
                        addNodeId(node.id);
                        queue.pushNewNode(node);
                        queue.scheduleFlush();
                    }
                    continue;
                }

                // Extract live token counts from Anthropic stream events before
                // the translator discards them. message_start carries input_tokens
                // for this turn; message_delta carries the running output_tokens.
                {
                    // Whether the main agent's model has ended its turn, for the
                    // busy predicate (`modelEndedTurn` on the Streaming phase).
                    const turnCommand = modelTurnCommand(rawEvent);
                    if (turnCommand) model.dispatchPane(turnCommand);
                    // MAIN-agent usage only: a subagent's lines carry a
                    // parent_tool_use_id and have their own context and model
                    // (main-agent-usage.ts).
                    const usage = readsUsage ? mainAgentUsage(rawEvent) : null;
                    if (readsUsage) {
                        pendingStreamedChars += mainAgentStreamedChars(rawEvent);
                        if (mainAgentRequestStarted(rawEvent)) {
                            flushStreamedChars();
                            model.dispatchPane({ type: "RequestStarted" });
                        }
                    }
                    // A call already counted from its message_start (its
                    // assistant frames repeat the same usage).
                    const sameCall = usage?.kind === "in" && usage.messageId != null && usage.messageId === lastUsageMessageId;
                    if (usage?.kind === "in" && !sameCall) {
                        flushStreamedChars();
                        lastUsageMessageId = usage.messageId;
                        if (awaitingCompactionSize) {
                            // Through the raw queue: the first call after a
                            // compaction is often the hidden memory
                            // re-injection turn's, whose own nodes the hiding
                            // queue drops; the card was shown before it.
                            fillCompactionCard(awaitingCompactionSize, usage.input, rawQueue);
                            awaitingCompactionSize = null;
                        }
                        // message.model is the resolved model id (e.g.
                        // "claude-opus-4-8") — the reading is measured on it
                        // and its window resolved for it (context-reading.ts).
                        // Also feeds memoryReinjectionController's own
                        // contextWindow lookup (§3.4.2) — kept as a
                        // simple last-seen value rather than threaded
                        // through TokensIn's dispatch, since the
                        // controller needs it read synchronously at
                        // trigger() time, not as reactive pane state.
                        if (usage.model) lastSeenModelId = usage.model;
                        // input_tokens is only the uncached prompt; the cache
                        // split is kept (not just the sum) so downstream state
                        // can tell a cheap cache-served turn from an expensive
                        // fresh one — see TurnTokens' doc comment in ../types.ts.
                        const paneEvents = model.dispatchPane({
                            type: "TokensIn",
                            input: usage.input,
                            model: usage.model,
                            freshInput: usage.freshInput,
                            cacheCreation: usage.cacheCreation,
                            cacheRead: usage.cacheRead,
                        });
                        // Detect context compaction from the reducer's event output.
                        // Primary signal for Claude is the real CompactionBoundary
                        // path above; this heuristic (≥50% token drop from a >10k
                        // baseline) is suppressed by the reducer itself shortly
                        // after a real boundary landed, and remains the ONLY signal
                        // for providers with no structured event (codex/gemini/copilot).
                        pushContextCompactedNodes(paneEvents, queue, hasNodeId, addNodeId);
                    } else if (usage?.kind === "out") {
                        flushStreamedChars();
                        model.dispatchPane({ type: "TokensOut", output: usage.output });
                    }
                    // The windows Claude Code reports for the models this turn
                    // used — authoritative over the model-name table. Read off
                    // the raw frame: the translator keeps only the usage.
                    const windows = readsUsage ? reportedContextWindowsFromResult(rawEvent) : null;
                    if (windows) model.dispatchPane({ type: "ContextWindowsReported", windows });
                }

                // Translate provider-specific format → StreamEvent[]
                const streamEvents = translator.translate(rawEvent);

                // Convert StreamEvents → DocumentNodes
                for (const event of streamEvents) {
                    // Handle session_end: store stats, clear loading state,
                    // and flush the parser's text/thinking accumulators so the
                    // NEXT turn creates fresh nodes instead of appending to the
                    // previous response (which sits above the user's message).
                    if (event.type === "session_end") {
                        // Checked BEFORE queue.flushNow() below, and against
                        // the WRAPPED `queue` (not a separate "real" queue
                        // reference) deliberately: onSessionEnd() flips the
                        // controller's hiding flag to false as its own side
                        // effect before returning, so by the time
                        // pushNewNode is called here the wrapper's own
                        // isHiding() check already reads false and lets it
                        // through — no special-casing needed at this call
                        // site beyond "clear hiding, then push", in that
                        // order. Returns null (a no-op) for every ordinary
                        // turn's session_end, which is the overwhelming
                        // majority of calls here. See
                        // memory-reinjection-controller.ts's module doc
                        // comment and SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_
                        // COMPACTION_2026_09_22.md §3.3.
                        const reinjectionNode = memoryReinjectionController.onSessionEnd();
                        if (reinjectionNode && !hasNodeId(reinjectionNode.id)) {
                            addNodeId(reinjectionNode.id);
                            queue.pushNewNode(reinjectionNode);
                        }
                        // Land the turn's own trailing document nodes BEFORE
                        // TurnEnd settles the phase. Without this, the tail
                        // sits in the RAF queue while finalizeTurn dispatches
                        // TurnEnd synchronously; the flush then arrives a
                        // frame later in Done.completed, where the reducer's
                        // StreamFlushObserved re-promotion reads it as a new
                        // round — a permanent false "Working…" after any turn
                        // whose final text shares the last stream batch with
                        // its session_end (log-confirmed live: TurnEnd at t,
                        // Done → Streaming at t+6ms, then nothing). Genuine
                        // multi-round continuations still re-promote — their
                        // flushes arrive after this point.
                        // The turn ended: a jekt still held for its last block goes out now.
                        parser.releaseHeld();
                        pushReleasedJekts();
                        queue.flushNow();
                        flushStreamedChars();
                        finalizeTurn(event.stats ?? null);
                        // AFTER finalizeTurn — turnPhase is now genuinely
                        // Done for whatever turn just ended (real or
                        // hidden). Safe point to fire a reinjection that
                        // was deferred because THIS turn was still in
                        // flight when compact_boundary landed (§ "fix 1",
                        // memory-reinjection-controller.ts) — a no-op if
                        // nothing is deferred.
                        memoryReinjectionController.maybeFireDeferred();
                        continue;
                    }
                    // Provider is rate-limited and retrying. Keep lastEventMs
                    // live (suppresses false "stream-stuck" watchdog) and surface
                    // "Rate limited…" in the working row instead of a thinking phrase.
                    if (event.type === "provider_waiting") {
                        model.dispatchPane({
                            type: "ProviderWaiting",
                            reason: event.reason,
                            retryAfterMs: event.retryAfterMs,
                            at: Date.now(),
                        });
                        continue;
                    }
                    // Track the currently-running tool for the status line.
                    // Per-tool subscription open/close was removed — a single
                    // per-block subscription installed on mount above handles
                    // every tool's chunks (the wrapper publishes on a fixed
                    // event name with the tool_use_id in the payload), and
                    // the broker's replay-on-subscribe covers the late-
                    // subscribe race that the per-tool model lost.
                    if (event.type === "tool_call") {
                        // Files this agent writes, for the Files pane's
                        // "touched by" badges (touched-files.ts).
                        noteToolCall(
                            { blockId, agentName, color: agentColorOf(blockId) },
                            { id: event.id, tool: event.tool, params: event.params }
                        );
                        if (event.tool) {
                            model.dispatchPane({
                                type: "ToolStart",
                                name: event.tool,
                                arg: toolActivityArg(event.tool, event.params),
                            });
                        } else {
                            model.dispatchPane({ type: "ToolEnd" });
                        }
                    } else if (event.type === "tool_result") {
                        noteToolResult(event.id, event.status);
                        model.dispatchPane({ type: "ToolEnd" });
                    } else if (event.type === "tool_chunk") {
                        // Live-log streaming (SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md):
                        // route chunks through their own reducer command
                        // instead of forcing the full node list through
                        // StreamFlush. Skip the per-event parseLine →
                        // node → pendingNew path; the reducer mutates
                        // one ToolNode in place.
                        const { toolId, chunk } = parser.parseToolChunkEvent(event);
                        queue.pushToolChunk(toolId, chunk);
                        queue.scheduleFlush();
                        continue;
                    }
                    const node = parser.parseLine(JSON.stringify(event));
                    pushReleasedJekts(); // before `node`: they arrived before the event that released them
                    if (!node) continue;

                    // Stamp a receive time on nodes that don't carry their own
                    // timestamp (markdown, tool, section).
                    // user_message and agent_message already have timestamps.
                    if (!("timestamp" in node) || (node as any).timestamp == null) {
                        (node as any).timestamp = Date.now();
                    }

                    if (hasNodeId(node.id)) {
                        queue.pushUpdatedNode(node);
                    } else {
                        addNodeId(node.id);
                        queue.pushNewNode(node);
                    }
                }
            }
            parser.setSourceLine(null);
            flushStreamedChars();

            // Schedule a single flush per animation frame
            if (queue.hasPendingNewOrUpdated()) {
                queue.scheduleFlush();
            }
        };

        // Places each transcript record by its line (Phase 5a-4,
        // transcript-cursor.ts): duplicates dropped, gaps read, and nothing
        // parsed before the history it follows. An echo — the user message
        // the controller wrote to the agent's stdin, whose node the pane
        // already shows — is not parsed; the ledger pairs it with that node.
        const cursor = new TranscriptCursor({
            deliver: parseRecords,
            echo: (text) => echoLedger.echoed(text),
            isOwnEcho: (line) => echoLedger.isOwnEcho(line),
            reset: resetStream,
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

        // Live records wait for the history load (the pane is covered until
        // then). A load that never reports doesn't hold them for good: after
        // HISTORY_HOLD_MAX_MS they are placed from the first event on.
        const stopWaiting = transcriptSettle
            ? transcriptSettle.onSettle((outcome) => cursor.settle(outcome))
            : (cursor.settle(null), () => {});
        const holdTimer = setTimeout(() => {
            if (cursor.isSettled()) return;
            console.warn(`[useAgentStream] no history outcome after ${HISTORY_HOLD_MAX_MS}ms; placing live records from the next one`);
            cursor.settle(null);
        }, HISTORY_HOLD_MAX_MS);

        // Lines no event brings: another block or srv instance appending to
        // the agent's shared zone. Filled one poll late, so lines this pane's
        // own events are still bringing (and its own echoes) arrive by event
        // first.
        let lastCount: { count: number; stream: string; gen: string } | null = null;
        let polling = false;
        const pollTimer = setInterval(async () => {
            const pin = cursor.position();
            if (!pin || !pin.stream.startsWith("g:") || polling || document.hidden) return;
            polling = true;
            try {
                const resp = await RpcApi.BlockfileLineCountCommand(
                    TabRpcClient,
                    { block_id: blockId, filename: OutputFileName },
                    { timeout: 5_000 },
                );
                if (!resp?.stream || !resp.gen) return;
                if (lastCount && lastCount.stream === resp.stream && lastCount.gen === resp.gen) {
                    cursor.observeCount(lastCount.count, resp.stream, resp.gen);
                }
                lastCount = { count: resp.count, stream: resp.stream, gen: resp.gen };
            } catch {
                // Soft: the next tick or event catches up.
            } finally {
                polling = false;
            }
        }, SHARED_ZONE_POLL_MS);

        onCleanup(() => {
            clearTimeout(holdTimer);
            clearInterval(pollTimer);
            stopWaiting();
            cursor.dispose();
            unregisterCursorStats();
            queue.cancelScheduledFlush();
            subscription.unsubscribe();
            // (the tool_chunk subscription is torn down by its own body-scope
            // onCleanup registered where useToolChunkStream is called — so it
            // is cleaned up even when this onMount early-returns.)
            // StreamUnsubscribe transitions a working turn into the
            // Disconnected phase (so a crash or exit without
            // session_end doesn't leave "Working…" stuck).
            const at = Date.now();
            model.dispatchPane({ type: "StreamUnsubscribe", at });
            // Defer SessionEnd to a microtask so it fires AFTER the synchronous
            // disposal chain completes. During error-boundary cleanup the <Key>
            // streaming-buffer scope is still partially live while onCleanup runs;
            // a synchronous document-nodes publish here re-triggers reconcileArrays
            // on a half-torn-down DOM → replaceChild NotFoundError (observed
            // 2026-06-06 crash 2, confirmed in
            // SPEC_REPLACECHILD_CRASH_FULL_ANALYSIS_AND_FIX_2026-06-06.md §3.1).
            // By microtask time all scope disposal is complete and the <Key>
            // effect is removed from the computation graph. model.dispatchDoc
            // uses the soft dispatchIfRegistered variant, so a gone slot is a
            // silent no-op rather than a throw.
            queueMicrotask(() => model.dispatchDoc({ type: "SessionEnd", at }));
        });
    });

    return backgroundTasksAtom;
}
