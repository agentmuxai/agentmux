// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Turns the CLI wrote itself. Claude Code puts user-role turns into the conversation that no person
// typed: a background task's `<task-notification>` (a Monitor event too), a `<system-reminder>`, a
// local command's output, a hook's output. They are automated, so they never take the human's
// style (docs/specs/SPEC_MESSAGE_ORIGIN_AND_ONE_CONTAINER_2026_10_25.md §2.1 row 17, §7 "CLI message").
//
// INTERIM text parsing, like system-notes.ts, until the origin model types these turns (P5 deletes
// it). It can only narrow: a turn is a CLI turn only when the whole payload is wrapper blocks, so
// a person's message that mentions a tag, or has any text outside the blocks, stays the person's.

import type { DocumentNode, SystemNoteNode } from "./types";

const WRAPPER_TAGS = [
    "task-notification",
    "system-reminder",
    "local-command-stdout",
    "local-command-stderr",
    "local-command-caveat",
    "user-prompt-submit-hook",
] as const;

type WrapperTag = (typeof WRAPPER_TAGS)[number];

interface WrapperBlock {
    tag: WrapperTag;
    body: string;
}

const OPEN_RE = new RegExp(`^<(${WRAPPER_TAGS.join("|")})>`);

/** The blocks if `message` is nothing but wrapper blocks (and whitespace between them), else null. */
function wrapperBlocks(message: string): WrapperBlock[] | null {
    const blocks: WrapperBlock[] = [];
    let rest = message.trim();
    while (rest.length > 0) {
        const open = OPEN_RE.exec(rest);
        if (!open) return null;
        const tag = open[1] as WrapperTag;
        const close = `</${tag}>`;
        const end = rest.indexOf(close, open[0].length);
        if (end < 0) return null;
        blocks.push({ tag, body: rest.slice(open[0].length, end) });
        rest = rest.slice(end + close.length).trimStart();
    }
    return blocks.length > 0 ? blocks : null;
}

function inner(body: string, tag: string): string | undefined {
    const m = new RegExp(`<${tag}>([\\s\\S]*?)</${tag}>`).exec(body);
    return m?.[1].trim() || undefined;
}

/** What one block says: a task notification's summary and event, else the block's own text. */
function blockText(block: WrapperBlock): string {
    if (block.tag === "task-notification") {
        const parts = [inner(block.body, "summary"), inner(block.body, "event")].filter((p): p is string => !!p);
        if (parts.length > 0) return parts.join("\n");
        // The dock's form, with a tool-use-id and a status and nothing to read.
        const status = inner(block.body, "status");
        return status ? `Background task ${status}` : "Background task update";
    }
    return block.body.trim();
}

/** A note made from a CLI turn: it starts a turn like the user message it replaces. */
export function isCliTurn(node: DocumentNode): node is SystemNoteNode {
    return node.type === "system_note" && node.source === "cli";
}

/**
 * A note that starts a turn, as a user message does: a CLI turn, an "Earlier message" (a record
 * from before origins, which was a user message), or a message from a signed origin other than
 * AgentMux's own (a schedule, a task, the Supervisor, a service). AgentMux's own notes ride after a
 * message and never start one.
 */
export function isTurnNote(node: DocumentNode): node is SystemNoteNode {
    return node.type === "system_note" && (node.source === "cli" || node.source === "earlier" || (node.origin !== undefined && node.origin.kind !== "system"));
}

/** A user turn, or the CLI turn that replaced one: its text and time, for the readers of task notifications. */
export function injectedTurn(node: DocumentNode): { text: string; timestamp?: number } | null {
    if (node.type === "user_message") return { text: node.message, timestamp: node.timestamp };
    if (node.type === "system_note" && node.source === "cli" && node.raw !== undefined) return { text: node.raw, timestamp: node.timestamp };
    return null;
}

/**
 * The text to show for a turn made entirely of CLI wrapper blocks, or null when it is anything
 * else (the human's, a jekt, a note...). Blocks with no text of their own are left out of the text.
 */
export function parseCliTurn(message: string): { text: string } | null {
    if (!message.includes("<")) return null;
    const blocks = wrapperBlocks(message);
    if (!blocks) return null;
    const text = blocks
        .map(blockText)
        .filter((t) => t !== "")
        .join("\n");
    return { text };
}
