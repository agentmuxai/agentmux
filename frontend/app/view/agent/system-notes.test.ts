// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * P0 of SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md: srv's `[AgentMux]`
 * notes get their own nodes, so neither a note nor the jekt it follows takes the
 * human's style (the operator's 0.60.1 screenshot). The split only narrows: text
 * it takes out of a message never ends up in the human's node.
 */

import { beforeEach, describe, expect, test } from "vitest";
import { ClaudeCodeStreamParser } from "./stream-parser";
import { splitSystemNotes } from "./system-notes";
import type { DocumentNode, JektMessageNode, SystemNoteNode, UserMessageNode } from "./types";

const JEKT = [
    "[JEKT:FROM=agent1 TO=agentx TIER=coord DELIVERY=host TRUST=host-verified MSGID=1792963213963-30516-1 PRIORITY=normal TS=1792963213]",
    "────────────────────────────────────────────────────────────",
    "From: agent1 | To: agentx | ts=1792963213",
    "Agent1 -> AgentX: #5538 and #5539 are merged.",
    "────────────────────────────────────────────────────────────",
    "Reply: mcp__agentmux__Message (action \"send\", to \"agent1\")",
    "[/JEKT]",
].join("\n");
const NOTE = "[AgentMux] AgentMux was upgraded from 0.60.0 to 0.60.1 at 14:17. Your session resumed; nothing is needed from you.";
const NOTE_2 = "[AgentMux] Agent5 also has uncommitted changes in crates/x.rs (repo a/b, branch c). Message them before changing it.";

let parser: ClaudeCodeStreamParser;
beforeEach(() => {
    parser = new ClaudeCodeStreamParser();
});

const parse = (message: string): Promise<DocumentNode[]> => parser.parseEvent({ type: "user_message", message });
const types = (nodes: DocumentNode[]) => nodes.map((n) => n.type);

describe("splitSystemNotes", () => {
    test("leaves a message with no notes alone", () => {
        expect(splitSystemNotes("fix the build")).toEqual({ head: "fix the build", delay: null, notes: [] });
        expect(splitSystemNotes(JEKT)).toEqual({ head: JEKT, delay: null, notes: [] });
    });

    test("takes the notes off the end of a message, one per line", () => {
        const split = splitSystemNotes(`merge it\n\n${NOTE}\n${NOTE_2}`);
        expect(split.head).toBe("merge it");
        expect(split.notes).toEqual([NOTE.slice(11), NOTE_2.slice(11)]);
    });

    test("a turn of only notes has no head", () => {
        expect(splitSystemNotes(NOTE).head).toBe("");
        expect(splitSystemNotes(NOTE).notes).toHaveLength(1);
    });

    test("a note's own extra lines stay with it", () => {
        expect(splitSystemNotes(`${NOTE}\ncontinued here`).notes).toEqual([`${NOTE.slice(11)}\ncontinued here`]);
    });

    test("notes come after a jekt block, never from inside one", () => {
        expect(splitSystemNotes(`${JEKT}\n\n${NOTE}`).head).toBe(JEKT);
        const inside = JEKT.replace("Reply:", `\n\n${NOTE}\nReply:`);
        expect(splitSystemNotes(inside)).toEqual({ head: inside, delay: null, notes: [] });
    });

    test("text after a jekt block that is not a note is left as it was", () => {
        const text = `${JEKT}\n\nsomething else`;
        expect(splitSystemNotes(text)).toEqual({ head: text, delay: null, notes: [] });
    });

    test("the late-delivery prefix is srv's only in front of a jekt or a note", () => {
        expect(splitSystemNotes(`(sent 14:30, delivered 14:41) ${NOTE}`)).toMatchObject({
            head: "",
            delay: "sent 14:30, delivered 14:41",
        });
        expect(splitSystemNotes(`(sent 14:30, delivered 14:41) ${JEKT}`)).toMatchObject({
            head: JEKT,
            delay: "sent 14:30, delivered 14:41",
        });
        const typed = "(sent 14:30, delivered 14:41) hello";
        expect(splitSystemNotes(typed)).toEqual({ head: typed, delay: null, notes: [] });
    });

    test("a marker in the middle of a line is not a note", () => {
        const text = `see the [AgentMux] note about it\n\nand [AgentMux] again`;
        expect(splitSystemNotes(text)).toEqual({ head: text, delay: null, notes: [] });
    });
});

describe("a user turn that carries notes", () => {
    test("the operator's screenshot: a jekt followed by a note is a jekt bubble and a note, not the human", async () => {
        const nodes = await parse(`${JEKT}\n\n${NOTE}`);
        expect(types(nodes)).toEqual(["jekt_message", "system_note"]);
        expect((nodes[0] as JektMessageNode).from).toBe("agent1");
        expect((nodes[1] as SystemNoteNode).text).toBe(NOTE.slice(11));
    });

    test("a note on its own is a note", async () => {
        const nodes = await parse(NOTE);
        expect(types(nodes)).toEqual(["system_note"]);
    });

    test("the human's words keep their style and the note under them leaves it", async () => {
        const nodes = await parse(`merge on approval\n\n${NOTE}`);
        expect(types(nodes)).toEqual(["user_message", "system_note"]);
        expect((nodes[0] as UserMessageNode).message).toBe("merge on approval");
    });

    test("a late jekt, and a late note, are recognised through srv's prefix", async () => {
        expect(types(await parse(`(sent 14:30, delivered 14:41) ${JEKT}`))).toEqual(["jekt_message"]);
        const [note] = await parse(`(sent 14:30, delivered 14:41) ${NOTE}`);
        expect(note.type).toBe("system_note");
        expect((note as SystemNoteNode).delay).toBe("sent 14:30, delivered 14:41");
    });

    test("every note gets its own node, in order, with distinct ids", async () => {
        const nodes = await parse(`hello\n\n${NOTE}\n${NOTE_2}`);
        expect(types(nodes)).toEqual(["user_message", "system_note", "system_note"]);
        expect(new Set(nodes.map((n) => n.id)).size).toBe(3);
        expect((nodes[2] as SystemNoteNode).text).toContain("Agent5 also has");
    });

    test("a note that arrives while the agent is writing waits for the block to end, like a jekt", async () => {
        parser.parseStreamEvent({ type: "text", content: "Working on it" });
        const during = await parse(NOTE);
        expect(during).toEqual([]);
        const after = parser.flushPending();
        const released = parser.drainReleased();
        expect(types(after)).toEqual(["markdown"]);
        expect(types([...released])).toEqual(["system_note"]);
    });
});

/**
 * T0 (spec §10): the split can only narrow. Whatever a message is built from,
 * no piece of a jekt or a note is ever inside the human's node, and the human's
 * node holds only the human's own text.
 */
describe("T0: nothing reaches the human style through the split", () => {
    const HUMAN = "please look at the build";
    const pieces: Record<string, string> = { human: HUMAN, jekt: JEKT, note: NOTE };
    const orders = [
        ["jekt", "note"],
        ["note"],
        ["human", "note"],
        ["note", "note"],
        ["human"],
        ["jekt"],
        ["human", "note", "note"],
        ["note", "human"],
    ];

    for (const order of orders) {
        test(order.join(" + "), async () => {
            const message = order.map((p) => pieces[p]).join("\n\n");
            const nodes = await parse(message);
            for (const node of nodes) {
                if (node.type !== "user_message") continue;
                // The human style holds only the human's text, never a note or a jekt marker.
                expect(node.message).not.toContain("[AgentMux]");
                expect(node.message).not.toContain("[JEKT:");
                expect(node.message).not.toContain("[/JEKT]");
            }
            // And a human node exists only when there was a human piece in the message.
            const humanNodes = nodes.filter((n) => n.type === "user_message");
            if (!order.includes("human")) expect(humanNodes).toEqual([]);
        });
    }

    test("a note typed before human text (the note text first) is one container, not the human", async () => {
        const nodes = await parse(`${NOTE}\n\n${HUMAN}`);
        // The trailing line is a note's continuation, so it stays with the note; nothing here is the human's node.
        expect(nodes.filter((n) => n.type === "user_message")).toEqual([]);
    });
});
