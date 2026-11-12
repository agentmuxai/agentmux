// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Typed pane's markup in parts (history-typed.tsx): the bar, the entries (or why there are none), the list of
// what was forgotten, and the footnote. Each reads the pane's state (history-typed-state.ts).

import { For, Show, type JSX } from "solid-js";

import { Button, FilterInput, Select } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import type { InputHistoryItem } from "@/types/rpc/InputHistoryItem";
import { formatWhen, typedTags } from "./history-shared";
import type { HistoryTypedState } from "./history-typed-state";

type Part = { t: HistoryTypedState };

/** The agent, the search, Refresh, and Clear… (which asks again by naming the agent). */
export function TypedBar(props: Part): JSX.Element {
    const t = props.t;
    return (
        <div class="warden-history-bar">
            <Select
                ariaLabel="Agent"
                density="compact"
                value={t.agentId()}
                onChange={t.setAgentId}
                disabled={t.list().length === 0}
                options={t.list().map((a) => ({ value: a.id, label: a.name }))}
            />
            <FilterInput value={t.query()} onInput={t.setQuery} placeholder="Search what was typed" />
            <Button
                tone="quiet"
                density="compact"
                icon="rotate"
                busy={t.loading()}
                onClick={() => {
                    void t.loadAgents();
                    void t.read(false);
                    void t.readTrash();
                }}
            >
                Refresh
            </Button>
            <Show when={t.entries().length > 0 && !t.query()}>
                <Show
                    when={t.confirmClear()}
                    fallback={
                        <Button tone="quiet" density="compact" onClick={() => t.setConfirmClear(true)}>
                            Clear…
                        </Button>
                    }
                >
                    <Button tone="danger" density="compact" onClick={() => void t.clearAll()}>
                        {`Forget everything typed to ${t.agentName()}`}
                    </Button>
                    <Button tone="quiet" density="compact" onClick={() => t.setConfirmClear(false)}>
                        Cancel
                    </Button>
                </Show>
            </Show>
        </div>
    );
}

/** How an entry was typed, and how many times in a row it was sent. */
const tags = (item: InputHistoryItem): string =>
    [...typedTags(item.kind, item.source), ...(item.sends > 1 ? [`sent ${item.sends} times in a row`] : [])].join(" · ");

/** One entry, with Forget (which asks again). */
function EntryRow(props: Part & { item: InputHistoryItem }): JSX.Element {
    const { t, item } = props;
    return (
        <tr data-entry={item.id}>
            <td class="warden-history-typed-when warden-history-cell-wide" data-label="When">{formatWhen(item.sent_at_ms)}</td>
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
                    when={t.confirmId() === item.id}
                    fallback={
                        <Button tone="quiet" density="compact" onClick={() => t.setConfirmId(item.id)}>
                            Forget
                        </Button>
                    }
                >
                    <Button tone="danger" density="compact" onClick={() => void t.forget(item)}>
                        Forget this
                    </Button>
                    <Button tone="quiet" density="compact" onClick={() => t.setConfirmId(null)}>
                        Cancel
                    </Button>
                </Show>
            </td>
        </tr>
    );
}

/** The entries, newest first, with Show older; or why there are none. */
export function TypedEntries(props: Part): JSX.Element {
    const t = props.t;
    return (
        <Show
            when={t.entries().length > 0}
            fallback={
                <div class="warden-section-stub">
                    {t.agents() === null || t.loading()
                        ? "Loading…"
                        : t.list().length === 0
                          ? "No agent to show."
                          : t.query()
                            ? "Nothing typed to this agent matches."
                            : "Nothing typed to this agent yet."}
                </div>
            }
        >
            <table class="warden-manager-table warden-history-table warden-history-typed-table">
                <thead>
                    <tr>
                        <th>When</th>
                        <th>Typed</th>
                        <th />
                    </tr>
                </thead>
                <tbody>
                    <For each={t.entries()}>{(item) => <EntryRow t={t} item={item} />}</For>
                </tbody>
            </table>
            <Show when={t.more()}>
                <Button tone="quiet" density="compact" busy={t.loading()} onClick={() => void t.read(true)}>
                    Show older
                </Button>
            </Show>
        </Show>
    );
}

/** What was forgotten and can still come back, with Restore. */
export function TypedTrash(props: Part): JSX.Element {
    const t = props.t;
    return (
        <Show when={t.trash().length > 0}>
            <div class="warden-history-typed-trash">
                <div class="warden-manager-dim">{`Recently forgotten (restorable for ${t.retentionDays()} days)`}</div>
                <For each={t.trash()}>
                    {(batch) => (
                        <div class="warden-history-typed-trash-row" data-batch={batch.batch_id}>
                            <span class="warden-history-typed-when">{formatWhen(batch.deleted_at_ms)}</span>
                            <span class="warden-history-typed-text" {...tip(batch.sample)}>
                                {batch.count > 1 ? `${batch.count} entries, from “${batch.sample}”` : batch.sample}
                            </span>
                            <span class="warden-manager-dim">{`by ${batch.deleted_by}`}</span>
                            <Button tone="quiet" density="compact" onClick={() => void t.restore(batch)}>
                                Restore
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
    const t = props.t;
    return (
        <div class="warden-manager-footnote">
            What was typed to an agent is kept with the agent, up to its limit (the agent:inputhistory:max setting), and
            goes when the agent is deleted. Forgotten entries can be restored for {t.retentionDays()} days, then they are
            gone.
        </div>
    );
}
