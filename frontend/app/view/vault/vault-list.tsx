// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault's list: one long list of coloured sections, one per type of entry, under one filter that covers
// all of them (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.8). After the types
// come what AgentMux keeps for itself and the Activity log, folded until opened. `vault:section` no longer
// picks a tab: it names the section to open and scroll to.

import { createEffect, createMemo, createSignal, For, on, Show, type JSX } from "solid-js";

import { Button, FilterInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { VaultActivityLog } from "./vault-activity";
import { isNarrowing, narrowedEntries, VaultEntries } from "./vault-entries";
import { expiryOf } from "./vault-format";
import { VaultKept } from "./vault-kept";
import { VAULT_SECTIONS, type VaultPaneSection } from "./vault-kinds";
import type { VaultViewModel } from "./vault-model";
import { vaultEntries, vaultLoading } from "./vault-store";

/** The types of entry, in the list's order. */
export const ENTRY_SECTIONS: VaultSection[] = ["personal", "cards", "api-keys", "logins", "notes"];

/** Which sections are folded, and the one the pane was asked to show: opened and scrolled to. */
function createSectionFolds(model: VaultViewModel, scroller: () => HTMLElement | undefined) {
    const [folded, setFolded] = createSignal<VaultPaneSection[]>(["kept", "activity"]);
    const isOpen = (id: VaultPaneSection) => !folded().includes(id);
    const toggle = (id: VaultPaneSection) => setFolded((was) => (was.includes(id) ? was.filter((s) => s !== id) : [...was, id]));
    // Clearing the request (so the same section can be asked for again) changes the section too: that one is not a request.
    let cleared = false;
    createEffect(
        on(model.sectionAtom, (id, before) => {
            if (cleared) {
                cleared = false;
                return;
            }
            // Opened with no section named: the list starts at its top.
            if (before === undefined && id === VAULT_SECTIONS.default) return;
            setFolded((was) => was.filter((s) => s !== id));
            queueMicrotask(() => scroller()?.querySelector<HTMLElement>(`[data-section="${id}"]`)?.scrollIntoView?.({ block: "start" }));
            if (id !== VAULT_SECTIONS.default) {
                cleared = true;
                model.setMeta({ "vault:section": null });
            }
        })
    );
    return { isOpen, toggle };
}

/** `vault` narrows the list to one Vault (its id); empty shows every Vault's entries. */
export function VaultList(props: { model: VaultViewModel; vault?: string }): JSX.Element {
    const [filter, setFilter] = createSignal("");
    const [soonAsked, setSoonAsked] = createSignal(false);
    let scroller: HTMLDivElement | undefined;
    const folds = createSectionFolds(props.model, () => scroller);
    const entries = createMemo(() => (vaultEntries() ?? []).filter((e) => !props.vault || e.vault_id === props.vault));
    // Expiring soon is offered while something has a date, and means nothing once nothing has.
    const dated = createMemo(() => entries().some((e) => expiryOf(e.fields.expires) !== null));
    const soonOnly = () => soonAsked() && dated();
    const narrowing = () => isNarrowing(filter(), soonOnly());
    const found = createMemo(() => narrowedEntries(entries(), filter(), soonOnly()).length);
    const note = () => {
        if (vaultEntries() === null) return vaultLoading() ? t("vault.list.loading") : null;
        if (narrowing()) return found() === 0 ? t("vault.list.noMatch") : null;
        return entries().length === 0 ? t("vault.list.empty") : null;
    };
    return (
        <div class="vault-list">
            <div class="vault-list__bar">
                <FilterInput value={filter()} onInput={setFilter} placeholder={t("vault.list.filter")} class="vault-list__filter" />
                <Show when={dated()}>
                    <Button density="compact" icon="clock" pressed={soonAsked()} onClick={() => setSoonAsked(!soonAsked())}>
                        {t("vault.list.soon")}
                    </Button>
                </Show>
            </div>
            <div class="vault-list__sections" ref={scroller} aria-label={t("vault.list.sections")}>
                <Show when={note()}>{(text) => <div class="warden-section-stub vault-list__note">{text()}</div>}</Show>
                <For each={ENTRY_SECTIONS}>
                    {(s) => (
                        <VaultEntries section={s} vault={props.vault} filter={filter()} soonOnly={soonOnly()} open={folds.isOpen(s)} onToggle={() => folds.toggle(s)} />
                    )}
                </For>
                <VaultKept shown={props.model.shown()} open={folds.isOpen("kept")} onToggle={() => folds.toggle("kept")} filter={filter()} narrowing={narrowing()} soonOnly={soonOnly()} />
                <VaultActivityLog shown={props.model.shown()} open={folds.isOpen("activity")} onToggle={() => folds.toggle("activity")} filter={filter()} narrowing={narrowing()} soonOnly={soonOnly()} />
            </div>
        </div>
    );
}
