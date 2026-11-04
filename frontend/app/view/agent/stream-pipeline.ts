// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The NDJSON line pipeline of `useAgentStream` (byte-decoded text → line buffer → JSON → frame
 * intercepts → token extraction → translator → parser → queued nodes), one function per stage. The
 * state that lives across calls is one `StreamState`, which the hook resets on every subscription;
 * `StreamDeps` is what the pipeline reads from its hook and does not own.
 */

import { getObjectValue, makeORef } from "@/app/store/mos";
import { noteToolCall, noteToolResult } from "@/app/store/touched-files";
import { blockRoleColor, isLightThemeActive } from "@/app/block/pane-identity";
import { createTranslator } from "./providers/translator-factory";
import { modelTurnCommand } from "./model-turn-signal";
import {
    mainAgentRequestStarted,
    mainAgentStreamedChars,
    mainAgentStreamedKind,
    mainAgentUsage,
    type StreamedKind,
} from "./main-agent-usage";
import type { ContextReader } from "./context-reader";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { ContextCompactedNode, DocumentNode, SessionOutcomeNode } from "../../agent-model/types";
import { noteTaskFrame } from "../../agent-model/task-outcomes";
import { compactionModelKey, parseCompactionSample, recordCompactionSample } from "./compaction-estimate";
import { CompactionSummaryTracker } from "./context-delivery";
import { sessionOutcomeNodeId, sessionOutcomeLiveTimestamp } from "@/app/store/agent-document/session-outcome";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import type { StreamFlushQueue } from "./stream-flush-queue";
import { restoreTitleOnContinue } from "./hooks/useBlockActivity";
import type { createStreamMemoryReinjection } from "./stream-memory-reinjection";
import { buildMemoryInjectedNode, supersededCard } from "./memory-injected";
import { interceptFrame, type FrameSink } from "./frame-intercepts";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import { reportedContextWindowsFromResult } from "@/app/store/agent-pane-state/context-reading";
import { toolActivityArg } from "../../agent-model/tool-descriptors";
import type { useTurnLifecycle } from "./hooks/useTurnLifecycle";
import { fillCompactionCard, pushContextCompactedNodes } from "./compaction-cards";

type StreamEvent = ReturnType<ReturnType<typeof createTranslator>["translate"]>[number];

/** What the pipeline keeps between calls. The hook replaces the parser, translator and sets on a new subscription. */
export interface StreamState {
    lineBuffer: string;
    translator: ReturnType<typeof createTranslator>;
    /** Pairs each compact_boundary with Claude Code's summary frame, which becomes a card instead of a user message. */
    compactionSummaries: CompactionSummaryTracker;
    parser: ClaudeCodeStreamParser;
    /** In-batch dedup cache (the reducer also dedups; this avoids enqueuing already-seen nodes). */
    nodeIdSet: Set<string>;
    lastSeenModelId: string | undefined;
    /** The API message the last TokensIn was for: one call is reported by its message_start and again by its assistant frame(s), and counts once. */
    lastUsageMessageId: string | undefined;
    /** A real compaction card still waiting for the context's size after it. */
    awaitingCompactionSize: ContextCompactedNode | null;
    /** A provider's own context reports, other than Claude Code's per-call usage; reset with the translator. */
    contextReader: ContextReader | null;
    /** Output characters streamed since the last dispatch, for the working row's counter. */
    pendingStreamedChars: number;
    /** The batch's latest kind of output, for the live status's phase. */
    pendingStreamedKind: StreamedKind | undefined;
}

export interface StreamDeps {
    blockId: string;
    agentName: string | undefined;
    model: AgentPaneModel;
    documentNodes: () => DocumentNode[];
    /** The queue every producer pushes into (the hiding wrapper), and the one under it. */
    queue: StreamFlushQueue;
    rawQueue: StreamFlushQueue;
    memory: ReturnType<typeof createStreamMemoryReinjection>;
    currentContextWindow: () => number;
    /** Fed every line in stream order: true for a new CLI session's `init`. */
    isNewSession: (raw: unknown) => boolean;
    readThinkingHeadline: (raw: unknown) => string | null | undefined;
    /** Only Claude Code's stream carries per-call usage in the shape the meter reads. */
    readsUsage: boolean;
    /** Bound once the hook has built its turn lifecycle. */
    finalizeTurn: ReturnType<typeof useTurnLifecycle>["finalizeTurn"];
}

export interface StreamPipeline {
    st: StreamState;
    deps: StreamDeps;
    sink: FrameSink;
}

export function createStreamState(outputFormat: string, agentName: string | undefined): StreamState {
    const parser = new ClaudeCodeStreamParser();
    if (agentName) parser.setAgentId(agentName);
    return {
        lineBuffer: "",
        translator: createTranslator(outputFormat),
        compactionSummaries: new CompactionSummaryTracker(),
        parser,
        nodeIdSet: new Set(),
        lastSeenModelId: undefined,
        lastUsageMessageId: undefined,
        awaitingCompactionSize: null,
        contextReader: null,
        pendingStreamedChars: 0,
        pendingStreamedKind: undefined,
    };
}

export function createStreamPipeline(st: StreamState, deps: StreamDeps): StreamPipeline {
    const pipeline = { st, deps } as StreamPipeline;
    pipeline.sink = createFrameSink(pipeline);
    return pipeline;
}

export const hasNodeId = (st: StreamState, id: string): boolean => st.nodeIdSet.has(id);
export const addNodeId = (st: StreamState, id: string): void => {
    st.nodeIdSet.add(id);
};

/** The colour an agent pane is drawn in (its focused border), if it has one. */
function agentColorOf(blockId: string): string | undefined {
    return blockRoleColor(getObjectValue<Block>(makeORef("block", blockId))?.meta, isLightThemeActive(), "identity");
}

/**
 * Jekts the parser held while a text/thinking block streamed, released now that it ended: pushed as
 * new nodes, BEFORE whatever node the releasing event produced (SPEC_JEKT_IMMEDIATE_DELIVERY_2026_09_28.md §2.2).
 */
export function pushReleasedJekts(st: StreamState, queue: StreamFlushQueue): void {
    for (const released of st.parser.drainReleased()) {
        if ((released as { timestamp?: number }).timestamp == null) {
            (released as { timestamp?: number }).timestamp = Date.now();
        }
        if (hasNodeId(st, released.id)) continue;
        addNodeId(st, released.id);
        queue.pushNewNode(released);
    }
}

/** One OutputStreamed per batch of lines, not per delta; flushed before any call boundary so a call's characters are never credited to the next one. */
function flushStreamedChars({ st, deps }: StreamPipeline): void {
    if (st.pendingStreamedChars <= 0) return;
    deps.model.dispatchPane({ type: "OutputStreamed", chars: st.pendingStreamedChars, kind: st.pendingStreamedKind });
    st.pendingStreamedChars = 0;
    st.pendingStreamedKind = undefined;
}

/**
 * A reset of the stream. `truncate` is today's: the reducer decides whether to honor it (late
 * truncates after a socket-reconnect race are suppressed), and the parse state is reset only when it
 * is. `replace` and `delete` (restore, archive) only move the cursor, and the pane keeps what it shows.
 */
export function resetStream({ st, deps }: StreamPipeline, fileop: "truncate" | "replace" | "delete"): void {
    if (fileop !== "truncate") {
        // Archive and restore swap the conversation under the pane: the meter's reading measured the
        // old one. srv sends no `fresh` outcome for an archive, so this is the only signal. Without
        // it, the next call's smaller reading would also look like a compaction to the TokensIn heuristic.
        deps.model.dispatchPane({ type: "ContextInvalidated", reason: "transcript_replaced" });
        st.awaitingCompactionSize = null;
        return;
    }
    const events = deps.model.dispatchDoc({
        type: "StreamTruncate",
        reason: "fileop",
    });
    const honored = events.some((e) => e.type === "truncate-applied");
    if (!honored) return;
    deps.queue.resetAll();
    st.lineBuffer = "";
    st.translator.reset();
    st.parser.reset();
    st.nodeIdSet = new Set();
    // The reducer clears sessionStats/currentTool/turnTokens and moves turnPhase to Idle in one shot.
    deps.model.dispatchPane({ type: "TurnReset" });
}

function createFrameSink(p: StreamPipeline): FrameSink {
    const { st, deps } = p;
    return {
        node: (node, update) => {
            if (hasNodeId(st, node.id)) {
                if (!update) return;
                deps.queue.pushUpdatedNode(node);
            } else {
                addNodeId(st, node.id);
                deps.queue.pushNewNode(node);
            }
            deps.queue.scheduleFlush();
        },
        placeReleased: () => pushReleasedJekts(st, deps.queue),
        compactBoundary: (compactBoundary, raw) => onCompactBoundary(p, compactBoundary, raw),
        sessionOutcome: (sessionOutcome) => onSessionOutcome(p, sessionOutcome),
        memoryInjected: (raw) => onMemoryInjected(p, raw),
    };
}

function onCompactBoundary({ st, deps }: StreamPipeline, compactBoundary: Parameters<FrameSink["compactBoundary"]>[0], raw: Parameters<FrameSink["compactBoundary"]>[1]): void {
    // Feeds only the compaction-time ESTIMATE (the working row's progress bar) with its own parser; it
    // reads the frame's real stdout shape (SPEC_COMPACTION_ESTIMATED_PROGRESS_AND_STREAM_FRAMES_2026_10_01.md §3/§5).
    recordCompactionSample(
        parseCompactionSample(raw, compactionModelKey(getObjectValue<Block>(makeORef("block", deps.blockId))?.meta, st.lastSeenModelId))
    );
    if (!compactBoundary) return;
    const paneEvents = deps.model.dispatchPane({
        type: "CompactionBoundary",
        trigger: compactBoundary.trigger,
        preTokens: compactBoundary.preTokens,
        postTokens: compactBoundary.postTokens,
        durationMs: compactBoundary.durationMs,
        at: Date.now(),
        frameTimestamp: compactBoundary.frameTimestamp,
        boundaryUuid: compactBoundary.uuid,
    });
    const card = pushContextCompactedNodes(paneEvents, deps.queue, (id) => hasNodeId(st, id), (id) => addNodeId(st, id));
    if (card) st.awaitingCompactionSize = card;
    // Fire-and-forget: trigger() handles its own fetch/send failures and re-entrancy
    // (SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md §1.2/§3.3). A manual compaction
    // is the user asking the agent to compact: once its own turn ends, the agent says where it is
    // picking up (a catch-up turn). An automatic one lands mid-turn and the agent's reply to that turn
    // already follows, so only the memory fallback applies.
    if (compactBoundary.trigger === "manual") {
        void deps.memory.catchUp(compactBoundary.frameTimestamp, "compaction", {
            boundaryUuid: compactBoundary.uuid,
            deferIfBusy: true,
        });
    } else {
        void deps.memory.trigger(compactBoundary.frameTimestamp, "compaction", compactBoundary.uuid);
    }
}

function onSessionOutcome({ st, deps }: StreamPipeline, sessionOutcome: Parameters<FrameSink["sessionOutcome"]>[0]): void {
    // `resumed` is demoted out of the working transcript, as in replay
    // (SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md §3.5). A `fresh`
    // node ends the old session's in-progress rows inside the StreamFlush that lands it (reducer.ts).
    if (!sessionOutcome || sessionOutcome.outcome === "resumed") return;
    const node: SessionOutcomeNode = {
        type: "session_outcome",
        id: sessionOutcomeNodeId(sessionOutcome),
        outcome: sessionOutcome.outcome,
        attemptedSid: sessionOutcome.attemptedSid,
        actualSid: sessionOutcome.actualSid,
        continued: sessionOutcome.continued,
        timestamp: sessionOutcomeLiveTimestamp(sessionOutcome.frameTimestamp),
    };
    if (hasNodeId(st, node.id)) return;
    addNodeId(st, node.id);
    deps.queue.pushNewNode(node);
    deps.queue.scheduleFlush();
    // A "fresh" outcome: the model has none of this identity's previous conversation, the loss
    // compaction's reinjection exists for, so the identity goes back in front of it too
    // (SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md §3.3a).
    if (sessionOutcome.outcome !== "fresh") return;
    // A session that carries the record is still about the same goal.
    if (sessionOutcome.continued) restoreTitleOnContinue(deps.blockId);
    // The meter's reading measured the conversation that is now gone.
    deps.model.dispatchPane({ type: "ContextInvalidated", reason: "fresh_session" });
    st.awaitingCompactionSize = null;
    // With the pane quiet nobody is talking to the agent yet (an account switch, a resume the CLI
    // rejected): the agent catches up and tells the user where it is picking up. With a turn in
    // flight the user already is, so only the memory fallback applies. Whether a message started the
    // session is srv's to say (`idleStart`): this pane's own turn state can be stale, left by the very
    // turn an account switch interrupted. An older srv says nothing, so the pane's own state decides then.
    const idleStart = sessionOutcome.idleStart ?? !workingFromPhase(paneSnapshot(deps.blockId)?.turnPhase ?? { kind: "Idle" });
    if (idleStart) {
        void deps.memory.catchUp(sessionOutcome.frameTimestamp, "fresh_session", {
            deferIfBusy: sessionOutcome.idleStart === true,
        });
    } else {
        void deps.memory.trigger(sessionOutcome.frameTimestamp, "fresh_session");
    }
}

function onMemoryInjected({ st, deps }: StreamPipeline, raw: Record<string, unknown>): void {
    // A fallback re-delivery's own card arrives while its hidden turn is in flight; the controller
    // shows the same card (same id) when the turn ends, so skip it here rather than mark the id seen
    // behind the hiding queue.
    if ((raw as { fallback?: unknown }).fallback === true && deps.memory.isHiding()) return;
    st.parser.flushPending();
    pushReleasedJekts(st, deps.queue);
    const node = buildMemoryInjectedNode(raw, { contextWindow: deps.currentContextWindow(), now: Date.now() });
    if (node && !hasNodeId(st, node.id)) {
        addNodeId(st, node.id);
        // A card queued for the next flush counts too: the reducer applies a flush's new nodes before its updates.
        const replaced = supersededCard([...deps.documentNodes(), ...deps.queue.pendingNewNodes()], node);
        if (replaced) deps.queue.pushUpdatedNode({ ...node, id: replaced.id });
        else deps.queue.pushNewNode(node);
        deps.queue.scheduleFlush();
    }
}

/**
 * Extract live token counts from Anthropic stream events before the translator discards them:
 * message_start carries input_tokens for this turn, message_delta the running output_tokens. MAIN
 * agent usage only: a subagent's lines carry a parent_tool_use_id and have their own context and
 * model (main-agent-usage.ts).
 */
function observeUsage(p: StreamPipeline, rawEvent: any): void {
    const { st, deps } = p;
    // Whether the main agent's model has ended its turn, for the busy predicate (`modelEndedTurn`).
    const turnCommand = modelTurnCommand(rawEvent);
    if (turnCommand) deps.model.dispatchPane(turnCommand);
    const usage = deps.readsUsage ? mainAgentUsage(rawEvent) : null;
    if (deps.readsUsage) {
        st.pendingStreamedChars += mainAgentStreamedChars(rawEvent);
        st.pendingStreamedKind = mainAgentStreamedKind(rawEvent) ?? st.pendingStreamedKind;
        if (mainAgentRequestStarted(rawEvent)) {
            flushStreamedChars(p);
            deps.model.dispatchPane({ type: "RequestStarted" });
        }
    }
    // A call already counted from its message_start (its assistant frames repeat the same usage).
    const sameCall = usage?.kind === "in" && usage.messageId != null && usage.messageId === st.lastUsageMessageId;
    if (usage?.kind === "in" && !sameCall) {
        flushStreamedChars(p);
        st.lastUsageMessageId = usage.messageId;
        if (st.awaitingCompactionSize) {
            // Through the raw queue: the first call after a compaction is often the hidden memory
            // re-injection turn's, whose own nodes the hiding queue drops; the card was shown before it.
            fillCompactionCard(st.awaitingCompactionSize, usage.input, deps.rawQueue);
            st.awaitingCompactionSize = null;
        }
        // message.model is the resolved model id; the reading is measured on it and its window
        // resolved for it. Kept as a last-seen value too, since the memory controller reads the
        // context window synchronously at trigger() time, not as reactive pane state.
        if (usage.model) st.lastSeenModelId = usage.model;
        // input_tokens is only the uncached prompt; the cache split is kept (not just the sum) so
        // downstream state can tell a cheap cache-served turn from an expensive fresh one.
        const paneEvents = deps.model.dispatchPane({
            type: "TokensIn",
            input: usage.input,
            model: usage.model,
            freshInput: usage.freshInput,
            cacheCreation: usage.cacheCreation,
            cacheRead: usage.cacheRead,
        });
        // Context compaction from the reducer's event output. The real CompactionBoundary path is the
        // primary signal for Claude; this heuristic (>=50% token drop from a >10k baseline) is suppressed
        // by the reducer shortly after a real boundary and is the ONLY signal for providers with no
        // structured event (codex/gemini/copilot).
        pushContextCompactedNodes(paneEvents, deps.queue, (id) => hasNodeId(st, id), (id) => addNodeId(st, id));
    } else if (usage?.kind === "out") {
        flushStreamedChars(p);
        deps.model.dispatchPane({ type: "TokensOut", output: usage.output });
    }
    // The windows Claude Code reports for the models this turn used, authoritative over the
    // model-name table. Read off the raw frame: the translator keeps only the usage.
    const windows = deps.readsUsage ? reportedContextWindowsFromResult(rawEvent) : null;
    if (windows) deps.model.dispatchPane({ type: "ContextWindowsReported", windows });
    const reported = st.contextReader?.observe(rawEvent);
    if (reported) {
        deps.model.dispatchPane({
            type: "ContextReported",
            tokens: reported.tokens,
            model: reported.model ?? undefined,
            window: reported.window ?? undefined,
        });
    }
}

/**
 * The turn is over: store stats, clear the loading state, and flush the parser's text/thinking
 * accumulators so the NEXT turn creates fresh nodes instead of appending to the previous response.
 */
function handleSessionEnd(p: StreamPipeline, event: Extract<StreamEvent, { type: "session_end" }>): void {
    const { st, deps } = p;
    // Checked BEFORE queue.flushNow(), against the WRAPPED `queue`: onSessionEnd() flips the
    // controller's hiding flag to false as its own side effect before returning, so by the time
    // pushNewNode is called the wrapper already lets the node through. Returns null (a no-op) for
    // every ordinary turn's session_end (SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md §3.3).
    const reinjectionNode = deps.memory.onSessionEnd();
    if (reinjectionNode && !hasNodeId(st, reinjectionNode.id)) {
        addNodeId(st, reinjectionNode.id);
        deps.queue.pushNewNode(reinjectionNode);
    }
    // Land the turn's own trailing document nodes BEFORE TurnEnd settles the phase. Otherwise the
    // tail sits in the RAF queue while finalizeTurn dispatches TurnEnd synchronously; the flush then
    // arrives a frame later in Done.completed, where StreamFlushObserved re-promotes it as a new
    // round: a permanent false "Working…" after any turn whose final text shares the last stream
    // batch with its session_end. Genuine multi-round continuations still re-promote, since their
    // flushes arrive after this point. A jekt still held for the turn's last block goes out now.
    st.parser.releaseHeld();
    pushReleasedJekts(st, deps.queue);
    deps.queue.flushNow();
    flushStreamedChars(p);
    deps.finalizeTurn(event.stats ?? null);
    // AFTER finalizeTurn: turnPhase is now genuinely Done for the turn that just ended (real or
    // hidden), the safe point to fire a reinjection deferred because THIS turn was still in flight
    // when compact_boundary landed. A no-op if nothing is deferred.
    deps.memory.maybeFireDeferred();
}

/** Track the running tool for the status line, route a tool chunk to its own reducer command, or turn the event into a queued node. */
function handleStreamEvent(p: StreamPipeline, event: StreamEvent, rawEvent: any): void {
    const { st, deps } = p;
    if (event.type === "session_end") return handleSessionEnd(p, event);
    // Provider is rate-limited and retrying. Keep lastEventMs live (suppresses a false "stream-stuck"
    // watchdog) and surface "Rate limited…" in the working row instead of a thinking phrase.
    if (event.type === "provider_waiting") {
        deps.model.dispatchPane({
            type: "ProviderWaiting",
            reason: event.reason,
            retryAfterMs: event.retryAfterMs,
            at: Date.now(),
        });
        return;
    }
    if (event.type === "tool_call") {
        // Files this agent writes, for the Files pane's "touched by" badges (touched-files.ts).
        noteToolCall(
            { blockId: deps.blockId, agentName: deps.agentName, color: agentColorOf(deps.blockId) },
            { id: event.id, tool: event.tool, params: event.params }
        );
        if (event.tool) {
            deps.model.dispatchPane({
                type: "ToolStart",
                name: event.tool,
                arg: toolActivityArg(event.tool, event.params),
                id: event.id,
                params: event.params,
                // A subagent's own call: shown under its Agent call.
                parentId: typeof rawEvent.parent_tool_use_id === "string" ? rawEvent.parent_tool_use_id : undefined,
            });
        } else {
            deps.model.dispatchPane({ type: "ToolEnd" });
        }
    } else if (event.type === "tool_result") {
        noteToolResult(event.id, event.status);
        deps.model.dispatchPane({ type: "ToolEnd", id: event.id });
    } else if (event.type === "tool_chunk") {
        // Live-log streaming (SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md): chunks go through their own
        // reducer command instead of the full node list through StreamFlush; the reducer mutates one
        // ToolNode in place.
        const { toolId, chunk } = st.parser.parseToolChunkEvent(event);
        deps.queue.pushToolChunk(toolId, chunk);
        deps.queue.scheduleFlush();
        return;
    }
    const node = st.parser.parseLine(JSON.stringify(event));
    pushReleasedJekts(st, deps.queue); // before `node`: they arrived before the event that released them
    if (!node) return;
    // Stamp a receive time on nodes that don't carry their own (markdown, tool, section).
    if (!("timestamp" in node) || (node as any).timestamp == null) {
        (node as any).timestamp = Date.now();
    }
    if (hasNodeId(st, node.id)) {
        deps.queue.pushUpdatedNode(node);
    } else {
        addNodeId(st, node.id);
        deps.queue.pushNewNode(node);
    }
}

/** Handle one JSON line: stderr, observers, intercepts, token extraction, then the translator's events. */
function processFrame(p: StreamPipeline, rawEvent: any): void {
    const { st, deps } = p;
    // Handle stderr events from subprocess
    if (rawEvent.type === "stderr" && rawEvent.text) {
        const text = rawEvent.text.trim();
        if (text.includes("Fast mode is not available") ||
            text.includes("[WARN]") && text.length < 200) {
            return;
        }
        deps.queue.pushNewNode({
            id: `stderr-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
            type: "markdown",
            content: `**stderr:** ${text}`,
            timestamp: Date.now(),
        });
        deps.queue.scheduleFlush();
        return;
    }
    // Observe only, the frame keeps flowing below. A background task's end is on this stream
    // (`system/task_notification`); the dock reads it from here so it never depends on srv's live
    // registry having watched the task (agent-model/task-outcomes.ts).
    if (rawEvent.type === "system") noteTaskFrame(deps.blockId, rawEvent, Date.now());
    // A new CLI session, not a compaction's own `init` (session-start.ts): a turn still holding live
    // tokens ended without a `result`, so its tokens go.
    if (deps.isNewSession(rawEvent)) deps.model.dispatchPane({ type: "StreamSessionStarted" });
    const headline = deps.readThinkingHeadline(rawEvent);
    if (headline) deps.model.dispatchPane({ type: "ThinkingHeadline", text: headline });
    // The intercepts history replay runs too, in the same order (frame-intercepts.ts): the real
    // compact_boundary frame, session outcomes, CLI notices and memory deliveries. What they find
    // goes to the sink.
    if (interceptFrame(rawEvent, st, p.sink, Date.now())) return;
    observeUsage(p, rawEvent);
    for (const event of st.translator.translate(rawEvent)) {
        handleStreamEvent(p, event, rawEvent);
    }
}

/**
 * Records the transcript cursor has placed, parsed as live input. `from` says where the text sits in
 * the transcript, so a tool node records its result's line (SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01.md §3.2).
 */
export function parseRecords(p: StreamPipeline, text: string, from?: { stream: string; gen: string; line: number }): void {
    const { st, deps } = p;
    // Line numbers only when this text starts a fresh line: a partial line held from before would shift every number.
    let sourceLine = from && st.lineBuffer === "" ? from.line : null;
    st.lineBuffer += text;
    const lines = st.lineBuffer.split("\n");
    st.lineBuffer = lines.pop() || ""; // Keep incomplete line
    // Drop the buffer if it grows absurdly large without a newline: protects against runaway memory
    // if something upstream is sending garbage.
    if (st.lineBuffer.length > 10_000_000) {
        console.warn(`[useAgentStream] line buffer exceeded 10MB, dropping`);
        st.lineBuffer = "";
    }

    for (const line of lines) {
        // Every element is one transcript line, blank or not.
        st.parser.setSourceLine(sourceLine != null && from ? { stream: from.stream, gen: from.gen, line: sourceLine } : null);
        // The id history gives this line on a reload (node_L<line>_<k>), so a collapsed row or a scroll anchor finds it again.
        st.parser.setIdLine(sourceLine != null && from ? sourceLine : null, from?.gen);
        if (sourceLine != null) sourceLine++;
        const trimmed = line.trim();
        if (!trimmed) continue;
        // Non-JSON lines (subprocess echoes, CLI warnings): skip without the cost of a try/catch on a huge string.
        if (!trimmed.startsWith("{")) continue;
        let rawEvent: any;
        try {
            rawEvent = JSON.parse(trimmed);
        } catch {
            // Don't log the full line: it may be 100KB+ (a Write tool call with a long file) and
            // forwarding it through the IPC log pipe stalls the main thread.
            console.warn(`[useAgentStream] JSON parse failed, len=${trimmed.length}`);
            continue;
        }
        processFrame(p, rawEvent);
    }
    st.parser.setSourceLine(null);
    st.parser.setIdLine(null);
    flushStreamedChars(p);

    // Schedule a single flush per animation frame
    if (deps.queue.hasPendingNewOrUpdated()) {
        deps.queue.scheduleFlush();
    }
}
