// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault list's "Kept by AgentMux" section (vault-list.tsx): the secrets AgentMux keeps for itself. They are
// managed where they are used; the Vault only lists them. Folded until opened, and read each time it is.

import { createEffect, createMemo, createSignal, For, on, Show, type JSX } from "solid-js";

import { t } from "@/app/i18n/i18n";
import { VaultSectionBox } from "./vault-section";
import { loadKept, type KeptSecret } from "./vault-store";

const MANAGED_IN = ["accounts", "muxbus", "tower", "browser"];

const managedIn = (k: KeptSecret): string => (MANAGED_IN.includes(k.managed_in) ? t(`vault.kept.in.${k.managed_in}`) : k.managed_in);

function keptMatches(k: KeptSecret, filter: string): boolean {
    const q = filter.trim().toLowerCase();
    return !q || [k.label, k.what, managedIn(k)].some((s) => s.toLowerCase().includes(q));
}

export interface VaultListSectionProps {
    /** The Vault is on screen. */
    shown: boolean;
    open: boolean;
    onToggle: () => void;
    /** The list's one filter: an open section shows its rows that match. */
    filter: string;
    /** The list is narrowed (a filter, or Expiring soon): a section with nothing to show for it is left out. */
    narrowing: boolean;
    /** Only what expires soon is asked for, which nothing here does. */
    soonOnly: boolean;
    /** Told whether the section shows a match of the narrowed list, so the list doesn't say nothing matches over it. */
    onMatch?: (matched: boolean) => void;
}

function KeptTable(props: { rows: KeptSecret[] }): JSX.Element {
    return (
        <table class="warden-manager-table vault-table">
            <thead>
                <tr>
                    <th>{t("vault.kept.name")}</th>
                    <th>{t("vault.kept.what")}</th>
                    <th>{t("vault.kept.managedIn")}</th>
                </tr>
            </thead>
            <tbody>
                <For each={props.rows}>
                    {(k) => (
                        <tr>
                            <td>{k.label}</td>
                            <td>{k.what}</td>
                            <td class="warden-manager-dim">{managedIn(k)}</td>
                        </tr>
                    )}
                </For>
            </tbody>
        </table>
    );
}

export function VaultKept(props: VaultListSectionProps): JSX.Element {
    const [kept, setKept] = createSignal<KeptSecret[] | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const visible = () => props.shown && props.open;
    // Read again each time it is opened: the keychain changes where these secrets are managed, with no event.
    createEffect(
        on(visible, (v) => {
            if (!v) return;
            loadKept()
                .then((list) => {
                    setKept(list);
                    setError(null);
                })
                .catch((e) => setError(String(e)));
        })
    );
    const rows = createMemo(() => (kept() ?? []).filter((k) => keptMatches(k, props.filter)));
    const matched = () => props.narrowing && props.open && !props.soonOnly && rows().length > 0;
    createEffect(() => props.onMatch?.(matched()));
    const count = () => {
        const all = kept();
        if (all === null) return undefined;
        return props.narrowing ? t("vault.section.countOf", { shown: rows().length, total: all.length }) : String(all.length);
    };
    return (
        <Show when={!props.narrowing || matched()}>
            <VaultSectionBox id="kept" count={count()} open={props.open} onToggle={props.onToggle}>
                <p class="warden-manager-summary">{t("vault.kept.about")}</p>
                <Show when={error()}>
                    <div class="warden-manager-error">⚠ {error()}</div>
                </Show>
                <Show when={rows().length > 0} fallback={<div class="warden-section-stub">{kept() === null ? t("vault.list.loading") : t("vault.kept.none")}</div>}>
                    <KeptTable rows={rows()} />
                </Show>
                <p class="warden-manager-footnote">{t("vault.kept.footnote")}</p>
            </VaultSectionBox>
        </Show>
    );
}
