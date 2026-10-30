// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit pane: one list, newest first, of every jekt, fleet and pane action, registration and agent start
// and end, read from `audit.query` a page at a time with filters, and below it what agents did with the
// person's secrets (vault-secrets.tsx).

import { createEffect, createMemo, createSignal, For, on, Show, untrack, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";

import { Button, FilterInput, Select } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { VaultSecrets } from "@/app/view/vault/vault-secrets";
import { pollWhileVisible, WARDEN_REFRESH_MS } from "@/app/view/warden-shared/warden-shared";
import type { AuditEntry } from "@/types/rpc/AuditEntry";
import type { AuditQueryReq } from "@/types/rpc/AuditQueryReq";
import type { AuditQueryResult } from "@/types/rpc/AuditQueryResult";
import type { AuditTotals } from "@/types/rpc/AuditTotals";
import { AuditRow } from "./audit-row";
import {
    AUDIT_PAGE,
    buildRequest,
    emptyFilter,
    KIND_CHIPS,
    notRecordedNote,
    RANGES,
    totalsLabel,
    type AuditFilter,
    type AuditRange,
} from "./audit-shared";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./audit-manager.scss";

const fetchAuditPage = (req: AuditQueryReq): Promise<AuditQueryResult> => RpcApi.AuditQueryCommand(TabRpcClient, req);

export const AuditManager = (props: { paneBlockId: string; visible?: boolean; agent?: string }): JSX.Element => {
    const [filter, setFilter] = createSignal<AuditFilter>(emptyFilter(props.agent ?? ""));
    const [entries, setEntries] = createSignal<AuditEntry[]>([]);
    const [cursor, setCursor] = createSignal<string | undefined>();
    const [totals, setTotals] = createSignal<AuditTotals | null>(null);
    const [notRecorded, setNotRecorded] = createSignal<string[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(true);
    const [loadingMore, setLoadingMore] = createSignal(false);
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));

    // The newest request wins: a slow answer for an earlier filter is dropped.
    let seq = 0;
    // The filter changed while the pane was hidden: the list shown is for the old one until it is shown again.
    let stale = false;
    // A first-page request in flight: a periodic refresh waits for it, so a slow answer isn't superseded every
    // five seconds and never shown. A filter change still supersedes it.
    let refreshing = false;

    const loadFirst = async (quiet = false) => {
        const mine = ++seq;
        refreshing = true;
        if (!quiet) setLoading(true);
        try {
            const res = await fetchAuditPage(buildRequest(untrack(filter), Date.now(), undefined, true));
            if (mine !== seq) return;
            setEntries(res.entries);
            setCursor(res.next_cursor);
            setTotals(res.totals ?? null);
            setNotRecorded(res.not_recorded);
            setError(null);
        } catch (e) {
            if (mine === seq) setError(String(e));
        } finally {
            if (mine === seq) {
                setLoading(false);
                refreshing = false;
            }
        }
    };

    const loadMore = async () => {
        const next = cursor();
        if (!next || loadingMore()) return;
        // Taking a number of its own drops a first-page refresh still in flight, which would overwrite what this adds.
        const mine = ++seq;
        refreshing = false;
        setLoadingMore(true);
        try {
            const res = await fetchAuditPage(buildRequest(untrack(filter), Date.now(), next));
            if (mine !== seq) return;
            const seen = new Set(entries().map((e) => e.id));
            setEntries([...entries(), ...res.entries.filter((e) => !seen.has(e.id))]);
            setCursor(res.next_cursor);
        } catch (e) {
            if (mine === seq) setError(String(e));
        } finally {
            setLoadingMore(false);
        }
    };

    // A new filter, or the pane's `audit:agent` changing (the Swarm's "Audit for this agent"), starts over.
    createEffect(
        on(
            () => props.agent,
            (agent) => {
                if (agent !== undefined) setFilter({ ...untrack(filter), agent });
            },
            { defer: true }
        )
    );
    createEffect(
        on(
            filter,
            () => {
                // Whatever is loading, and the cursor of the old list, belongs to the old filter.
                seq++;
                refreshing = false;
                setCursor(undefined);
                if (props.visible ?? true) void loadFirst();
                else stale = true;
            },
            { defer: true }
        )
    );

    // Newest entries arrive on their own while the pane is shown and nothing older has been loaded; once
    // you have scrolled back, the list stays where it is until the filter changes.
    pollWhileVisible(
        () => {
            if (untrack(loadingMore) || refreshing) return;
            if (stale || untrack(entries).length <= AUDIT_PAGE) {
                stale = false;
                void loadFirst(true);
            }
        },
        () => props.visible ?? true
    );

    const set = (patch: Partial<AuditFilter>) => setFilter({ ...filter(), ...patch });
    const toggleKind = (kind: string) => {
        const kinds = new Set(filter().kinds);
        if (!kinds.delete(kind)) kinds.add(kind);
        set({ kinds });
    };
    const empty = () => entries().length === 0;
    const unrecorded = () => notRecorded().filter((k) => filter().kinds.has(k));

    return (
        <div class="warden-manager-body">
            <p class="warden-manager-summary">Every jekt, fleet action, registration and agent start and end, most recent first</p>
            <div class="audit-filter-bar">
                <FilterInput
                    value={filter().agent}
                    onInput={(agent) => set({ agent })}
                    placeholder="Agent"
                    ariaLabel="Only entries of this agent"
                />
                <FilterInput
                    value={filter().text}
                    onInput={(text) => set({ text })}
                    placeholder="Search entries"
                    ariaLabel="Search the entries' summary, agents and reason"
                />
                <Select
                    value={filter().range}
                    onChange={(range) => set({ range: range as AuditRange })}
                    options={RANGES.map((r) => ({ value: r.value, label: r.label }))}
                    density="compact"
                    ariaLabel="How far back"
                />
            </div>
            <div class="audit-kind-chips" role="group" aria-label="Kinds of entry">
                <For each={KIND_CHIPS}>
                    {(chip) => (
                        <Button
                            tone="quiet"
                            density="compact"
                            pressed={filter().kinds.has(chip.kind)}
                            onClick={() => toggleKind(chip.kind)}
                        >
                            {chip.label}
                        </Button>
                    )}
                </For>
            </div>
            <Show when={totals()}>{(t) => <div class="audit-totals">{totalsLabel(t())}</div>}</Show>
            <Show when={unrecorded().length > 0}>
                <div class="warden-manager-summary">{notRecordedNote(unrecorded())}</div>
            </Show>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show
                when={!empty()}
                fallback={<div class="warden-section-stub">{loading() ? "Loading…" : "No entries match."}</div>}
            >
                <ul class="audit-feed">
                    <For each={entries()}>{(entry) => <AuditRow entry={entry} now={now()} paneBlockId={props.paneBlockId} />}</For>
                </ul>
                <Show when={cursor()}>
                    <div class="audit-more">
                        <Button tone="quiet" density="compact" icon="arrow-down" busy={loadingMore()} onClick={() => void loadMore()}>
                            Show older
                        </Button>
                    </div>
                </Show>
            </Show>
            <VaultSecrets visible={props.visible} />
            <div class="warden-manager-footnote">Refreshes every {WARDEN_REFRESH_MS / 1000}s while you are at the newest entries.</div>
        </div>
    );
};

AuditManager.displayName = "AuditManager";
