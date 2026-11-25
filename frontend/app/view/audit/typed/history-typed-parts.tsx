// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Typed pane's markup in parts (history-typed.tsx): the bar, the entries (or why there are none), the list of
// what was forgotten, and the footnote. Each reads the pane's state (history-typed-state.ts).

import { For, Show, type JSX } from "solid-js";

import { Button, FilterInput, Select } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";
import type { InputHistoryItem } from "@/types/rpc/InputHistoryItem";
import { formatWhen, typedTags } from "../history-shared";
import type { HistoryTypedState } from "./history-typed-state";

type Part = { t: HistoryTypedState };

/** The agent, the search, Refresh, and Clear… (which asks again by naming the agent). */
export function TypedBar(props: Part): JSX.Element {
    const st = props.t;
    return (
        <div class="warden-history-bar">
            <Select
                class="warden-history-select"
                ariaLabel={t("history.typed.agent")}
                density="compact"
                value={st.agentId()}
                onChange={st.setAgentId}
                disabled={st.list().length === 0}
                options={st.list().map((a) => ({ value: a.id, label: a.name }))}
            />
            <FilterInput class="warden-history-filter" value={st.query()} onInput={st.setQuery} placeholder={t("history.typed.search")} />
            <Button
                tone="quiet"
                density="compact"
                icon="rotate"
                busy={st.loading()}
                onClick={() => {
                    void st.loadAgents();
                    void st.read(false);
                    void st.readTrash();
                }}
            >
                {t("history.typed.refresh")}
            </Button>
            <Show when={st.entries().length > 0 && !st.query()}>
                <Show
                    when={st.confirmClear()}
                    fallback={
                        <Button tone="quiet" density="compact" onClick={() => st.setConfirmClear(true)}>
                            {t("history.typed.clear")}
                        </Button>
                    }
                >
                    <Button tone="danger" density="compact" onClick={() => st.clearAll()}>
                        {t("history.typed.forgetAll", { name: st.agentName() })}
                    </Button>
                    <Button tone="quiet" density="compact" onClick={() => st.setConfirmClear(false)}>
                        {t("history.typed.cancel")}
                    </Button>
                </Show>
            </Show>
        </div>
    );
}

/** How an entry was typed, and how many times in a row it was sent. */
const tags = (item: InputHistoryItem): string =>
    [...typedTags(item.kind, item.source), ...(item.sends > 1 ? [t("history.typed.sent", { count: item.sends })] : [])].join(" · ");

/** One entry, with Forget (which asks again). */
function EntryRow(props: Part & { item: InputHistoryItem }): JSX.Element {
    const { t: st, item } = props;
    return (
        <tr data-entry={item.id}>
            <td class="warden-history-typed-when warden-history-cell-wide" data-label={t("history.typed.when")}>{formatWhen(item.sent_at_ms)}</td>
            <td class="warden-history-cell-wide">
                <div class="warden-history-typed-text" {...tip(item.text)}>
                    {item.text}
                </div>
                <Show when={tags(item)}>
                    <div class="warden-manager-dim warden-history-typed-tags">{tags(item)}</div>
                </Show>
            </td>
            <td class="warden-history-actions warden-history-cell-wide">
                <Show
                    when={st.confirmId() === item.id}
                    fallback={
                        <Button tone="quiet" density="compact" onClick={() => st.setConfirmId(item.id)}>
                            {t("history.typed.forget")}
                        </Button>
                    }
                >
                    <Button tone="danger" density="compact" onClick={() => st.forget(item)}>
                        {t("history.typed.forgetThis")}
                    </Button>
                    <Button tone="quiet" density="compact" onClick={() => st.setConfirmId(null)}>
                        {t("history.typed.cancel")}
                    </Button>
                </Show>
            </td>
        </tr>
    );
}

/** The entries, newest first, with Show older; or why there are none. */
export function TypedEntries(props: Part): JSX.Element {
    const st = props.t;
    return (
        <Show
            when={st.entries().length > 0}
            fallback={
                <div class="warden-section-stub">
                    {st.agents() === null || st.loading()
                        ? t("history.typed.loading")
                        : st.list().length === 0
                          ? t("history.typed.noAgent")
                          : st.query()
                            ? t("history.typed.noMatch")
                            : t("history.typed.none")}
                </div>
            }
        >
            <table class="warden-manager-table warden-history-table warden-history-typed-table">
                <thead>
                    <tr>
                        <th>{t("history.typed.when")}</th>
                        <th>{t("history.typed.typed")}</th>
                        <th />
                    </tr>
                </thead>
                <tbody>
                    <For each={st.entries()}>{(item) => <EntryRow t={st} item={item} />}</For>
                </tbody>
            </table>
            <Show when={st.more()}>
                <Button tone="quiet" density="compact" busy={st.loading()} onClick={() => st.read(true)}>
                    {t("history.typed.older")}
                </Button>
            </Show>
        </Show>
    );
}

/** What was forgotten and can still come back, with Restore. */
export function TypedTrash(props: Part): JSX.Element {
    const st = props.t;
    return (
        <Show when={st.trash().length > 0}>
            <div class="warden-history-typed-trash">
                <div class="warden-manager-dim">{t("history.typed.trash", { count: st.retentionDays() })}</div>
                <For each={st.trash()}>
                    {(batch) => (
                        <div class="warden-history-typed-trash-row" data-batch={batch.batch_id}>
                            <span class="warden-history-typed-when">{formatWhen(batch.deleted_at_ms)}</span>
                            <span class="warden-history-typed-text" {...tip(batch.sample)}>
                                {batch.count > 1 ? t("history.typed.batch", { count: batch.count, sample: batch.sample }) : batch.sample}
                            </span>
                            <span class="warden-manager-dim">{t("history.typed.by", { name: batch.deleted_by })}</span>
                            <Button tone="quiet" density="compact" onClick={() => st.restore(batch)}>
                                {t("history.typed.restore")}
                            </Button>
                        </div>
                    )}
                </For>
            </div>
        </Show>
    );
}

/** What is kept, and for how long forgotten entries can come back. */
export function TypedFootnote(props: Part): JSX.Element {
    const st = props.t;
    return (
        <div class="warden-manager-footnote">
            {t("history.typed.footnote", { count: st.retentionDays() })}
        </div>
    );
}
