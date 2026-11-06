// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit pane's list: the pages read from `audit.query` for a filter, and how the newest entries reach it.
// At the newest entries it follows new ones as srv writes them; paused, or once older ones are loaded, the list
// holds still and the newest page is fetched on the side, so a pill can count what is new (audit-live.ts).

import { createEffect, createMemo, createSignal, on, untrack, type Accessor } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AuditEntry } from "@/types/rpc/AuditEntry";
import type { AuditQueryReq } from "@/types/rpc/AuditQueryReq";
import type { AuditQueryResult } from "@/types/rpc/AuditQueryResult";
import type { AuditTotals } from "@/types/rpc/AuditTotals";
import { serial, useAuditLive } from "./audit-live";
import { buildRequest, countNew, reuseRows, type AuditFilter } from "./audit-shared";

const fetchAuditPage = (req: AuditQueryReq): Promise<AuditQueryResult> => RpcApi.AuditQueryCommand(TabRpcClient, req);

/** What the list shows, and how a page of the newest entries replaces it. */
function createListState() {
    const [entries, setEntries] = createSignal<AuditEntry[]>([]);
    const [cursor, setCursor] = createSignal<string | undefined>();
    const [totals, setTotals] = createSignal<AuditTotals | null>(null);
    const [notRecorded, setNotRecorded] = createSignal<string[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(true);
    const [loadingMore, setLoadingMore] = createSignal(false);
    const [paused, setPaused] = createSignal(false);
    // Older entries are loaded: the list is no longer the newest page.
    const [olderLoaded, setOlderLoaded] = createSignal(false);
    // The newest page, fetched while the list holds still: what the pill counts and "newest" shows.
    const [pending, setPending] = createSignal<AuditQueryResult | null>(null);
    const following = () => !paused() && !olderLoaded();
    const newCount = createMemo(() => countNew(entries(), pending()?.entries ?? []));

    const showPage = (res: AuditQueryResult) => {
        setEntries(reuseRows(untrack(entries), res.entries));
        setCursor(res.next_cursor);
        setTotals(res.totals ?? null);
        setNotRecorded(res.not_recorded);
        setOlderLoaded(false);
        setPending(null);
        setError(null);
    };

    // The newest page, kept on the side: a query that worked clears the last one's error.
    const holdPage = (res: AuditQueryResult) => {
        setPending(res);
        setError(null);
    };

    return {
        entries, setEntries, cursor, setCursor, totals, notRecorded, error, setError, loading, setLoading,
        loadingMore, setLoadingMore, paused, setPaused, setOlderLoaded, pending, setPending, following, newCount, showPage, holdPage,
    };
}

export function createAuditList(filter: Accessor<AuditFilter>, visible: () => boolean) {
    const state = createListState();
    const { entries, setEntries, cursor, setCursor, error, setError, setLoading, loadingMore, setLoadingMore } = state;
    const { setPaused, setOlderLoaded, pending, setPending, following, showPage, holdPage } = state;

    let seq = 0; // the newest request wins: a slow answer for an earlier filter is dropped
    let peekSeq = 0; // the same for the newest page fetched while the list holds still
    let stale = false; // the filter changed while the pane was hidden: the list is the old one's until it is shown

    // `refreshing`: the page is only the newest entries again, so if the list was paused while it was on its way,
    // it is kept as the newest page the pill counts, not shown.
    const loadFirst = async (quiet = false, refreshing = false) => {
        const mine = ++seq;
        if (!quiet) setLoading(true);
        try {
            const res = await fetchAuditPage(buildRequest(untrack(filter), Date.now(), undefined, true));
            if (mine !== seq) return;
            if (refreshing && !untrack(following)) holdPage(res);
            else showPage(res);
        } catch (e) {
            if (mine === seq) setError(String(e));
        } finally {
            if (mine === seq) setLoading(false);
        }
    };

    // The list holds still (paused, or scrolled back): look at the newest page without showing it.
    const peek = async () => {
        const mine = ++peekSeq;
        try {
            const res = await fetchAuditPage(buildRequest(untrack(filter), Date.now(), undefined, true));
            if (mine === peekSeq && !untrack(following)) holdPage(res);
        } catch (e) {
            if (mine === peekSeq) setError(String(e));
        }
    };

    // Follow the newest entries again: show the page already fetched, or fetch it.
    const showNewest = () => {
        const page = untrack(pending);
        peekSeq++;
        setPaused(false);
        if (page) showPage(page);
        else void loadFirst(true);
    };

    const loadMore = async () => {
        const next = cursor();
        if (!next || loadingMore()) return;
        // Taking a number of its own drops a first-page refresh still in flight, which would overwrite what this adds.
        const mine = ++seq;
        setLoadingMore(true);
        try {
            const res = await fetchAuditPage(buildRequest(untrack(filter), Date.now(), next));
            if (mine !== seq) return;
            const seen = new Set(entries().map((e) => e.id));
            setEntries([...entries(), ...res.entries.filter((e) => !seen.has(e.id))]);
            setCursor(res.next_cursor);
            setOlderLoaded(true);
        } catch (e) {
            if (mine === seq) setError(String(e));
        } finally {
            setLoadingMore(false);
        }
    };

    // A new filter starts over.
    createEffect(
        on(
            filter,
            () => {
                // Whatever is loading, and the cursor of the old list, belongs to the old filter.
                seq++;
                peekSeq++;
                setCursor(undefined);
                setPending(null);
                setOlderLoaded(false);
                if (visible()) void loadFirst();
                else stale = true;
            },
            { defer: true }
        )
    );

    // New entries arrive as srv writes them while the pane is shown. At the newest entries the list takes them;
    // paused, or once older ones are loaded, it stays where it is and the pill counts them.
    const refresh = serial(async () => {
        if (untrack(loadingMore)) return;
        if (stale) {
            stale = false;
            return loadFirst(true);
        }
        return untrack(following) ? loadFirst(true, true) : peek();
    });
    useAuditLive(visible, () => void refresh());

    const { totals, notRecorded, loading, paused, newCount } = state;
    return { entries, cursor, totals, notRecorded, error, loading, loadingMore, paused, setPaused, newCount, loadMore, showNewest };
}
