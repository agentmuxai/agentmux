// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Maps one raw provider stream line to the pane command that tracks whether
 * the main agent's model has ended its turn (`modelEndedTurn` on the Streaming
 * phase, read by working-indicator.ts). `null` for anything else.
 *
 * `message_start` means the model is working again; a `message_delta` with
 * `stop_reason: "end_turn"` means it has finished. `tool_use` doesn't: the
 * model is about to wait on a tool and will continue. Lines carrying a
 * `parent_tool_use_id` belong to a subagent and say nothing about the main
 * model.
 */
/** The fields of a raw stream line this reads; everything else is ignored. */
interface StreamLineShape {
    type?: string;
    parent_tool_use_id?: string | null;
    event?: StreamEventShape;
    delta?: { stop_reason?: string | null };
    /** Real lines carry many other fields. */
    [field: string]: unknown;
}
type StreamEventShape = Pick<StreamLineShape, "type" | "delta">;

export function modelTurnCommand(
    rawEvent: StreamLineShape,
): { type: "ModelEndedTurn" } | { type: "ModelMessageStarted" } | null {
    if (rawEvent.parent_tool_use_id) return null;
    const inner = rawEvent.type === "stream_event" ? rawEvent.event : rawEvent;
    if (inner?.type === "message_start") return { type: "ModelMessageStarted" };
    if (inner?.type === "message_delta" && inner.delta?.stop_reason === "end_turn") {
        return { type: "ModelEndedTurn" };
    }
    return null;
}
