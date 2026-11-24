// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An entry's history: each change to it and each use of it, newest first, with who, where, which version of the
// value, and the reason the agent gave (docs/specs/SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md
// §6). Never a value. The footer says whether the record still holds together.

import { createMemo, createResource, createSignal, For, Show, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { tip } from "@/app/element/tip";
import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultActivity } from "@/types/rpc/VaultActivity";
import type { VaultChain } from "@/types/rpc/VaultChain";
import { formatWhen } from "./vault-format";
import { loadChain, loadHistory } from "./vault-store";

type Filter = "all" | "changes" | "uses";

// Built when drawn, so a change of language shows.
const filters = (): { id: Filter; label: string }[] => [
    { id: "all", label: t("vault.history.filterAll") },
    { id: "changes", label: t("vault.history.filterChanges") },
    { id: "uses", label: t("vault.history.filterUses") },
];

export function historyFilter(rows: VaultActivity[], filter: Filter): VaultActivity[] {
    return filter === "all" ? rows : rows.filter((r) => r.is_use === (filter === "uses"));
}

/** Why a row says what it does: the agent's words, or what a missing reason means for that row. */
export function whyOf(a: VaultActivity): string {
    if (a.reason) return a.reason;
    if (!a.actor_kind) return t("vault.history.beforeReasons");
    return a.actor_kind === "agent" ? t("vault.history.noReason") : "";
}

/** Who did it, with the machine when the rows come from more than one. */
export function whoOf(a: VaultActivity): string {
    return a.actor === "you" ? t("vault.history.you") : agentNameOf(a.actor);
}

/** An agent's row is written `agent:<id>`; the panel shows the name alone, as it always has. */
export function agentNameOf(actor: string): string {
    return actor.startsWith("agent:") ? actor.slice("agent:".length) : actor;
}

export function chainNote(c: VaultChain | undefined): string {
    if (!c) return "";
    if (!c.intact) return t("vault.history.chainBroken", { line: c.broken_at, problem: c.problem });
    const before = c.before_chain > 0 ? `; ${t("vault.history.chainBefore", { count: c.before_chain })}` : "";
    const older = c.unchained_after > 0 ? ` ${t("vault.history.chainOlder", { count: c.unchained_after })}` : "";
    const head = c.head_checked ? "" : ` ${t("vault.history.chainHead")}`;
    return t("vault.history.chain", { checked: c.checked, before, older, head });
}

function HistoryRow(props: { row: VaultActivity; showHost: boolean }): JSX.Element {
    const a = () => props.row;
    return (
        <tr data-outcome={a().outcome} data-seq={a().seq}>
            <td class="warden-manager-dim" {...tip(new Date(a().at).toLocaleString())}>
                {formatWhen(a().at)}
            </td>
            <td>
                {whoOf(a())}
                <Show when={props.showHost && a().host}>
                    <div class="warden-manager-dim">{a().host}</div>
                </Show>
            </td>
            <td>{a().action}</td>
            <td class="warden-manager-dim">{a().target}</td>
            <td class="warden-manager-mono">{a().version > 0 ? `v${a().version}` : ""}</td>
            <td>
                {whyOf(a())}
                <Show when={a().refs.length > 0}>
                    <div class="warden-manager-dim">{a().refs.join(" · ")}</div>
                </Show>
            </td>
            <td class="warden-manager-dim">{a().outcome}</td>
        </tr>
    );
}

export function VaultHistoryDialog(props: { entryId: string; label: string; onClose: () => void }): JSX.Element {
    const [history] = createResource(() => props.entryId, loadHistory);
    const [chain] = createResource(loadChain);
    const [filter, setFilter] = createSignal<Filter>("all");
    const rows = createMemo(() => historyFilter(history() ?? [], filter()));
    const showHost = createMemo(() => new Set((history() ?? []).map((r) => r.host).filter(Boolean)).size > 1);

    return (
        <Modal open={true} scope="pane" onClose={props.onClose} size="lg" panelClass="vault-form vault-history">
            <ModalHeader title={t("vault.history.title", { label: props.label })} description={t("vault.history.description")} />
            <ModalBody>
                <div class="vault-history-filters" role="group" aria-label={t("vault.history.show")}>
                    <For each={filters()}>
                        {(f) => (
                            <Button density="compact" pressed={filter() === f.id} onClick={() => setFilter(f.id)}>
                                {f.label}
                            </Button>
                        )}
                    </For>
                </div>
                <Show when={history.error}>
                    <div class="warden-manager-error" role="alert">
                        ⚠ {String(history.error)}
                    </div>
                </Show>
                <Show when={rows().length > 0} fallback={<div class="warden-section-stub">{history.loading ? t("vault.history.loading") : t("vault.history.nothing")}</div>}>
                    <div class="vault-dialog-scope">
                    <table class="warden-manager-table vault-history-table">
                        <thead>
                            <tr>
                                <th>{t("vault.activity.when")}</th>
                                <th>{t("vault.activity.who")}</th>
                                <th>{t("vault.activity.what")}</th>
                                <th>{t("vault.activity.where")}</th>
                                <th>{t("vault.history.colVersion")}</th>
                                <th>{t("vault.activity.why")}</th>
                                <th>{t("vault.secrets.outcome")}</th>
                            </tr>
                        </thead>
                        <tbody>
                            <For each={rows()}>{(r) => <HistoryRow row={r} showHost={showHost()} />}</For>
                        </tbody>
                    </table>
                    </div>
                </Show>
                <Show when={chain()}>
                    {(c) => (
                        <p class="warden-manager-footnote" classList={{ "warden-manager-error": !c().intact }} role={c().intact ? undefined : "alert"}>
                            {chainNote(c())}
                        </p>
                    )}
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.onClose}>
                    {t("vault.history.close")}
                </Button>
            </ModalFooter>
        </Modal>
    );
}
