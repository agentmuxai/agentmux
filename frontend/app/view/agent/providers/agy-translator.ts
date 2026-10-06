// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { SessionStats, StreamEvent } from "../types";
import { replayedUserMessage, type OutputTranslator } from "./translator";
import { ToolCorrelator, wrapOutput } from "./tool-correlation";

/**
 * Translates Antigravity CLI (`agy --output-format stream-json`) events into
 * StreamEvent format. Not Gemini CLI's schema, despite the shared lineage;
 * captured against agy 1.1.11 in
 * docs/specs/SPEC_ANTIGRAVITY_HARNESS_REAL_CLI_2026_10_06.md:
 *
 *   {"event":"init","conversation_id":"…","init":{"cwd":"…","tools":[…]}}
 *   {"event":"step_update","step_update":{"step_index":4,"state":"ACTIVE","step_type":"agent_response","text_delta":"The"}}
 *   {"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"view_file","tool_info":{"parameters":{…}}}}
 *   {"event":"step_update","step_update":{"step_index":2,"state":"DONE","step_type":"tool","tool_name":"view_file","tool_info":{"parameters":{…},"output":"…"}}}
 *   {"event":"result","result":{"status":"SUCCESS","response":"…","usage":{…}}}
 *
 * - Reply text streams as `text_delta` pieces on one step; each piece is
 *   incremental, and the stream-parser accumulates consecutive text events.
 * - A tool step is seen `ACTIVE` when it starts and `DONE` when it ends. Its
 *   id is the conversation id plus the step index: step indexes keep counting
 *   across a resumed conversation, so the pair is unique, and it comes from
 *   the line itself, so re-reading one stored line (`readToolResult`) gives
 *   the same id the live stream did.
 * - `agy` doesn't echo the prompt; the backend writes the user record, which
 *   only replay renders (`replayedUserMessage`).
 */
export class AgyTranslator implements OutputTranslator {
    private tools = new ToolCorrelator();
    /** Tool steps of this turn whose call has been emitted. */
    private startedTools = new Set<string>();

    constructor(private readonly opts: { replay?: boolean } = {}) {}

    translate(rawEvent: any): StreamEvent[] {
        if (!rawEvent || typeof rawEvent !== "object") return [];
        const user = replayedUserMessage(rawEvent, this.opts.replay);
        if (user) return user;

        switch (rawEvent.event) {
            case "init":
                this.startedTools.clear();
                return [];
            case "step_update":
                return this.step(rawEvent.step_update);
            case "result":
                return this.result(rawEvent.result);
            default:
                return [];
        }
    }

    private step(step: any): StreamEvent[] {
        if (!step || typeof step !== "object") return [];
        switch (step.step_type) {
            case "agent_response": {
                const text = step.text_delta;
                return typeof text === "string" && text ? [{ type: "text", content: text }] : [];
            }
            case "tool":
                return this.toolStep(step);
            default:
                // user_input, checkpoint and anything newer carry nothing to show.
                return [];
        }
    }

    private toolStep(step: any): StreamEvent[] {
        const id = `agy-${step.conversation_id ?? ""}-${step.step_index ?? ""}`;
        const name: string = step.tool_name ?? step.tool_info?.name ?? "unknown";
        const params: Record<string, any> = step.tool_info?.parameters ?? {};
        const out: StreamEvent[] = [];
        if (!this.startedTools.has(id)) {
            this.startedTools.add(id);
            out.push(this.tools.call(name, id, params));
        }
        if (step.state === "ACTIVE") return out;
        const failed = step.state !== "DONE" || step.tool_info?.error != null;
        const output = step.tool_info?.output ?? step.tool_info?.error ?? "";
        out.push(this.tools.result(id, failed ? "failed" : "success", wrapOutput(output)));
        return out;
    }

    private result(result: any): StreamEvent[] {
        const out: StreamEvent[] = [];
        if (result?.status && result.status !== "SUCCESS") {
            const msg: string = result.error?.message ?? result.error ?? `antigravity turn ${result.status}`;
            out.push({ type: "text", content: `**Error:** ${msg}` });
        }
        const stats: SessionStats = {};
        const usage = result?.usage;
        if (usage && typeof usage === "object") {
            if (typeof usage.input_tokens === "number") stats.input_tokens = usage.input_tokens;
            if (typeof usage.output_tokens === "number") stats.output_tokens = usage.output_tokens;
        }
        if (typeof result?.duration_seconds === "number") stats.duration_ms = Math.round(result.duration_seconds * 1000);
        if (typeof result?.num_turns === "number") stats.num_turns = result.num_turns;
        out.push({ type: "session_end", stats });
        return out;
    }

    reset(): void {
        this.tools.reset();
        this.startedTools.clear();
    }
}
