// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What an agent changed in the Vault that the person has not looked at: a quiet strip at the top of the entries list, and a
// list behind its Review button (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md §5). No card, no
// modal, no sound. An agent's words are shown as text, quoted and attributed, and decide nothing. Undo, Seen, Accept and
// Dismiss are the person's own actions; a removal is confirmed in the row, not in a modal.

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { AgentChange } from "@/types/rpc/AgentChange";
import { formatWhen } from "./vault-format";
import { acceptProposal, dismissProposal, markAllChangesSeen, markChangeSeen, undoChange, vaultChanges } from "./vault-store";

/** The strip's words: how many changes wait. */
export function stripText(count: number): string {
    return count === 1 ? t("vault.changes.countOne") : t("vault.changes.countMany", { count });
}

/** What one change says, in words: who did what to which entry. The agent's reason is shown apart, quoted. */
export function changeSentence(c: AgentChange): string {
    const base = { agent: c.agent, label: c.entry_label };
    switch (c.kind) {
        case "added":
            return t("vault.changes.added", base);
        case "new value":
            return c.replaces_yours ? t("vault.changes.replacedYours", base) : t("vault.changes.newValue", base);
        case "changed details":
            return t("vault.changes.changed", { ...base, what: c.target });
        case "proposal":
            return t("vault.changes.proposal", { ...base, field: c.proposal?.field ?? "", value: c.proposal?.value || t("vault.changes.nothing"), current: c.proposal?.current || t("vault.changes.nothing") });
        default:
            return t("vault.changes.changed", { ...base, what: c.target });
    }
}

/** The small mark on an entry's row: new, or changed by an agent, until the person has looked. */
export function markOf(entryId: string, changes: AgentChange[] = vaultChanges()): string | null {
    const mine = (changes ?? []).filter((c) => c.entry_id === entryId && c.kind !== "proposal");
    if (mine.length === 0) return null;
    return mine.some((c) => c.kind === "added") ? t("vault.changes.markNew") : t("vault.changes.markChanged", { agent: mine[0].agent });
}

function ChangeRow(props: { change: AgentChange; busy: boolean; run: (what: () => Promise<unknown>) => void }): JSX.Element {
    const c = () => props.change;
    const [confirming, setConfirming] = createSignal(false);
    const proposal = () => c().kind === "proposal";
    return (
        <li class="vault-change" data-seq={c().seq} data-kind={c().kind} data-first={proposal() || c().replaces_yours ? "" : undefined}>
            <div class="vault-change__what">{changeSentence(c())}</div>
            <div class="vault-change__when warden-manager-dim">{formatWhen(c().at)}</div>
            <Show when={c().reason}>
                <p class="vault-change__reason">
                    <q>{c().reason}</q>
                </p>
            </Show>
            <div class="vault-change__actions">
                <Show
                    when={proposal()}
                    fallback={
                        <>
                            <Show when={c().undo && !confirming()}>
                                <Button density="compact" disabled={props.busy} onClick={() => (c().undo === "remove" ? setConfirming(true) : props.run(() => undoChange(c().seq)))}>
                                    {t("vault.changes.undo")}
                                </Button>
                            </Show>
                            <Show when={!confirming()}>
                                <Button density="compact" disabled={props.busy} onClick={() => props.run(() => markChangeSeen(c().seq))}>
                                    {t("vault.changes.seen")}
                                </Button>
                            </Show>
                        </>
                    }
                >
                    <Button density="compact" disabled={props.busy} onClick={() => props.run(() => acceptProposal(c().entry_id, c().proposal!.field, c().proposal!.at))}>
                        {t("vault.changes.accept")}
                    </Button>
                    <Button density="compact" disabled={props.busy} onClick={() => props.run(() => dismissProposal(c().entry_id, c().proposal!.field, c().proposal!.at))}>
                        {t("vault.changes.dismiss")}
                    </Button>
                </Show>
            </div>
            <Show when={confirming()}>
                <div class="vault-change__confirm" role="group" aria-label={t("vault.changes.removeAsk", { label: c().entry_label })}>
                    <span>{t("vault.changes.removeAsk", { label: c().entry_label })}</span>
                    <Button density="compact" disabled={props.busy} onClick={() => props.run(() => undoChange(c().seq))}>
                        {t("vault.changes.remove")}
                    </Button>
                    <Button density="compact" onClick={() => setConfirming(false)}>
                        {t("vault.changes.keep")}
                    </Button>
                </div>
            </Show>
        </li>
    );
}

/** The strip at the top of the entries list: nothing while there is nothing to look at. */
export function VaultChangesStrip(): JSX.Element {
    const [open, setOpen] = createSignal(false);
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const run = (what: () => Promise<unknown>) => {
        setBusy(true);
        setError(null);
        what()
            .catch((e) => setError(e instanceof Error ? e.message : String(e)))
            .finally(() => setBusy(false));
    };
    return (
        <Show when={vaultChanges().length > 0}>
            <section class="vault-changes" aria-label={t("vault.changes.label")}>
                <div class="vault-changes__bar" role="status">
                    <span class="vault-changes__text">{stripText(vaultChanges().length)}</span>
                    <Button density="compact" pressed={open()} aria-expanded={open()} onClick={() => setOpen(!open())}>
                        {t("vault.changes.review")}
                    </Button>
                    <Show when={open()}>
                        <Button density="compact" disabled={busy()} onClick={() => run(() => markAllChangesSeen())}>
                            {t("vault.changes.seenAll")}
                        </Button>
                    </Show>
                </div>
                <Show when={error()}>
                    {(message) => (
                        <div class="warden-manager-error" role="alert">
                            ⚠ {message()}
                        </div>
                    )}
                </Show>
                <Show when={open()}>
                    <ul class="vault-changes__list">
                        <For each={vaultChanges()}>{(c) => <ChangeRow change={c} busy={busy()} run={run} />}</For>
                    </ul>
                </Show>
            </section>
        </Show>
    );
}
