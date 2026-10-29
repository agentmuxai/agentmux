// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm filter bar's matching, pure (docs/specs/SPEC_SWARM_FILTER_BAR_2026_10_29.md §2).
 *
 * One text box narrows the list by what a row shows: an agent's name, or the work it is on. Every
 * word of the query must appear (case-insensitive, no ranking, so the list keeps its order). A
 * first word `name:` or `work:` scopes the match to one of the two (`name:lark`, `work:login`).
 * Only what a row's tree node already holds is read: no block meta, no fetch.
 */

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { AgentTreeNode } from "./swarm-model";
import type { RemoteRow } from "./swarm-remote";

export type SwarmFilterScope = "both" | "name" | "work";

export interface SwarmQuery {
    /** Lower-cased words, none empty. No words means no filter. */
    words: readonly string[];
    scope: SwarmFilterScope;
}

/** The meta key of the pane's filter text (written by the bar, like `swarm:groupby`). */
export const FILTER_META_KEY = "swarm:filter";

export const NO_FILTER: SwarmQuery = { words: [], scope: "both" };

const PREFIXES: Record<string, SwarmFilterScope> = { "name:": "name", "work:": "work" };

/** `raw` as typed. A first word `name:` or `work:` (any case) sets the scope; what follows it, in
 *  that word or after it, is the query. A prefix with nothing after it filters nothing yet. */
export function parseQuery(raw: string | null | undefined): SwarmQuery {
    const text = (raw ?? "").trim().toLowerCase();
    if (!text) return NO_FILTER;
    let scope: SwarmFilterScope = "both";
    let rest = text;
    for (const [prefix, s] of Object.entries(PREFIXES)) {
        if (text.startsWith(prefix)) {
            scope = s;
            rest = text.slice(prefix.length);
            break;
        }
    }
    const words = rest.split(/\s+/).filter(Boolean);
    return words.length ? { words, scope } : NO_FILTER;
}

export function isFiltering(query: SwarmQuery): boolean {
    return query.words.length > 0;
}

/** True when every word of `query` is in the name (scope `name`), the work (`work`), or either (`both`). */
function matches(query: SwarmQuery, name: string, work: readonly (string | null | undefined)[]): boolean {
    if (!isFiltering(query)) return true;
    const names = name.toLowerCase();
    const works = work.filter((t): t is string => !!t).map((t) => t.toLowerCase()).join("\n");
    const haystack = query.scope === "name" ? names : query.scope === "work" ? works : `${names}\n${works}`;
    return query.words.every((w) => haystack.includes(w));
}

/** The text of a local agent's row that says what it is working on: its line, its in-progress
 *  todo, the tool in flight, and the label of each running subagent, workflow, shell and cron row. */
export function workTextsOf(node: AgentTreeNode): string[] {
    const texts: (string | null | undefined)[] = [node.line?.text, node.currentTool];
    for (const todo of node.todoRows ?? []) if (todo.status === "in_progress") texts.push(todo.text);
    for (const s of node.agentToolRows ?? []) texts.push(s.display_name ?? s.slug);
    for (const w of node.workflowRows ?? []) texts.push(w.name);
    for (const s of node.shellRows ?? []) texts.push(s.title);
    for (const c of node.cronRows ?? []) texts.push(c.name);
    return texts.filter((t): t is string => !!t);
}

/** Whether a local agent's row matches. Sub-rows are part of their agent: a match in one shows it. */
export function nodeMatches(query: SwarmQuery, node: AgentTreeNode): boolean {
    return matches(query, node.agentName, workTextsOf(node));
}

/** Whether an agent of another instance matches: its name and the line its instance shares. */
export function remoteMatches(query: SwarmQuery, row: RemoteRow): boolean {
    return matches(query, row.agent.name, [row.agent.line]);
}

/** Whether a created agent with no pane matches: its name only, so a `work:` query shows none. */
export function stoppedMatches(query: SwarmQuery, agent: Pick<AgentRuntime, "name">): boolean {
    return matches(query, agent.name, []);
}

/** `text` in pieces, each marked when it is a word of `query` (for the highlight). Overlapping and
 *  repeated matches merge; the pieces always join back into `text`. Pure. */
export function splitMatches(text: string, words: readonly string[]): { text: string; match: boolean }[] {
    if (!text || words.length === 0) return [{ text, match: false }];
    const lower = text.toLowerCase();
    const marked = new Array<boolean>(text.length).fill(false);
    for (const word of words) {
        for (let at = lower.indexOf(word); word && at !== -1; at = lower.indexOf(word, at + 1)) {
            for (let i = at; i < at + word.length; i++) marked[i] = true;
        }
    }
    const pieces: { text: string; match: boolean }[] = [];
    let start = 0;
    for (let i = 1; i <= text.length; i++) {
        if (i === text.length || marked[i] !== marked[start]) {
            pieces.push({ text: text.slice(start, i), match: marked[start] });
            start = i;
        }
    }
    return pieces;
}

/**
 * Which rows are shown, with the membership rule that keeps a list from changing under the cursor
 * (spec §3): a row already shown that stops matching because of its own update stays until the
 * query itself changes; a row that starts matching appears at once, and a change the person makes
 * (typing, clearing) re-evaluates every row. Holds the set it last returned.
 */
export class FilterMembership {
    private shown = new Set<string>();
    private last: SwarmQuery = NO_FILTER;

    /** The keys of `keys` to show, in order. `matchOf` says whether a key's row matches now. */
    update(query: SwarmQuery, keys: readonly string[], matchOf: (key: string) => boolean): string[] {
        if (!isFiltering(query)) {
            this.shown.clear();
            this.last = query;
            return keys.slice();
        }
        const same = sameQuery(this.last, query);
        const next = new Set<string>();
        for (const key of keys) {
            if (matchOf(key) || (same && this.shown.has(key))) next.add(key);
        }
        this.shown = next;
        this.last = query;
        return keys.filter((k) => next.has(k));
    }
}

function sameQuery(a: SwarmQuery, b: SwarmQuery): boolean {
    return a.scope === b.scope && a.words.length === b.words.length && a.words.every((w, i) => w === b.words[i]);
}
