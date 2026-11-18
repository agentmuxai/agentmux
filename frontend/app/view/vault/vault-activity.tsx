// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault list's Activity section (vault-list.tsx): every change to the Vault and every use of an entry.
// Folded until opened; it loads when opened and again whenever the Vault changes while open.

import { createEffect, createMemo, createSignal, For, on, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import type { VaultActivity } from "@/types/rpc/VaultActivity";
import { groupBatches, type ActivityGroup } from "./vault-activity-batches";
import { formatWhen, outcomeOf } from "./vault-format";
import { VaultHistoryDialog, whoOf, whyOf } from "./vault-history";
import type { VaultListSectionProps } from "./vault-kept";
import { VaultSectionBox } from "./vault-section";
import { loadActivity, vaultChangedMs, vaultEntries } from "./vault-store";

/** One line of the Activity section; an entry that still exists opens its History. A batch's own line (`secrets`
 *  lines of its own) opens and closes the lines of its secrets; `nested` is one of those. */
function ActivityRow(props: { row: VaultActivity; onHistory: (a: VaultActivity) => void; secrets?: number; open?: boolean; onToggle?: () => void; nested?: boolean }): JSX.Element {
    const a = () => props.row;
    const exists = () => (vaultEntries() ?? []).some((e) => e.id === a().entry_id);
    return (
        <tr data-outcome={a().outcome} classList={{ "vault-activity-nested": !!props.nested }}>
            <td class="warden-manager-dim vault-activity-word">{formatWhen(a().at)}</td>
            <td class="vault-activity-word">{whoOf(a())}</td>
            <td class="vault-activity-word">
                {a().action}
                <Show when={outcomeOf(a().outcome)}>
                    {(o) => (
                        <span class={o().warn ? "vault-activity-outcome vault-expiry-warn" : "vault-activity-outcome"}>
                            {" · "}
                            {o().text}
                        </span>
                    )}
                </Show>
            </td>
            <td>
                <Show when={exists()} fallback={a().entry_label}>
                    <Button density="compact" onClick={() => props.onHistory(a())}>
                        {a().entry_label}
                    </Button>
                </Show>
                <Show when={props.secrets}>
                    {" "}
                    <Button density="compact" aria-expanded={!!props.open} onClick={() => props.onToggle?.()}>
                        {t("vault.activity.batchSecrets", { count: props.secrets ?? 0 })}
                    </Button>
                </Show>
            </td>
            <td class="warden-manager-dim">{a().target}</td>
            <td class="warden-manager-dim">{whyOf(a())}</td>
        </tr>
    );
}

/** A line of the Activity section and, under a batch's own line once it is opened, the line of each of its secrets. */
function ActivityLines(props: { group: ActivityGroup; open: boolean; onToggle: () => void; onHistory: (a: VaultActivity) => void }): JSX.Element {
    return (
        <>
            <ActivityRow row={props.group.row} onHistory={props.onHistory} secrets={props.group.items.length} open={props.open} onToggle={props.onToggle} />
            <Show when={props.open}>
                <For each={props.group.items}>{(a) => <ActivityRow row={a} onHistory={props.onHistory} nested />}</For>
            </Show>
        </>
    );
}

function rowMatches(a: VaultActivity, q: string): boolean {
    return [whoOf(a), a.action, a.entry_label, a.target, whyOf(a), outcomeOf(a.outcome)?.text ?? ""].some((s) => (s ?? "").toLowerCase().includes(q));
}

/** A line matches the list's filter by who, what, which entry, where or why; a batch, by its own line or any of its secrets'. */
export function activityMatches(g: ActivityGroup, filter: string): boolean {
    const q = filter.trim().toLowerCase();
    return !q || rowMatches(g.row, q) || g.items.some((a) => rowMatches(a, q));
}

function ActivityTable(props: { groups: ActivityGroup[]; onHistory: (a: VaultActivity) => void }): JSX.Element {
    // The batches whose secrets' lines are shown: kept by batch, so a reload of the list leaves them open.
    const [openBatches, setOpenBatches] = createSignal<string[]>([]);
    const toggleBatch = (batch: string) => setOpenBatches((was) => (was.includes(batch) ? was.filter((b) => b !== batch) : [...was, batch]));
    return (
        <table class="warden-manager-table vault-table vault-table--wide">
            <thead>
                <tr>
                    <th>{t("vault.activity.when")}</th>
                    <th>{t("vault.activity.who")}</th>
                    <th>{t("vault.activity.what")}</th>
                    <th>{t("vault.activity.entry")}</th>
                    <th>{t("vault.activity.where")}</th>
                    <th>{t("vault.activity.why")}</th>
                </tr>
            </thead>
            <tbody>
                <For each={props.groups}>
                    {(g) => <ActivityLines group={g} open={openBatches().includes(g.batch)} onToggle={() => toggleBatch(g.batch)} onHistory={props.onHistory} />}
                </For>
            </tbody>
        </table>
    );
}

export function VaultActivityLog(props: VaultListSectionProps): JSX.Element {
    const [rows, setRows] = createSignal<VaultActivity[] | null>(null);
    const [historyOf, setHistoryOf] = createSignal<VaultActivity | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const load = () =>
        loadActivity()
            .then((r) => {
                setRows(r);
                setError(null);
            })
            .catch((e) => setError(String(e)));
    // Loads when first opened, and again whenever the Vault changes while open.
    createEffect(
        on([() => props.shown && props.open, vaultChangedMs], ([visible]) => {
            if (visible) void load();
        })
    );
    const all = createMemo(() => groupBatches(rows() ?? []));
    const groups = createMemo(() => all().filter((g) => activityMatches(g, props.filter)));
    const matched = () => props.narrowing && props.open && !props.soonOnly && groups().length > 0;
    createEffect(() => props.onMatch?.(matched()));
    const count = () => {
        if (rows() === null) return undefined;
        return props.narrowing ? t("vault.section.countOf", { shown: groups().length, total: all().length }) : String(all().length);
    };
    return (
        <>
            <Show when={!props.narrowing || matched()}>
                <VaultSectionBox id="activity" count={count()} open={props.open} onToggle={props.onToggle}>
                    <p class="warden-manager-summary">
                        {t("vault.activity.about")}{" "}
                        <Button density="compact" onClick={() => void openOrFocusPaneByView("audit")}>
                            {t("vault.activity.audit")}
                        </Button>
                    </p>
                    <Show when={error()}>
                        <div class="warden-manager-error">⚠ {error()}</div>
                    </Show>
                    <Show when={groups().length > 0} fallback={<div class="warden-section-stub">{rows() === null ? t("vault.list.loading") : t("vault.activity.none")}</div>}>
                        <ActivityTable groups={groups()} onHistory={setHistoryOf} />
                    </Show>
                </VaultSectionBox>
            </Show>
            <Show when={historyOf()}>
                {(a) => <VaultHistoryDialog entryId={a().entry_id} label={a().entry_label} onClose={() => setHistoryOf(null)} />}
            </Show>
        </>
    );
}
