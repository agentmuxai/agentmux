// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Agent names (spec §3, D3). A name links only when it is in the roster, written whole as the roster shows it (its
 * display name with the capital, or its slug or id when that is the name's own word or carries a digit; an id that is
 * another plain word links only with its computer), and not an ordinary word standing alone. `Name@computer` links
 * only when that agent is on that computer, so an email address is never a link.
 */

import { openAgentEntity } from "../openers";
import type { AgentEntry, Candidate, EntityKind, LinkContext } from "../types";

const TOKEN = /(?<![\w@./-])[A-Za-z][A-Za-z0-9_-]*(?:@[A-Za-z0-9][A-Za-z0-9._-]*)?/g;

function trimToken(token: string): string {
    // A hyphen or an underscore at the end is punctuation of the sentence, not of the name.
    return token.replace(/[-_]+$/, "");
}

function splitComputer(token: string): { written: string; computer?: string } {
    const at = token.indexOf("@");
    if (at < 0) return { written: token };
    return { written: token.slice(0, at), computer: token.slice(at + 1).replace(/[._-]+$/, "") };
}

function onComputer(agent: AgentEntry, computer: string, ctx: LinkContext): boolean {
    const there = (agent.computer ?? ctx.agents.localComputer ?? "").toLowerCase();
    return there !== "" && there === computer.toLowerCase();
}

function followedByName(text: string, end: number): boolean {
    // `Name.md`, `Name/x` and `Name@` are not the agent: a file, a path, an address.
    return /^(?:\.[A-Za-z0-9]|\/|@)/.test(text.slice(end, end + 2));
}

/** `Name #3`: a name followed by a number sign and a number, the way a numbered agent is written. */
const NUMBER_AFTER = /^ #\d+(?!\w)/;

const HOST = /^@([A-Za-z0-9][A-Za-z0-9._-]*)/;

export const agentKind: EntityKind = {
    id: "agent",
    priority: 10,
    recognise(text, ctx) {
        const found: Candidate[] = [];
        let consumed = 0;
        for (const m of text.matchAll(TOKEN)) {
            // The rest of a name already linked, such as the `X` of `Agent X`, is not looked up on its own.
            if (m.index < consumed || !ctx.agents.startsWith(m[0][0])) continue;
            const whole = ctx.agents.longestName(text, m.index);
            let name: string;
            let computer: string | undefined;
            let end: number;
            if (whole) {
                name = whole;
                const host = HOST.exec(text.slice(m.index + whole.length, m.index + whole.length + 80));
                computer = host?.[1].replace(/[._-]+$/, "");
                end = m.index + whole.length + (computer !== undefined ? 1 + computer.length : 0);
            } else {
                const split = splitComputer(m[0]);
                name = trimToken(split.written);
                computer = split.computer;
                end = m.index + (computer !== undefined ? split.written.length + 1 + computer.length : name.length);
            }
            // `Loap #4` is a numbered agent's name when some `Loap #N` exists: `Loap` alone is not who is meant.
            if (computer === undefined && NUMBER_AFTER.test(text.slice(end, end + 12)) && ctx.agents.isNumberedFamily(name)) {
                consumed = end;
                continue;
            }
            const agent = ctx.agents.find(name, computer !== undefined);
            if (!agent) continue;
            if (computer !== undefined ? !onComputer(agent, computer, ctx) : ctx.agents.neverAlone(name)) continue;
            if (followedByName(text, end)) continue;
            consumed = end;
            found.push({ kind: "agent", start: m.index, end, data: { id: agent.id, name: agent.name || agent.id, computer: computer ?? "" } });
        }
        return found;
    },
    resolve(candidate) {
        const { id, name, computer } = candidate.data;
        return { destination: { kind: "agent", id, name, ...(computer ? { computer } : {}) }, state: "ready", name: `Open agent ${name}` };
    },
    open(destination) {
        if (destination.kind !== "agent") return Promise.resolve({ ok: false, reason: "not an agent" });
        return openAgentEntity(destination);
    },
};
