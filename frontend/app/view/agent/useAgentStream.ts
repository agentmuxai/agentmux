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
 * IMPORTANT — crash history: `tool_chunk` WPS events used to call
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

import { getFileSubject } from "@/app/store/wps";
import { base64ToArray } from "@/util/util";
import { onCleanup, onMount } from "solid-js";
import { createTranslator } from "./providers/translator-factory";
import type { PendingMessage, SignalPair } from "./state";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { ContextCompactedNode, DocumentNode, SessionOutcomeNode } from "./types";
import { parseCompactBoundaryFrame, contextCompactedNodeId, contextCompactedLiveTimestamp } from "./compact-boundary";
import { parseSessionOutcomeFrame, sessionOutcomeNodeId, sessionOutcomeLiveTimestamp } from "./session-outcome";
import type { AgentPaneEvent, TurnPhase } from "@/app/store/agent-pane-state/types";
import { getNodeIdSet } from "@/app/store/agent-document-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { createStreamFlushQueue, type StreamFlushQueue } from "./stream-flush-queue";
import { useToolChunkStream } from "./hooks/useToolChunkStream";
import { useShellNodeStream } from "./hooks/useShellNodeStream";
import { useCompactionStream } from "./hooks/useCompactionStream";
import { useTurnLifecycle } from "./hooks/useTurnLifecycle";
import { usePendingMessageAcceptance } from "./hooks/usePendingMessageAcceptance";

const OutputFileName = "output";

/** Extract the first significant argument from a tool's params for display.
 *  File path for read/write/edit; command string for bash; query for search.
 *  Returns undefined when no useful single-argument is present. */
function extractToolArg(tool: string, params: Record<string, unknown> | undefined): string | undefined {
    if (!params) return undefined;
    const p = params as Record<string, unknown>;
    switch (tool) {
        case "read": case "Read": case "read_file":
            return typeof p.file_path === "string" ? p.file_path : typeof p.path === "string" ? p.path : undefined;
        case "write": case "Write": case "write_file":
            return typeof p.file_path === "string" ? p.file_path : typeof p.path === "string" ? p.path : undefined;
        case "edit": case "Edit": case "str_replace_editor": case "multiedit":
            return typeof p.file_path === "string" ? p.file_path : typeof p.path === "string" ? p.path : undefined;
        case "bash": case "Bash": case "computer":
            return typeof p.command === "string" ? p.command : undefined;
        case "glob": case "Glob":
            return typeof p.pattern === "string" ? p.pattern : undefined;
        case "grep": case "Grep":
            return typeof p.pattern === "string" ? p.pattern : undefined;
        default:
            // Try common arg names in priority order.
            for (const k of ["file_path", "path", "command", "query", "pattern"]) {
                if (typeof p[k] === "string") return p[k] as string;
            }
            return undefined;
    }
}

/**
 * Shared node-construction for the reducer's `context-compacted` event —
 * used by BOTH the real `CompactionBoundary` path and the `TokensIn`
 * heuristic fallback, so the two never drift into different node shapes.
 * `source`/`trigger`/`durationMs` come straight from the event: "real"
 * carries all three, "heuristic" carries only `source`. See
 * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.3.
 */
function pushContextCompactedNodes(
    paneEvents: AgentPaneEvent[] | undefined,
    queue: StreamFlushQueue,
    hasNodeId: (id: string) => boolean,
    addNodeId: (id: string) => void,
): void {
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
        }
    }
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
    documentAtom: SignalPair<DocumentNode[]>;
    /**
     * The reducer's turn-phase signal — read by the hook to detect
     * "was this a user-initiated stop?" at session_end so the
     * "⏹ Interrupted by user" markdown row can be appended for
     * durable visual confirmation. Replaces the legacy
     * `turnActiveAtom` / `stoppingAtom` props dropped in PR G; the
     * predicate is `turnPhase.kind === "Interrupting"`.
     */
    turnPhaseAtom: SignalPair<TurnPhase>;
    /**
     * Pending queue shared with the composer's `sendMessage` path. On
     * `agent-message-accepted` events, the hook removes the matching
     * entry and promotes it to a `user_message` document node — this is
     * the visible "accepted" transition for the user.
     */
    pendingMessagesAtom?: SignalPair<PendingMessage[]>;
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
}

/**
 * Subscribe to subprocess output and parse it into styled DocumentNodes.
 */
export function useAgentStream({
    blockId,
    model,
    outputFormat,
    documentAtom,
    turnPhaseAtom,
    pendingMessagesAtom,
    enabled,
    provider,
    agentName,
    onTurnStartFromQueue,
}: UseAgentStreamOpts): void {
    // Mutable state that doesn't trigger re-renders. Kept here (not
    // extracted) because it's tightly coupled to the NDJSON parse loop
    // below: lineBuffer/translator/parser accumulate per-byte parse state
    // across fileSubject callbacks, and nodeIdSet is this hook's in-batch
    // dedup cache that the extracted producer hooks also need to consult
    // via the hasNodeId/addNodeId closures passed to them.
    let lineBuffer = "";
    let translator = createTranslator(outputFormat);
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
    const queue = createStreamFlushQueue(model);

    // Tool-chunk and persistent-shell streaming subscriptions, installed at
    // body scope (not inside onMount) so they tear down even if onMount
    // below early-returns (e.g. enabled:false). Both push into `queue`
    // rather than scheduling their own flush.
    useToolChunkStream({ blockId, queue });
    useShellNodeStream({ blockId, queue });
    useCompactionStream({ blockId, model, queue, hasNodeId, addNodeId });

    onMount(() => {
        if (!enabled || !blockId) return;

        // Reset state on new subscription
        lineBuffer = "";
        translator = createTranslator(outputFormat);
        parser = new ClaudeCodeStreamParser();
        if (agentName) parser.setAgentId(agentName);
        nodeIdSet = new Set();
        queue.resetNodeQueues();

        // Turn-lifecycle finalization (session_end / Esc-fallback / crash
        // grace timer) and the stuck-stream watchdog. `finalizeTurn` is
        // called below on the real `session_end` StreamEvent.
        const { finalizeTurn } = useTurnLifecycle({
            blockId,
            model,
            turnPhaseAtom,
            provider,
            queue,
            flushParserPending: () => parser.flushPending(),
            hasNodeId,
            addNodeId,
        });

        // Promotes accepted pending messages into user_message document
        // nodes. No-ops internally if pendingMessagesAtom wasn't provided.
        usePendingMessageAcceptance({
            blockId,
            model,
            pendingMessagesAtom,
            queue,
            hasNodeId,
            addNodeId,
            onTurnStartFromQueue,
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
        const subscription = fileSubject.subscribe((msg: { fileop: string; data64: string }) => {
            if (msg.fileop === "truncate") {
                // Reducer decides whether to honor — late truncates after
                // a socket-reconnect race are suppressed. Only reset the
                // hook-local parser/stats/etc. when the truncate is actually
                // honored; if suppressed, the live stream is still flowing
                // and resetting would corrupt in-flight parse state.
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
                return;
            }

            if (msg.fileop !== "append" || !msg.data64) return;

            // Decode base64 subprocess data to UTF-8 text
            const bytes = base64ToArray(msg.data64);
            const text = new TextDecoder().decode(bytes);

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
                if (rawEvent.type === "system" && rawEvent.subtype === "compact_boundary") {
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
                    const compactBoundary = parseCompactBoundaryFrame(rawEvent);
                    if (compactBoundary) {
                        const paneEvents = model.dispatchPane({
                            type: "CompactionBoundary",
                            trigger: compactBoundary.trigger,
                            preTokens: compactBoundary.preTokens,
                            postTokens: compactBoundary.postTokens,
                            durationMs: compactBoundary.durationMs,
                            at: Date.now(),
                            frameTimestamp: compactBoundary.frameTimestamp,
                        });
                        pushContextCompactedNodes(paneEvents, queue, hasNodeId, addNodeId);
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
                    const sessionOutcome = parseSessionOutcomeFrame(rawEvent);
                    if (sessionOutcome) {
                        const node: SessionOutcomeNode = {
                            type: "session_outcome",
                            id: sessionOutcomeNodeId(sessionOutcome),
                            outcome: sessionOutcome.outcome,
                            attemptedSid: sessionOutcome.attemptedSid,
                            actualSid: sessionOutcome.actualSid,
                            timestamp: sessionOutcomeLiveTimestamp(sessionOutcome.frameTimestamp),
                        };
                        if (!hasNodeId(node.id)) {
                            addNodeId(node.id);
                            queue.pushNewNode(node);
                            queue.scheduleFlush();
                        }
                    }
                    continue;
                }

                // Extract live token counts from Anthropic stream events before
                // the translator discards them. message_start carries input_tokens
                // for this turn; message_delta carries the running output_tokens.
                {
                    const inner = rawEvent.type === "stream_event" ? rawEvent.event : rawEvent;
                    if (inner?.type === "message_start") {
                        // input_tokens is only the uncached prompt; cache_creation/
                        // cache_read carry the rest of the real prompt size.
                        const u = inner.message?.usage;
                        const inputTok =
                            u?.input_tokens != null
                                ? (u.input_tokens as number)
                                  + ((u.cache_creation_input_tokens as number | undefined) ?? 0)
                                  + ((u.cache_read_input_tokens as number | undefined) ?? 0)
                                : undefined;
                        if (inputTok != null) {
                            // message.model is the resolved model id (e.g.
                            // "claude-opus-4-8") — used to seed the context-window
                            // meter per model (Opus/Sonnet 1M, Haiku 200K).
                            const modelId = inner.message?.model as string | undefined;
                            const paneEvents = model.dispatchPane({ type: "TokensIn", input: inputTok, model: modelId });
                            // Detect context compaction from the reducer's event output.
                            // Primary signal for Claude is the real CompactionBoundary
                            // path above; this heuristic (≥50% token drop from a >10k
                            // baseline) is suppressed by the reducer itself shortly
                            // after a real boundary landed, and remains the ONLY signal
                            // for providers with no structured event (codex/gemini/copilot).
                            pushContextCompactedNodes(paneEvents, queue, hasNodeId, addNodeId);
                        }
                    } else if (inner?.type === "message_delta") {
                        const outputTok = inner.usage?.output_tokens as number | undefined;
                        if (outputTok != null) {
                            model.dispatchPane({ type: "TokensOut", output: outputTok });
                        }
                    }
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
                        finalizeTurn(event.stats ?? null);
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
                        if (event.tool) {
                            model.dispatchPane({
                                type: "ToolStart",
                                name: event.tool,
                                arg: extractToolArg(event.tool, event.params),
                            });
                        } else {
                            model.dispatchPane({ type: "ToolEnd" });
                        }
                    } else if (event.type === "tool_result") {
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

            // Schedule a single flush per animation frame
            if (queue.hasPendingNewOrUpdated()) {
                queue.scheduleFlush();
            }
        });

        onCleanup(() => {
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
            // a synchronous documentAtom write here re-triggers reconcileArrays
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
}
