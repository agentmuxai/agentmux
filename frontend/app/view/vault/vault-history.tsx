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
import type { VaultActivity } from "@/types/rpc/VaultActivity";
import type { VaultChain } from "@/types/rpc/VaultChain";
import { formatWhen } from "./vault-format";
import { loadChain, loadHistory } from "./vault-store";

type Filter = "all" | "changes" | "uses";

const FILTERS: { id: Filter; label: string }[] = [
    { id: "all", label: "All" },
    { id: "changes", label: "Changes" },
    { id: "uses", label: "Uses" },
];

export function historyFilter(rows: VaultActivity[], filter: Filter): VaultActivity[] {
    return filter === "all" ? rows : rows.filter((r) => r.is_use === (filter === "uses"));
}

/** Why a row says what it does: the agent's words, or what a missing reason means for that row. */
export function whyOf(a: VaultActivity): string {
    if (a.reason) return a.reason;
    if (!a.actor_kind) return "(before reasons were recorded)";
    return a.actor_kind === "agent" ? "(no reason given)" : "";
}

/** Who did it, with the machine when the rows come from more than one. */
export function whoOf(a: VaultActivity): string {
    return a.actor === "you" ? "You" : a.actor;
}

export function chainNote(c: VaultChain | undefined): string {
    if (!c) return "";
    if (!c.intact) return `The record has been changed: line ${c.broken_at} — ${c.problem}.`;
    const before = c.before_chain > 0 ? `; ${c.before_chain} older ${c.before_chain === 1 ? "line was" : "lines were"} made before checking began` : "";
    const older = c.unchained_after > 0 ? ` ${c.unchained_after} ${c.unchained_after === 1 ? "line was" : "lines were"} written without a place in the chain, as an older AgentMux on this computer does; they are not checked.` : "";
    const head = c.head_checked ? "" : " The copy of the newest line kept in the keychain could not be checked, so lines removed from the end would not show.";
    return `Each line follows the one before it (${c.checked} checked${before}).${older}${head} This is evidence for you, not proof against someone who holds both this computer's database and its keychain.`;
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
            <ModalHeader title={`History of ${props.label}`} description="Every change and every use, newest first. A row never holds a value." />
            <ModalBody>
                <div class="vault-history-filters" role="group" aria-label="Show">
                    <For each={FILTERS}>
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
                <Show when={rows().length > 0} fallback={<div class="warden-section-stub">{history.loading ? "Loading…" : "Nothing here yet."}</div>}>
                    <div class="vault-dialog-scope">
                    <table class="warden-manager-table vault-history-table">
                        <thead>
                            <tr>
                                <th>When</th>
                                <th>Who</th>
                                <th>What</th>
                                <th>Where</th>
                                <th>Version</th>
                                <th>Why</th>
                                <th>Outcome</th>
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
                    Close
                </Button>
            </ModalFooter>
        </Modal>
    );
}
