// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit pane: one list, newest first, of every jekt, fleet and pane action, registration and agent start
// and end, read from `audit.query` a page at a time with filters, and below it what agents did with the
// person's secrets (vault-secrets.tsx). The list and how new entries reach it are audit-list.ts.

import { createEffect, createMemo, createSignal, For, on, onCleanup, Show, untrack, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";

import { Button, FilterInput, Select } from "@/app/element/ui";
import { VaultSecrets } from "@/app/view/vault/vault-secrets";
import { createAuditList } from "./audit-list";
import { AuditRow } from "./audit-row";
import {
    emptyFilter,
    KIND_CHIPS,
    newPillLabel,
    notRecordedNote,
    RANGES,
    totalsLabel,
    type AuditFilter,
    type AuditRange,
} from "./audit-shared";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./audit-manager.scss";

const TYPING_PAUSE_MS = 250;

export const AuditManager = (props: { paneBlockId: string; visible?: boolean; agent?: string }): JSX.Element => {
    const [filter, setFilter] = createSignal<AuditFilter>(emptyFilter(props.agent ?? ""));
    // What is typed in the two text boxes shows at once; the filter (and so a query, which makes srv rescan the
    // file for totals) follows once typing pauses.
    const [agentText, setAgentText] = createSignal(untrack(filter).agent);
    const [searchText, setSearchText] = createSignal(untrack(filter).text);
    let typing: ReturnType<typeof setTimeout> | undefined;
    const list = createAuditList(filter, () => props.visible ?? true);
    // The pane's scrolling body: the newest entries are at its top, under the filters.
    let body: HTMLDivElement | undefined;
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));
    const jumpToNewest = () => {
        list.showNewest();
        body?.scrollTo?.({ top: 0 });
    };

    // The pane's `audit:agent` changing (the Swarm's "Audit for this agent") sets the filter, which starts the list over.
    createEffect(
        on(
            () => props.agent,
            (agent) => {
                if (agent === undefined) return;
                setAgentText(agent);
                setFilter({ ...untrack(filter), agent });
            },
            { defer: true }
        )
    );
    const set = (patch: Partial<AuditFilter>) => setFilter({ ...filter(), ...patch });
    const typed = () => {
        if (typing !== undefined) clearTimeout(typing);
        typing = setTimeout(() => {
            typing = undefined;
            set({ agent: untrack(agentText), text: untrack(searchText) });
        }, TYPING_PAUSE_MS);
    };
    onCleanup(() => typing !== undefined && clearTimeout(typing));
    const toggleKind = (kind: string) => {
        const kinds = new Set(filter().kinds);
        if (!kinds.delete(kind)) kinds.add(kind);
        set({ kinds });
    };
    const empty = () => list.entries().length === 0;
    const unrecorded = () => list.notRecorded().filter((k) => filter().kinds.has(k));

    return (
        <div class="warden-manager-body" ref={body}>
            <p class="warden-manager-summary">Every jekt, fleet action, registration and agent start and end, most recent first</p>
            <div class="audit-filter-bar">
                <FilterInput
                    value={agentText()}
                    onInput={(agent) => {
                        setAgentText(agent);
                        typed();
                    }}
                    placeholder="Agent"
                    ariaLabel="Only entries of this agent"
                />
                <FilterInput
                    value={searchText()}
                    onInput={(text) => {
                        setSearchText(text);
                        typed();
                    }}
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
            <div class="audit-live-bar">
                <Button
                    tone="quiet"
                    density="compact"
                    icon={list.paused() ? "play" : "pause"}
                    pressed={list.paused()}
                    onClick={() => (list.paused() ? jumpToNewest() : list.setPaused(true))}
                >
                    {list.paused() ? "Resume" : "Pause"}
                </Button>
                <Show when={list.newCount() > 0}>
                    <Button tone="accent" density="compact" icon="arrow-up" onClick={jumpToNewest}>
                        {newPillLabel(list.newCount())}
                    </Button>
                </Show>
                <Show when={list.totals()}>{(t) => <span class="audit-totals">{totalsLabel(t())}</span>}</Show>
            </div>
            <Show when={unrecorded().length > 0}>
                <div class="warden-manager-summary">{notRecordedNote(unrecorded())}</div>
            </Show>
            <Show when={list.error()}>
                <div class="warden-manager-error">⚠ {list.error()}</div>
            </Show>
            <Show
                when={!empty()}
                fallback={<div class="warden-section-stub">{list.loading() ? "Loading…" : "No entries match."}</div>}
            >
                <ul class="audit-feed">
                    <For each={list.entries()}>{(entry) => <AuditRow entry={entry} now={now()} paneBlockId={props.paneBlockId} />}</For>
                </ul>
                <Show when={list.cursor()}>
                    <div class="audit-more">
                        <Button tone="quiet" density="compact" icon="arrow-down" busy={list.loadingMore()} onClick={() => void list.loadMore()}>
                            Show older
                        </Button>
                    </div>
                </Show>
            </Show>
            <VaultSecrets visible={props.visible} />
            <div class="warden-manager-footnote">New entries appear as they are written while you are at the newest ones.</div>
        </div>
    );
};

AuditManager.displayName = "AuditManager";
