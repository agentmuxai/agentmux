// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Replay driver for agent-pane session fixtures.
 *
 * The single public entry point — `replayInstant(fixture)` — applies
 * every event in `seq` order using the real reducers + the real
 * stream-parser. No SolidJS rendering, no slot-store registration,
 * no async waits. Returns the final document + pane state for the
 * test to assert against.
 *
 * Three event sources, three demuxer arms:
 *
 * 1. `stream-json` — feed the raw line to `ClaudeCodeStreamParser`.
 *    Each line that yields a `DocumentNode` becomes a `StreamFlush`
 *    command (new vs update routed by id-presence). Lines that yield
 *    `null` (partial, init, session-end-only) are skipped.
 *
 * 2. `wps` — match `event` + `data.op` to the right reducer command:
 *      - `tool_chunk` + `op: "chunk"` → `dispatchDoc(ToolChunkAppend)`
 *      - `tool_chunk` + `op: "terminal"` → synthesize a system chunk
 *        (the frontend's chunk handler does the same — see
 *        `useAgentStream.ts` `blockChunkUnsub` handler).
 *      - Other broker events are recorded into `warnings` and skipped
 *        (extend as needed).
 *
 * 3. `dispatch` — direct command application. `store` discriminates
 *    between doc and pane.
 *
 * Higher-fidelity modes (`realtime`, `step`) come in a follow-up —
 * `instant` is enough for unit-level reducer + parser regression
 * coverage, which is Phase 3 of the spec.
 */

import { ClaudeCodeStreamParser } from "@/app/view/agent/stream-parser";
import type { ProviderDefinition } from "@/app/view/agent/providers";
import { update as updateDoc } from "@/app/store/agent-document/reducer";
import {
    initialState as initialDocState,
    type AgentDocumentState,
    type AgentDocumentCommand,
} from "@/app/store/agent-document/types";
import { update as updatePane } from "@/app/store/agent-pane-state/reducer";
import {
    initialState as initialPaneState,
    type AgentPaneState,
    type AgentPaneCommand,
} from "@/app/store/agent-pane-state/types";

import type {
    Fixture,
    FixtureWpsEvent,
    FixtureStreamEvent,
    FixtureDispatchEvent,
} from "./types";

export interface ReplayResult {
    docState: AgentDocumentState;
    paneState: AgentPaneState;
    /** Non-fatal anomalies the driver noticed (unknown event types,
     *  dropped chunks, parser nulls, etc). Tests can assert on
     *  `warnings.length === 0` for the strict path. */
    warnings: string[];
    /** Stats — handy for diagnostic asserts. */
    stats: {
        streamLinesParsed: number;
        nodesAppended: number;
        wpsEvents: number;
        toolChunksApplied: number;
        dispatchEvents: number;
        eventsDropped: number;
    };
}

export interface ReplayOptions {
    /** Frozen wall clock for deterministic `nowMs` injection.
     *  Defaults to the fixture's header time. */
    nowMs?: number;
    /** Provider for the parser. Defaults to a minimal stub —
     *  override when testing provider-specific paths. */
    provider?: ProviderDefinition;
}

/**
 * Apply every event in fixture order. Synchronous.
 */
export function replayInstant(
    fixture: Fixture,
    options: ReplayOptions = {},
): ReplayResult {
    const nowMs = options.nowMs ?? (Date.parse(fixture.header.recorded_at) || 0);
    const provider = options.provider ?? minimalProvider(fixture.header.provider);

    const blockId = fixture.header.block_id;
    const agentId = `replay-${blockId}`;

    let docState = initialDocState();
    let paneState = initialPaneState(agentId);
    const warnings: string[] = [];
    const stats = {
        streamLinesParsed: 0,
        nodesAppended: 0,
        wpsEvents: 0,
        toolChunksApplied: 0,
        dispatchEvents: 0,
        eventsDropped: 0,
    };

    const parser = new ClaudeCodeStreamParser(provider);
    const nodeIds = new Set<string>();

    const applyDoc = (cmd: AgentDocumentCommand): void => {
        const result = updateDoc(docState, cmd, nowMs);
        docState = result.state;
        if (cmd.type === "StreamFlush") {
            for (const n of cmd.newNodes) nodeIds.add(n.id);
        } else if (cmd.type === "HistoryLoaded") {
            for (const n of cmd.nodes) nodeIds.add(n.id);
        }
        // Surface tool-chunk-dropped events as warnings — they're the
        // late-subscribe race signal we care about diagnosing.
        for (const ev of result.events) {
            if (ev.type === "tool-chunk-dropped") {
                warnings.push(
                    `dropped tool_chunk for ${ev.toolId} — reason: ${ev.reason}`,
                );
                stats.eventsDropped += 1;
            }
        }
    };

    const applyPane = (cmd: AgentPaneCommand): void => {
        const result = updatePane(paneState, cmd, nowMs);
        paneState = result.state;
    };

    for (const ev of fixture.events) {
        if (ev.src === "stream-json") {
            handleStreamLine(ev, parser, nodeIds, applyDoc, stats, warnings);
        } else if (ev.src === "wps") {
            handleWpsEvent(ev, applyDoc, stats, warnings);
        } else if (ev.src === "dispatch") {
            handleDispatch(ev, applyDoc, applyPane, stats, warnings);
        }
    }

    return { docState, paneState, warnings, stats };
}

function handleStreamLine(
    ev: FixtureStreamEvent,
    parser: ClaudeCodeStreamParser,
    seenIds: Set<string>,
    applyDoc: (cmd: AgentDocumentCommand) => void,
    stats: ReplayResult["stats"],
    warnings: string[],
): void {
    stats.streamLinesParsed += 1;
    const node = parser.parseLine(ev.line);
    if (!node) {
        // Parser returned null. Expected for line types the parser
        // intentionally ignores (e.g. `tool_chunk` lines handled
        // out-of-band, message_start/_stop). Surface it as a
        // soft warning so authors of new fixtures notice when a
        // line they expected to yield a node silently doesn't.
        warnings.push(
            `stream-json line at seq ${ev.seq} produced no node ` +
                `(line starts with: ${ev.line.slice(0, 80)})`,
        );
        return;
    }
    const isUpdate = seenIds.has(node.id);
    if (isUpdate) {
        applyDoc({ type: "StreamFlush", newNodes: [], updatedNodes: [node] });
    } else {
        applyDoc({ type: "StreamFlush", newNodes: [node], updatedNodes: [] });
        stats.nodesAppended += 1;
    }
}

function handleWpsEvent(
    ev: FixtureWpsEvent,
    applyDoc: (cmd: AgentDocumentCommand) => void,
    stats: ReplayResult["stats"],
    warnings: string[],
): void {
    stats.wpsEvents += 1;
    if (ev.event !== "tool_chunk") {
        // Recognized but unhandled — extend the demuxer when a test
        // needs controller-status / blockfile replay.
        warnings.push(
            `wps event "${ev.event}" not handled by replay driver (seq ${ev.seq})`,
        );
        return;
    }
    const data = ev.data as {
        op?: string;
        tool_id?: string;
        kind?: string;
        content?: string;
        timestamp?: number;
        exit_code?: number;
    } | null;
    if (!data || typeof data !== "object") return;
    const toolId = typeof data.tool_id === "string" ? data.tool_id : "";
    if (!toolId) return;

    if (data.op === "chunk") {
        applyDoc({
            type: "ToolChunkAppend",
            toolId,
            chunk: {
                kind: (data.kind as never) ?? "stdout",
                content: data.content ?? "",
                timestamp: data.timestamp ?? ev.t_ms,
            },
        });
        stats.toolChunksApplied += 1;
    } else if (data.op === "terminal") {
        // Mirrors useAgentStream's handler: synthesize a system chunk.
        applyDoc({
            type: "ToolChunkAppend",
            toolId,
            chunk: {
                kind: "system",
                content: `[exited ${data.exit_code ?? "?"}]`,
                timestamp: data.timestamp ?? ev.t_ms,
            },
        });
        stats.toolChunksApplied += 1;
    }
}

function handleDispatch(
    ev: FixtureDispatchEvent,
    applyDoc: (cmd: AgentDocumentCommand) => void,
    applyPane: (cmd: AgentPaneCommand) => void,
    stats: ReplayResult["stats"],
    warnings: string[],
): void {
    stats.dispatchEvents += 1;
    if (ev.store === "doc") {
        applyDoc(ev.action as AgentDocumentCommand);
    } else if (ev.store === "pane") {
        applyPane(ev.action as AgentPaneCommand);
    } else {
        warnings.push(`dispatch event seq ${ev.seq}: unknown store "${ev.store}"`);
    }
}

/** Lightweight stub provider — enough for the parser to emit basic
 *  nodes. Replace via `options.provider` for provider-specific tests. */
function minimalProvider(name: string): ProviderDefinition {
    return {
        id: name,
        displayName: name,
        styledOutputFormat: "stream-json",
        sessionIdField: "session_id",
        resumeFlag: "",
        controllerType: "subprocess",
    } as unknown as ProviderDefinition;
}
