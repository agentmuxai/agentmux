// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Audit pane: one list, newest first, of every jekt, fleet and pane action, registration and agent start
// and end, read from `audit.query` a page at a time with filters, and below it what agents did with the
// person's secrets (vault-secrets.tsx). The list and how new entries reach it are audit-list.ts.

import { createEffect, createMemo, createSignal, on, onCleanup, Show, untrack, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";

import { Button, FilterInput, Select, SegmentedControl } from "@/app/element/ui";
import { VaultSecrets } from "@/app/view/vault/vault-secrets";
import { AuditFeed } from "./audit-feed";
import { AuditKindChips } from "./audit-kind-chips";
import { AuditLiveBar } from "./audit-live-bar";
import { createAuditList } from "./audit-list";
import { createAuditSounds } from "./audit-sound";
import {
    emptyFilter,
    notRecordedNote,
    RANGES,
    SCOPES,
    type AuditFilter,
    type AuditRange,
    type AuditScope,
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
    let body: HTMLDivElement | undefined; // the pane's scrolling body: the newest entries are at its top
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));
    const sounds = createAuditSounds(list.entries, now);
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
                <SegmentedControl
                    options={SCOPES}
                    value={filter().scope}
                    onChange={(scope) => set({ scope: scope as AuditScope })}
                    density="compact"
                    ariaLabel="Whose entries"
                />
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
            <AuditKindChips kinds={filter().kinds} onToggle={toggleKind} />
            <AuditLiveBar
                paused={list.paused()}
                onTogglePause={() => (list.paused() ? jumpToNewest() : list.setPaused(true))}
                newCount={list.newCount()}
                onJumpToNewest={jumpToNewest}
                totals={list.totals()}
                lastSound={sounds.last()}
            />
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
                <AuditFeed
                    entries={list.entries()}
                    sectioned={filter().scope === "machine"}
                    channel={list.channel()}
                    now={now()}
                    paneBlockId={props.paneBlockId}
                    sounds={sounds}
                />
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
