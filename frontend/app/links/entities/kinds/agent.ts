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

export const agentKind: EntityKind = {
    id: "agent",
    priority: 10,
    recognise(text, ctx) {
        const found: Candidate[] = [];
        for (const m of text.matchAll(TOKEN)) {
            if (!ctx.agents.startsWith(m[0][0])) continue;
            const { written, computer } = splitComputer(m[0]);
            const name = trimToken(written);
            const agent = ctx.agents.find(name, computer !== undefined);
            if (!agent) continue;
            if (computer !== undefined ? !onComputer(agent, computer, ctx) : ctx.agents.neverAlone(name)) continue;
            const end = m.index + (computer !== undefined ? written.length + 1 + computer.length : name.length);
            if (followedByName(text, end)) continue;
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
