// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { modelTurnCommand } from "./model-turn-signal";

// Shapes as they appear in a real Claude stream-json feed with
// --include-partial-messages.
const messageStart = (parent?: string) => ({
    type: "stream_event",
    event: { type: "message_start", message: { usage: { input_tokens: 3 } } },
    ...(parent ? { parent_tool_use_id: parent } : {}),
});
const messageDelta = (stopReason: string | null, parent?: string) => ({
    type: "stream_event",
    event: { type: "message_delta", delta: { stop_reason: stopReason }, usage: { output_tokens: 9 } },
    ...(parent ? { parent_tool_use_id: parent } : {}),
});

describe("modelTurnCommand", () => {
    it("message_start: the model is working again", () => {
        expect(modelTurnCommand(messageStart())).toEqual({ type: "ModelMessageStarted" });
    });

    it("message_delta with end_turn: the model has ended its turn", () => {
        expect(modelTurnCommand(messageDelta("end_turn"))).toEqual({ type: "ModelEndedTurn" });
    });

    it("message_delta with tool_use: the model is about to wait on a tool, not done", () => {
        expect(modelTurnCommand(messageDelta("tool_use"))).toBeNull();
    });

    it("ignores a subagent's messages", () => {
        expect(modelTurnCommand(messageStart("toolu_parent"))).toBeNull();
        expect(modelTurnCommand(messageDelta("end_turn", "toolu_parent"))).toBeNull();
    });

    it("also reads an unwrapped event", () => {
        expect(modelTurnCommand({ type: "message_delta", delta: { stop_reason: "end_turn" } })).toEqual({
            type: "ModelEndedTurn",
        });
    });

    it("ignores everything else", () => {
        expect(modelTurnCommand({ type: "assistant", message: {} })).toBeNull();
        expect(modelTurnCommand({ type: "stream_event", event: { type: "content_block_delta" } })).toBeNull();
        expect(modelTurnCommand({ type: "result", subtype: "success" })).toBeNull();
    });
});
