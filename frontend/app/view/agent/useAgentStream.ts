// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentStream — SolidJS hook that subscribes to a block's subprocess output,
 * pipes it through the provider translator + stream parser, and feeds
 * the resulting DocumentNodes into SolidJS signals.
 */

import { getFileSubject, waveEventSubscribe } from "@/app/store/wps";
import * as WOS from "@/app/store/wos";
import { base64ToArray } from "@/util/util";
import { trail } from "@/log/render-trail";
import { batch, createEffect, onCleanup, onMount } from "solid-js";
import { createTranslator } from "./providers/translator-factory";
import type { PendingMessage, SignalPair } from "./state";
import { ClaudeCodeStreamParser, STARTUP_HEADING_RE } from "./stream-parser";
import type { DocumentNode, SessionStats, ShellNode, ToolLogChunk, UserMessageNode } from "./types";
import { recordTurn } from "@/store/token-usage";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";
import {
    dispatch as dispatchDoc,
    getNodeIdSet,
} from "@/app/store/agent-document-store";
import {
    dispatch as dispatchPane,
    snapshot as paneSnapshot,
} from "@/app/store/agent-pane-state-store";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";

const OutputFileName = "output";

/**
 * Watchdog tick rate. Every 5s the hook dispatches a StreamWatchdogTick
 * to the pane-state reducer; the reducer compares against
 * `STUCK_THRESHOLD_MS` (45s) and emits a `stream-stuck` event when the
 * subscribed stream has been silent that long. Issue #728 gap 3.
 */
const WATCHDOG_INTERVAL_MS = 5_000;

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
}: UseAgentStreamOpts): void {
    // Read-side accessors only — all writes route through dispatchPane.
    // Maintaining this contract is how the agent pane stays 100%
    // reducer-routed and why `recordDispatch` is a sufficient tap for
    // session-replay fixtures. See docs/analysis/AGENT_PANE_REDUCER_
    // AUDIT_2026_05_12.md.
    //
    // PR G: turnTokens is fetched directly from the reducer snapshot
    // at finalize time (paneSnapshot) instead of being threaded through
    // a dedicated accessor — fewer props, same content. Similarly, the
    // user-initiated-stop check reads `turnPhase.kind === "Interrupting"`
    // from the turnPhaseAtom that the agent-view registered.
    const [getTurnPhase] = turnPhaseAtom;

    // Mutable state that doesn't trigger re-renders
    let lineBuffer = "";
    let translator = createTranslator(outputFormat);
    let parser = new ClaudeCodeStreamParser();
    // nodeIdSet remains for fast in-batch dedup (the reducer also dedups,
    // but checking here avoids enqueuing already-seen nodes into pendingNew).
    let nodeIdSet = new Set<string>();

    // Batching: accumulate parsed nodes between RAF flushes
    let pendingNew: DocumentNode[] = [];
    let pendingUpdates: DocumentNode[] = [];
    let flushRafId: number | null = null;

    // Tool-chunk accumulator. `tool_chunk` WPS events previously called
    // dispatchDoc(ToolChunkAppend) directly — one immediate signal write per
    // chunk. During active tool streaming that means many independent signal
    // writes, each triggering its own Solid reactive flush. When a chunk write
    // races with a concurrent RAF StreamFlush (both live in the same browser
    // task), two separate runUpdates frames can interleave, leaving the <Index>
    // reconciler holding a stale `current` array → replaceChild NotFoundError.
    // Fix: accumulate chunks here and flush them inside the same batch() as
    // StreamFlush, so all documentAtom writes from streaming originate from one
    // code path and one reactive frame. (Retro: RETRO_REPLACECHILD_CRASH_2026-06-06.md)
    type PendingChunk = { toolId: string; chunk: ToolLogChunk };
    let pendingChunks: PendingChunk[] = [];

    // Shell-node accumulators. Mirrors the tool_chunk pattern — batched inside
    // the same RAF flush so ShellNodeCreate exists before ShellChunkAppend
    // tries to append to it. (SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md §5.6)
    type PendingShellCreate = { node: ShellNode };
    type PendingShellChunk = { shellId: string; chunk: ToolLogChunk };
    type PendingShellExit = { shellId: string; status: ShellNode["status"]; exitCode: number; exitedAt: number };
    let pendingShellCreates: PendingShellCreate[] = [];
    let pendingShellChunks: PendingShellChunk[] = [];
    let pendingShellExits: PendingShellExit[] = [];

    // Single per-block WPS subscription for `tool_chunk` events.
    // `agentmux-bashwrap exec` publishes every stdout/stderr line to a
    // fixed event name with `scopes: ["block:<id>"]` and the tool_use_id
    // in the payload. The broker persists ~1024 events per scope, so
    // the subscription installed on mount picks up any chunks that
    // landed before Claude's stream-json caught up enough for the
    // frontend to learn the tool_use_id — closes the late-subscribe
    // race that the previous per-tool subscription model could not.
    // See `docs/specs/SPEC_STREAMING_BASH_RUNNER_2026_05_11.md` §6.
    const blockChunkUnsub = waveEventSubscribe({
        eventType: "tool_chunk",
        scope: `block:${blockId}`,
        handler: (event: any) => {
            const data = event?.data;
            if (!data || typeof data !== "object") return;
            const toolId = typeof data.tool_id === "string" ? data.tool_id : "";
            if (!toolId) return;
            if (data.op === "terminal") {
                pendingChunks.push({
                    toolId,
                    chunk: {
                        kind: "system",
                        content: `[exited ${data.exit_code ?? "?"}]`,
                        timestamp: data.timestamp ?? Date.now(),
                    },
                });
                scheduleFlush();
                return;
            }
            if (data.op !== "chunk") return;
            pendingChunks.push({
                toolId,
                chunk: {
                    kind: data.kind ?? "stdout",
                    content: data.content ?? "",
                    timestamp: data.timestamp ?? Date.now(),
                },
            });
            scheduleFlush();
        },
    });

    // Own the tool_chunk subscription at body scope so it is torn down even if
    // onMount early-returns (e.g. enabled:false). Its only other teardown lives
    // inside onMount's onCleanup, which is skipped on early-return — so without
    // this the global handler would leak one per mount.
    onCleanup(() => { try { blockChunkUnsub(); } catch { /* ignore */ } });

    // shell_node_create: backend published immediately when Shell tool is called.
    // We build the full ShellNode and queue it for the next RAF flush.
    const shellNodeCreateUnsub = waveEventSubscribe({
        eventType: "shell_node_create",
        scope: `block:${blockId}`,
        handler: (event: any) => {
            const d = event?.data;
            if (!d || typeof d !== "object") return;
            const shellId = typeof d.shell_id === "string" ? d.shell_id : "";
            if (!shellId) return;
            const node: ShellNode = {
                type: "shell",
                id: shellId,
                cmd: d.cmd ?? "",
                title: d.title ?? d.cmd ?? "",
                cwd: typeof d.cwd === "string" ? d.cwd : undefined,
                status: "running",
                spawnedAt: d.timestamp ?? Date.now(),
                log: { chunks: [], open: true },
            };
            pendingShellCreates.push({ node });
            scheduleFlush();
        },
    });
    onCleanup(() => { try { shellNodeCreateUnsub(); } catch { /* ignore */ } });

    // shell_chunk: per-line output (op="chunk") or process exit (op="exit").
    const shellChunkUnsub = waveEventSubscribe({
        eventType: "shell_chunk",
        scope: `block:${blockId}`,
        handler: (event: any) => {
            const d = event?.data;
            if (!d || typeof d !== "object") return;
            const shellId = typeof d.shell_id === "string" ? d.shell_id : "";
            if (!shellId) return;
            if (d.op === "exit") {
                const exitCode = typeof d.exit_code === "number" ? d.exit_code : -1;
                const status: ShellNode["status"] = exitCode === 0 ? "exited-ok" : "exited-err";
                pendingShellExits.push({ shellId, status, exitCode, exitedAt: d.timestamp ?? Date.now() });
                scheduleFlush();
                return;
            }
            if (d.op !== "chunk") return;
            pendingShellChunks.push({
                shellId,
                chunk: {
                    kind: d.kind ?? "stdout",
                    content: d.content ?? "",
                    timestamp: d.timestamp ?? Date.now(),
                },
            });
            scheduleFlush();
        },
    });
    onCleanup(() => { try { shellChunkUnsub(); } catch { /* ignore */ } });

    function flushPendingNodes() {
        flushRafId = null;
        if (pendingNew.length === 0 && pendingUpdates.length === 0 && pendingChunks.length === 0
            && pendingShellCreates.length === 0 && pendingShellChunks.length === 0 && pendingShellExits.length === 0) return;

        const batchNew = pendingNew;
        const batchUpdates = pendingUpdates;
        const batchChunks = pendingChunks;
        const batchShellCreates = pendingShellCreates;
        const batchShellChunks = pendingShellChunks;
        const batchShellExits = pendingShellExits;
        pendingNew = [];
        pendingUpdates = [];
        pendingChunks = [];
        pendingShellCreates = [];
        pendingShellChunks = [];
        pendingShellExits = [];

        // Wrap both store writes in a single Solid batch so all reactive
        // effects (partition memo → <Index> reconciler, DocumentRow
        // re-renders, pane-state observers) settle together in one
        // synchronous pass. Without batch(), the two sequential writes
        // can interleave reactive re-renders: the first write triggers
        // the <Index> outer reconciler, which starts inserting new DOM
        // rows; the second write (or a concurrent DocumentRow update
        // triggered by the first) then mutates the same DOM subtree
        // mid-reconcile, causing reconcileArrays to call replaceChild on
        // a node that was just moved — the confirmed crash root cause
        // (render_trail 2026-06-05: replaceChild / reconcileArrays /
        // insertExpression in solid-js/web).
        batch(() => {
            // Document mutation first — the reducer owns dedup, in-place
            // updates, and the markdown-content merge. StreamFlush must run
            // BEFORE ToolChunkAppend so that any ToolNode created by this
            // flush exists before we try to append chunks to it. Chunks that
            // arrive before their ToolNode is created (the WPS late-subscribe
            // case) are dropped by the reducer's findToolIndex guard; ordering
            // StreamFlush first is the narrowest window possible.
            model.dispatchDoc({
                type: "StreamFlush",
                newNodes: batchNew,
                updatedNodes: batchUpdates,
            });
            // Tool-chunk appends after StreamFlush has committed the ToolNode.
            for (const { toolId, chunk } of batchChunks) {
                model.dispatchDoc({ type: "ToolChunkAppend", toolId, chunk });
            }
            // Shell: create nodes first, then chunks, then exits —
            // same ordering guarantee as the tool_chunk/StreamFlush pair.
            for (const { node } of batchShellCreates) {
                model.dispatchDoc({ type: "ShellNodeCreate", node });
            }
            for (const { shellId, chunk } of batchShellChunks) {
                model.dispatchDoc({ type: "ShellChunkAppend", shellId, chunk });
            }
            for (const { shellId, status, exitCode, exitedAt } of batchShellExits) {
                model.dispatchDoc({ type: "ShellStatusUpdate", shellId, status, exitCode, exitedAt });
            }
            // Lifecycle counter bump — agent-pane-state owns streaming
            // metadata (active flag + bufferSize + lastEventTime).
            model.dispatchPane({
                type: "StreamFlushObserved",
                addedCount: batchNew.length,
                at: Date.now(),
            });
        });
    }

    function scheduleFlush() {
        if (flushRafId == null) {
            flushRafId = requestAnimationFrame(flushPendingNodes);
        }
    }

    onMount(() => {
        if (!enabled || !blockId) return;

        // Reset state on new subscription
        lineBuffer = "";
        translator = createTranslator(outputFormat);
        parser = new ClaudeCodeStreamParser();
        nodeIdSet = new Set();
        pendingNew = [];
        pendingUpdates = [];

        /**
         * Shared finalization for "the turn is over." Called by both the
         * real `session_end` event from the CLI's result line AND the
         * fallback timer armed when the user presses Esc (killing the
         * subprocess prevents it from emitting its own result event).
         *
         * If `turnPhase.kind === "Interrupting"` when this runs, the
         * ending was user-initiated — append a visible "⏹ Interrupted
         * by user" row to the document so the user has durable
         * confirmation that the stop landed.
         */
        const finalizeTurn = (stats: SessionStats | null) => {
            parser.flushPending();
            // Snapshot live turn tokens into SessionStats before nulling
            // the live signal. The Worked footer reads from sessionStats
            // (since turnTokens is cleared on session_end); without this
            // merge the headline tokens-in-Worked feature shows nothing.
            // Per PR #549 reagent/codex P1.
            //
            // PR G: turn-tokens are read from the reducer snapshot
            // instead of a dedicated signal accessor — same source of
            // truth, fewer props threaded into the hook.
            // Prefer the result event's turn-total usage. The live
            // turnTokens hold only the last message_start/message_delta
            // (TokensIn/TokensOut overwrite, not accumulate), so they
            // undercount multi-call turns; fall back to them only when
            // session_end carries no usage (e.g. providers without a
            // token-bearing result line).
            const liveTokens = paneSnapshot(blockId)?.turnTokens ?? null;
            const statsTokens =
                stats && (stats.input_tokens != null || stats.output_tokens != null)
                    ? { input: stats.input_tokens ?? 0, output: stats.output_tokens ?? 0 }
                    : null;
            const tokens = statsTokens ?? liveTokens;
            // Aggregate the completed turn's tokens into the global
            // session-local token-usage store so the status bar's
            // indicator + breakdown popover stay up to date. Guarded
            // against double-counting by recordTurn's own no-op-on-zero
            // check — see SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §5.1.
            if (provider && tokens) {
                recordTurn(provider, tokens);
            }
            // Detect user-initiated stop via the reducer's turn phase.
            // PR G: replaces the legacy `stoppingAtom` getter — the
            // predicate is exactly `turnPhase.kind === "Interrupting"`
            // since RequestStop is the only command that enters
            // Interrupting and TurnEnd is the next transition out.
            // The reducer's TurnEnd handler does the cross-atom cleanup
            // in one shot: merges live tokens into stats, clears
            // tool/tokens, and transitions the phase to Done.
            const wasStopping = getTurnPhase().kind === "Interrupting";
            model.dispatchPane({ type: "TurnEnd", stats });
            if (wasStopping) {
                const interruptedNode: DocumentNode = {
                    type: "markdown",
                    id: `interrupted-${Date.now()}`,
                    content: "⏹ _Interrupted by user_",
                    timestamp: Date.now(),
                };
                if (!nodeIdSet.has(interruptedNode.id)) {
                    nodeIdSet.add(interruptedNode.id);
                    pendingNew.push(interruptedNode);
                    scheduleFlush();
                }
            }
        };

        // Fallback timer: if the user presses Esc and the CLI doesn't
        // emit `session_end` within 1.5s (normal for a killed subprocess
        // — TerminateProcess skips any final output), run the same
        // finalization locally so the UI doesn't hang on "Stopping…".
        //
        // PR G: previously gated on `getStopping()` (legacy boolean);
        // now gated on `turnPhase.kind === "Interrupting"`. Same edge
        // semantics — the reducer transitions into Interrupting on
        // RequestStop and out on TurnEnd / disconnect.
        let stopFallbackTimer: number | null = null;
        createEffect(() => {
            const stopping = getTurnPhase().kind === "Interrupting";
            if (stopFallbackTimer != null) {
                clearTimeout(stopFallbackTimer);
                stopFallbackTimer = null;
            }
            if (stopping) {
                stopFallbackTimer = window.setTimeout(() => {
                    stopFallbackTimer = null;
                    if (getTurnPhase().kind === "Interrupting") {
                        finalizeTurn(null);
                    }
                }, 1500);
            }
        });
        onCleanup(() => {
            if (stopFallbackTimer != null) {
                clearTimeout(stopFallbackTimer);
                stopFallbackTimer = null;
            }
        });

        // Subscribe to `agent-message-accepted`: when the backend picks
        // up a queued message, promote the matching entry out of the
        // pending zone into a real `user_message` document node. That
        // color shift (amber → accent blue) is the user's visible
        // "accepted" signal — the spec's core requirement.
        if (pendingMessagesAtom) {
            const [getPending] = pendingMessagesAtom;
            const acceptedUnsub = waveEventSubscribe({
                eventType: "agent-message-accepted",
                scope: WOS.makeORef("block", blockId),
                handler: (event) => {
                    const data = (event as any)?.data;
                    if (!data) return;
                    const messageId: string | undefined = data.message_id;
                    if (!messageId) return;
                    const pending = getPending().find((m) => m.id === messageId);
                    if (!pending) {
                        // Accepted event for an id we don't know about.
                        // Can legitimately happen if the entry was already
                        // promoted or the pane was re-mounted mid-queue.
                        return;
                    }
                    // Reducer removes the entry + emits pending-accepted.
                    model.dispatchPane({
                        type: "PendingMessageAccepted",
                        id: messageId,
                    });
                    // Queue-drain case: the prior turn ended (phase Done/Idle/
                    // Disconnected) and the backend is now picking up the next
                    // queued message. Re-enter Submitting so the working
                    // animation re-activates.
                    //
                    // For idle sends (phase Submitting/Streaming/Interrupting),
                    // TurnStart was already dispatched in handleSendMessage —
                    // a second fire here regresses Streaming → Submitting and
                    // re-arms the 30 s submit timeout unnecessarily.
                    // See docs/analysis/ANALYSIS_IDLE_SEND_RACE_2026_06_11.md.
                    const currentPhase = paneSnapshot(blockId)?.turnPhase;
                    const needsTurnStart =
                        currentPhase?.kind === "Done" ||
                        currentPhase?.kind === "Idle" ||
                        currentPhase?.kind === "Disconnected" ||
                        currentPhase == null;
                    if (needsTurnStart) {
                        trail("agent:dispatch:TurnStart", { messageId });
                        model.dispatchPane({ type: "TurnStart", at: Date.now() });
                        trail("agent:dispatch:TurnStart:done", { messageId });
                    }
                    // Append as a normal user_message so it joins the
                    // conversation stream. Keeps the same id so the new
                    // node ties back to the pending entry 1:1.
                    // The optimistic-acceptance path goes through the
                    // same `handleSendMessage` pipeline as the startup
                    // injection (see agent-view.tsx `onReadyFn`). Apply
                    // the same heuristic here as in the stream-parser
                    // so the startup payload is flagged on first
                    // render — otherwise UserMessageBlock would render
                    // it as a regular user message (the full Markdown
                    // wall, not the collapsed summary).
                    // Codex P1 round 2 on PR #1020.
                    const node: UserMessageNode = {
                        type: "user_message",
                        id: pending.id,
                        message: pending.text,
                        timestamp: Date.now(),
                        isStartup: STARTUP_HEADING_RE.test(pending.text),
                    };
                    if (!nodeIdSet.has(node.id)) {
                        nodeIdSet.add(node.id);
                        pendingNew.push(node);
                        scheduleFlush();
                    }
                },
            });
            onCleanup(() => acceptedUnsub());
        }

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

        // Two reducers signaled in lockstep: pane-state owns the streaming
        // metadata (active flag), agent-document owns the session phase
        // gate that drives truncate suppression.
        const subscribedAt = Date.now();
        model.dispatchPane({ type: "StreamSubscribe", at: subscribedAt });
        model.dispatchDoc({ type: "SessionStart", at: subscribedAt });

        // Stuck-stream watchdog (issue #728 gap 3). The reducer evaluates
        // each tick against `lastEventMs` and emits a `stream-stuck`
        // event when the gap exceeds `STUCK_THRESHOLD_MS`. The interval
        // cleans up via the same effect cleanup as the subscription.
        const watchdogId = setInterval(() => {
            model.dispatchPane({
                type: "StreamWatchdogTick",
                nowMs: Date.now(),
            });
        }, WATCHDOG_INTERVAL_MS);
        onCleanup(() => clearInterval(watchdogId));

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
                if (flushRafId != null) { cancelAnimationFrame(flushRafId); flushRafId = null; }
                pendingNew = [];
                pendingUpdates = [];
                pendingChunks = [];
                pendingShellCreates = [];
                pendingShellChunks = [];
                pendingShellExits = [];
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
                    pendingNew.push({
                        id: `stderr-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
                        type: "markdown",
                        content: `**stderr:** ${text}`,
                        timestamp: Date.now(),
                    });
                    scheduleFlush();
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
                            model.dispatchPane({ type: "TokensIn", input: inputTok });
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
                    // Track the currently-running tool for the status line.
                    // Per-tool subscription open/close was removed — a single
                    // per-block subscription installed on mount above handles
                    // every tool's chunks (the wrapper publishes on a fixed
                    // event name with the tool_use_id in the payload), and
                    // the broker's replay-on-subscribe covers the late-
                    // subscribe race that the per-tool model lost.
                    if (event.type === "tool_call") {
                        if (event.tool) {
                            model.dispatchPane({ type: "ToolStart", name: event.tool });
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
                        pendingChunks.push({ toolId, chunk });
                        scheduleFlush();
                        continue;
                    }
                    const node = parser.parseLine(JSON.stringify(event));
                    if (!node) continue;

                    // Stamp a receive time on nodes that don't carry their own
                    // timestamp (markdown, tool, section, subagent_link).
                    // user_message and agent_message already have timestamps.
                    if (!("timestamp" in node) || (node as any).timestamp == null) {
                        (node as any).timestamp = Date.now();
                    }

                    if (nodeIdSet.has(node.id)) {
                        pendingUpdates.push(node);
                    } else {
                        nodeIdSet.add(node.id);
                        pendingNew.push(node);
                    }
                }
            }

            // Schedule a single flush per animation frame
            if (pendingNew.length > 0 || pendingUpdates.length > 0) {
                scheduleFlush();
            }
        });

        onCleanup(() => {
            if (flushRafId != null) { cancelAnimationFrame(flushRafId); flushRafId = null; }
            subscription.unsubscribe();
            // (the tool_chunk subscription is torn down by its own body-scope
            // onCleanup registered where blockChunkUnsub is created — so it is
            // cleaned up even when this onMount early-returns.)
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
