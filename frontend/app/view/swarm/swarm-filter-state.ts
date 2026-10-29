// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Swarm filter's state, one per Swarm pane (docs/specs/SPEC_SWARM_FILTER_BAR_2026_10_29.md).
 *
 * The box shows what was typed at once; the list applies it 150 ms after the last keystroke; the
 * pane's meta (`swarm:filter`) takes it 500 ms after, so it survives a restart. The shown rows are
 * a pure function of the rows the list already keeps: no tree is rebuilt, and with no filter the
 * match reads nothing at all.
 *
 * What the rest of the pane needs from it is published per model: the keys the filter hides, so a
 * fleet action reaches the selected and shown agents only, and the words to highlight.
 */

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { batch, createComputed, createMemo, createSignal, onCleanup, type Accessor } from "solid-js";
import {
    FILTER_META_KEY,
    FilterMembership,
    isFiltering,
    nodeMatches,
    parseQuery,
    remoteMatches,
    stoppedMatches,
    type SwarmQuery,
} from "./swarm-filter";
import { remoteAgentKey } from "./swarm-fleet-targets";
import type { AgentTreeNode } from "./swarm-model";
import type { RemoteRow } from "./swarm-remote";

/** How long after the last keystroke the list applies the filter, and the pane's meta takes it. */
export const FILTER_APPLY_MS = 150;
export const FILTER_PERSIST_MS = 500;

export interface SwarmFilterState {
    /** What the box shows. */
    text: Accessor<string>;
    setText(text: string): void;
    clear(): void;
    /** The query the list applies. */
    query: Accessor<SwarmQuery>;
    /** The keys of the shown rows (this instance's and the other instances'), or null with no filter. */
    shown: Accessor<ReadonlySet<string> | null>;
    /** Created agents with no pane that match. */
    stopped: Accessor<AgentRuntime[]>;
    /** Rows shown and rows listed, the stopped ones counted in both. */
    counts: Accessor<{ shown: number; total: number }>;
}

/** What publishes the filter to the rest of the pane, keyed by the pane's model. */
interface Published {
    hidden: ReturnType<typeof createSignal<ReadonlySet<string>>>;
    query: ReturnType<typeof createSignal<SwarmQuery>>;
}
const published = new WeakMap<object, Published>();
const EMPTY: ReadonlySet<string> = new Set();
const NONE: SwarmQuery = { words: [], scope: "both" };

function publishedOf(model: object): Published {
    let p = published.get(model);
    if (!p) {
        p = {
            hidden: createSignal<ReadonlySet<string>>(EMPTY, { equals: sameSet }),
            query: createSignal<SwarmQuery>(NONE),
        };
        published.set(model, p);
    }
    return p;
}

function sameSet(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
    if (a === b) return true;
    if (a.size !== b.size) return false;
    for (const k of a) if (!b.has(k)) return false;
    return true;
}

/** The keys the filter hides now (empty with no filter): reactive. A selected agent among them stays
 *  selected, but no fleet action reaches it while it is hidden. */
export function hiddenByFilter(model: object): ReadonlySet<string> {
    return publishedOf(model).hidden[0]();
}

/** The words to highlight in `field` of a row (empty when none, or when the query is scoped to the
 *  other field): reactive. */
export function highlightWords(model: object, field: "name" | "work"): readonly string[] {
    const q = publishedOf(model).query[0]();
    return q.scope === "both" || q.scope === field ? q.words : [];
}

/** Whether a filter narrows this pane's list now: reactive. */
export function filterActive(model: object): boolean {
    return publishedOf(model).query[0]().words.length > 0;
}

/** `selected` without the agents the filter hides: the ones an action reaches. */
export function actionableKeys(model: object, selected: Iterable<string>): string[] {
    const hidden = hiddenByFilter(model);
    return [...selected].filter((key) => !hidden.has(key));
}

export function useSwarmFilter(
    model: object,
    ctx: Pick<PaneTabHostContext, "meta" | "setMeta">,
    rows: {
        keyedRows: Accessor<readonly (readonly [string, AgentTreeNode])[]>;
        remote: Accessor<readonly RemoteRow[]>;
        stopped: Accessor<readonly AgentRuntime[]>;
    }
): SwarmFilterState {
    const initial = ctx.meta()?.[FILTER_META_KEY];
    const [text, setTextSignal] = createSignal(typeof initial === "string" ? initial : "");
    const [query, setQuery] = createSignal<SwarmQuery>(parseQuery(typeof initial === "string" ? initial : ""));
    let applyTimer: ReturnType<typeof setTimeout> | undefined;
    let persistTimer: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => {
        clearTimeout(applyTimer);
        clearTimeout(persistTimer);
    });

    const persist = (value: string) => {
        clearTimeout(persistTimer);
        persistTimer = setTimeout(() => void ctx.setMeta({ [FILTER_META_KEY]: value || null }), FILTER_PERSIST_MS);
    };
    const setText = (value: string) => {
        setTextSignal(value);
        clearTimeout(applyTimer);
        // Clearing is applied at once: nothing to wait for when the person empties the box.
        if (!value.trim()) setQuery(parseQuery(""));
        else applyTimer = setTimeout(() => setQuery(parseQuery(value)), FILTER_APPLY_MS);
        persist(value);
    };
    const clear = () => batch(() => setText(""));

    const membership = new FilterMembership();
    const shown = createMemo<ReadonlySet<string> | null>(() => {
        const q = query();
        // No filter: nothing else is read, so a Swarm with no filter has no new subscription.
        if (!isFiltering(q)) {
            membership.update(q, [], () => false);
            return null;
        }
        const local = new Map(rows.keyedRows());
        const remote = new Map(rows.remote().map((r) => [remoteAgentKey(r.section, r.agent), r] as const));
        const keys = [...local.keys(), ...remote.keys()];
        const matchOf = (key: string) => {
            const node = local.get(key);
            return node ? nodeMatches(q, node) : remoteMatches(q, remote.get(key)!);
        };
        return new Set(membership.update(q, keys, matchOf));
    });
    const stopped = createMemo(() => {
        const q = query();
        return isFiltering(q) ? rows.stopped().filter((a) => stoppedMatches(q, a)) : [...rows.stopped()];
    });
    // Not a memo: only the bar reads it, and only while a filter is on, so no row list is
    // subscribed to for it otherwise.
    const counts = () => {
        const set = shown();
        const total = rows.keyedRows().length + rows.remote().length + rows.stopped().length;
        return { shown: set ? set.size + stopped().length : total, total };
    };

    // Publish to the rest of the pane (the toolbar, Broadcast, the fleet actions, the rows).
    const pub = publishedOf(model);
    createComputed(() => pub.query[1](query()));
    createComputed(() => {
        const set = shown();
        if (!set) return pub.hidden[1](EMPTY);
        const hidden = new Set<string>();
        for (const [key] of rows.keyedRows()) if (!set.has(key)) hidden.add(key);
        for (const r of rows.remote()) {
            const key = remoteAgentKey(r.section, r.agent);
            if (!set.has(key)) hidden.add(key);
        }
        pub.hidden[1](hidden);
    });
    onCleanup(() => published.delete(model));

    return { text, setText, clear, query, shown, stopped, counts };
}
