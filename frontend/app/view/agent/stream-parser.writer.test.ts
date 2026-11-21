// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Rows a container agent may have written carry that to the row (`fromContainer`), from srv's `amx.writer`
 * (SPEC_MESSAGES_CARRY_THE_RUNTIME_OF_THEIR_WRITER_2026_11_20.md).
 */

import { describe, expect, test } from "vitest";
import { writtenInContainer } from "./providers/translator";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { UserMessageEvent } from "../../agent-model/types";

describe("writtenInContainer", () => {
    test("only srv's container writer marks a record", () => {
        expect(writtenInContainer({ amx: { v: 1, writer: { runtime: "container", agent: "a" }, verified: true } })).toBe(true);
        expect(writtenInContainer({ amx: { writer: { runtime: "container", attributed: true } } })).toBe(true);
    });

    test.each([
        ["a host writer", { amx: { writer: { runtime: "host" } } }],
        ["a writer with no runtime", { amx: { writer: {} } }],
        ["a runtime that is not a word we know", { amx: { writer: { runtime: "Container" } } }],
        ["no amx", { type: "assistant" }],
        ["a malformed amx", { amx: "container" }],
        ["null", null],
    ])("%s marks nothing", (_name, record) => {
        expect(writtenInContainer(record)).toBe(false);
    });
});

describe("the parser's writer context", () => {
    test("rows made while it is set are marked, and rows made after it is cleared are not", () => {
        const parser = new ClaudeCodeStreamParser();
        parser.setFromContainer(true);
        const marked = parser.parseStreamEvent({ type: "text", content: "![](/home/me/secret.png)" });
        parser.endTurn();
        parser.setFromContainer(false);
        const plain = parser.parseStreamEvent({ type: "text", content: "hello" });
        expect((marked as { fromContainer?: boolean }).fromContainer).toBe(true);
        expect((plain as { fromContainer?: boolean }).fromContainer).toBeUndefined();
    });

    test("a tool row made in a container record is marked", () => {
        const parser = new ClaudeCodeStreamParser();
        parser.setFromContainer(true);
        const node = parser.parseStreamEvent({ type: "tool_call", tool: "Read", id: "t1", params: { file_path: "/x.png" } });
        expect((node as { fromContainer?: boolean } | null)?.fromContainer).toBe(true);
    });

    test("a jekt held for a block that is still open keeps the mark of the record it came in", () => {
        const parser = new ClaudeCodeStreamParser();
        parser.parseStreamEvent({ type: "text", content: "working" });
        parser.setFromContainer(true);
        const jekt: UserMessageEvent = {
            type: "user_message",
            message: "[JEKT:FROM=boxed TO=me TIER=info DELIVERY=host TRUST=host-verified]\nsee /x.png\n[/JEKT]",
        };
        expect(parser.parseStreamEvent(jekt)).toBeNull();
        parser.setFromContainer(false);
        parser.releaseHeld();
        const released = parser.drainReleased();
        expect(released.length).toBeGreaterThan(0);
        for (const n of released) expect((n as { fromContainer?: boolean }).fromContainer).toBe(true);
    });
});
