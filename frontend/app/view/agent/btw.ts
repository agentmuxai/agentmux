// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `/btw <question>` — support code for the frontend half of AgentMux's
 * `/btw` (modelled on Claude Code CLI's own native one, and offered on
 * Claude, Codex and Gemini panes): a tool-less, context-aware side question asked
 * in a floating overlay (`components/BtwOverlay.tsx`) without touching the
 * pane's own live turn or transcript.
 *
 * Two pieces live here:
 *   - `buildBtwContextSnapshot` — turns the pane's `DocumentNode[]` into a
 *     compact, capped text summary suitable as LLM context.
 *   - `askSideQuestion` — the real implementation wired into
 *     `SlashCommandContext.askSideQuestion` at the `useAgentCommands({...})`
 *     call site in agent-view.tsx (mirrors `quick-fork.ts`'s `quickForkAgent`
 *     for `ctx.quickFork`).
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { DocumentNode } from "./types";

/**
 * Cap on how many rendered transcript lines are included, most-recent-first
 * (i.e. the OLDEST included lines are dropped first) — a side question is
 * almost always about what's happening right now, not the full session.
 */
const MAX_SNAPSHOT_NODES = 40;

/** Overall character budget for the snapshot, applied after node capping. */
const MAX_SNAPSHOT_CHARS = 6000;

/** Per-line budget so one enormous tool result/markdown block can't blow
 *  the whole budget on its own, starving every other line. */
const MAX_LINE_CHARS = 400;

function truncateLine(text: string, max: number): string {
    // Collapse internal whitespace/newlines — this is a one-line-per-node
    // summary, not a faithful re-render.
    const collapsed = text.replace(/\s+/g, " ").trim();
    return collapsed.length > max ? `${collapsed.slice(0, max)}…` : collapsed;
}

/**
 * Render one `DocumentNode` as a single compact line, or `null` to omit it
 * entirely (nodes with no meaningful textual content for an LLM prompt —
 * dividers, boundary markers, etc.).
 */
function renderSnapshotLine(node: DocumentNode): string | null {
    switch (node.type) {
        case "user_message":
            return `User: ${truncateLine(node.message, MAX_LINE_CHARS)}`;
        case "markdown":
            // Thinking blocks are the model's own internal reasoning, not
            // something it said — omit them from what is effectively a new
            // prompt to a (possibly different) model.
            if (node.metadata?.thinking) return null;
            return `Assistant: ${truncateLine(node.content, MAX_LINE_CHARS)}`;
        case "tool":
            return `Tool ${node.tool}: ${truncateLine(node.summary, MAX_LINE_CHARS)}`;
        case "agent_message": {
            const who = node.direction === "incoming" ? `from ${node.from}` : `to ${node.to}`;
            return `Agent message (${who}): ${truncateLine(node.message, MAX_LINE_CHARS)}`;
        }
        case "jekt_message": {
            const who = node.direction === "incoming" ? `from ${node.from}` : `to ${node.to}`;
            return `Jekt (${who}): ${truncateLine(node.message, MAX_LINE_CHARS)}`;
        }
        case "shell":
            return `Shell (${node.status}): ${truncateLine(node.title || node.cmd, MAX_LINE_CHARS)}`;
        case "agent_error":
            return `Error ${node.code}: ${truncateLine(node.message, MAX_LINE_CHARS)}`;
        // No meaningful textual content for an LLM prompt: pure UI
        // boundary/decoration nodes.
        case "section":
        case "context_compacted":
        case "compaction_started":
        case "session_outcome":
        case "day_divider":
        case "history_link":
        case "resume_preflight":
            return null;
        // AgentMux's own narration, not something the user or the model said —
        // it must not appear in a prompt as if it were part of the conversation.
        case "ambient_narration":
            return null;
        default:
            return null;
    }
}

/**
 * Turn `ctx.documentNodes()` into a reasonably-sized text summary suitable
 * as LLM context — capped by both node count and character budget (last
 * `MAX_SNAPSHOT_NODES` meaningful lines, then trimmed from the front if
 * still over `MAX_SNAPSHOT_CHARS`) rather than sending an unbounded
 * transcript. Pure function — no reactive reads — so it has direct unit
 * coverage without a SolidJS reactive-root harness.
 */
export function buildBtwContextSnapshot(nodes: DocumentNode[]): string {
    const lines: string[] = [];
    for (const node of nodes) {
        const line = renderSnapshotLine(node);
        if (line) lines.push(line);
    }
    const capped = lines.slice(-MAX_SNAPSHOT_NODES);
    let text = capped.join("\n");
    if (text.length > MAX_SNAPSHOT_CHARS) {
        // Keep the tail (most recent content), not the head.
        text = text.slice(text.length - MAX_SNAPSHOT_CHARS);
    }
    return text;
}

/**
 * Ask a `/btw` side question. Wired at the `useAgentCommands({...})` call
 * site in agent-view.tsx as `askSideQuestion: (question) =>
 * askSideQuestion(model.blockId, question, paneModel.document())`.
 *
 * Calls the backend's `AskSideQuestionCommand`
 * (`agentmux-srv/src/server/agent_handlers/side_question.rs`), which mints
 * `request_id` and returns immediately — the actual answer streams
 * separately as `WpsEvent.BtwAnswerChunk` events scoped
 * `block:<blockId>:btw:<requestId>` (see `mps-events.ts` and
 * `components/BtwOverlay.tsx`, which subscribes to that scope once this
 * resolves).
 */
export async function askSideQuestion(
    blockId: string,
    question: string,
    nodes: DocumentNode[]
): Promise<{ requestId: string }> {
    const result = await RpcApi.AskSideQuestionCommand(TabRpcClient, {
        block_id: blockId,
        question,
        context_snapshot: buildBtwContextSnapshot(nodes),
    });
    return { requestId: result.request_id };
}
