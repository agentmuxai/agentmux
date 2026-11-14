// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A jekt asked for by its id: the Messages panel's "Open in Audit" names the message, and the Audit shows its row.
// The row is the one whose `provenance.msg_id` is the marker's MSGID (docs/specs/SPEC_JEKT_SIDE_PANEL_RESEARCH_AND_DESIGN_2026_11_04.md §5.9).

import type { AuditEntry } from "@/types/rpc/AuditEntry";

/** A message as the Audit pane's `audit:message` meta carries it: "at|ask|msgid". `ask` is the time of the ask, so
 *  asking for the same message again changes the meta and the Audit looks again. The id is last: it is free text. */
export interface MessageRef {
    msgId: string;
    /** When the message reached its pane: how far back the list has to reach to hold it. */
    at: number;
    asked: number;
}

export function encodeMessageRef(msgId: string, at: number, asked = Date.now()): string {
    return [at, asked, msgId].join("|");
}

export function parseMessageRef(raw: unknown): MessageRef | undefined {
    if (typeof raw !== "string") return undefined;
    const [at, asked, ...id] = raw.split("|");
    const msgId = id.join("|");
    if (!msgId || !Number.isFinite(Number(at))) return undefined;
    return { msgId, at: Number(at), asked: Number(asked) || 0 };
}

/** The row of the message: a jekt entry carrying its id. */
export function rowOfMessage(entries: readonly AuditEntry[], msgId: string): AuditEntry | undefined {
    return entries.find((e) => e.kind === "jekt" && e.provenance?.msg_id === msgId);
}
