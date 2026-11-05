// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Two small things useAgentStream does beside reading frames: noting a running tool's output (so the pane
 * can say "no output for 1m" and show a test run's progress), and checking the agent's shared transcript
 * zone for lines that no event brought.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentPaneModel } from "@/app/store/agent-pane-model";
import { createTestProgressTracker } from "./status/test-progress";
import type { TranscriptCursor } from "./transcript-cursor";

/** How often a pane on an agent's shared zone checks for lines no event brought. */
const SHARED_ZONE_POLL_MS = 5_000;
/** At most this often a call's output is reported to the reducer: enough for "no output for 1m". */
const OUTPUT_NOTE_MS = 5_000;

/** The handler for a tool's live output: reports it to the reducer, and a known test runner's progress. */
export function createToolOutputNoter(model: AgentPaneModel): (toolId: string, content: string) => void {
    // A known test runner's progress, from a call's live output.
    const testProgress = new Map<string, ReturnType<typeof createTestProgressTracker>>();
    // When each call last reported output to the reducer.
    const outputNoted = new Map<string, number>();
    return (toolId, content) => {
        const now = Date.now();
        if (now - (outputNoted.get(toolId) ?? 0) >= OUTPUT_NOTE_MS) {
            if (outputNoted.size > 64) outputNoted.clear();
            outputNoted.set(toolId, now);
            model.dispatchPane({ type: "ToolOutput", id: toolId, at: now });
        }
        let track = testProgress.get(toolId);
        if (!track) {
            if (testProgress.size > 64) testProgress.clear();
            track = createTestProgressTracker();
            testProgress.set(toolId, track);
        }
        const text = track(content);
        if (text) model.dispatchPane({ type: "ToolProgress", id: toolId, text });
    };
}

/**
 * Lines no event brings: another block or srv instance appending to the agent's shared zone. Filled one
 * poll late, so lines this pane's own events are still bringing (and its own echoes) arrive by event
 * first. Returns the function that stops it.
 */
export function startSharedZonePoll(blockId: string, filename: string, cursor: Pick<TranscriptCursor, "position" | "observeCount">): () => void {
    let lastCount: { count: number; stream: string; gen: string } | null = null;
    let polling = false;
    const timer = setInterval(async () => {
        const pin = cursor.position();
        if (!pin || !pin.stream.startsWith("g:") || polling || document.hidden) return;
        polling = true;
        try {
            const resp = await RpcApi.BlockfileLineCountCommand(TabRpcClient, { block_id: blockId, filename }, { timeout: 5_000 });
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
    return () => clearInterval(timer);
}
