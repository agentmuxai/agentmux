// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sharing a login with an agent (docs/specs/SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md §5): the
// Share button's dialog, and a login's list of who it is shared with, each with a button to stop. Only the person
// shares: an agent can only ask (`Vault` `share`), and the person answers that on a card.

import { createMemo, createResource, createSignal, For, Show, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, CloseButton, Field, IconButton, Select, type SelectOption } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import { formatWhen } from "./vault-format";
import { loadShares, shareLogin, unshareLogin } from "./vault-store";
import { siteOf } from "./vault-use-question";

/** The Share dialog's value for every agent: srv's own mark for it. */
export const EVERY_AGENT = "*";

/** Whether a login can be shared: one saved for a site. A sign-in for a site's HTTP prompt or a proxy is typed by the
 *  browser itself, so it has no share. */
export function canShare(e: VaultEntryInfo): boolean {
    return e.kind === "login" && !!e.fields.site && !e.fields.realm && !e.fields.proxy;
}

/** Whether a login is shared with anyone, as its fields say. */
export function isShared(e: VaultEntryInfo): boolean {
    return !!e.fields.shared;
}

/** What a share is for, said in the dialog: it lets the agent ask, and never answers for the person. */
function ShareNote(props: { entry: VaultEntryInfo; every: boolean }): JSX.Element {
    return (
        <p class="warden-manager-dim">
            <Rich k={props.every ? "vault.share.anyAgentNote" : "vault.share.agentNote"} params={{ site: siteOf(props.entry.fields.site ?? "") }} />
        </p>
    );
}

/** Shares a login with an agent, or with every agent. */
export function VaultShareDialog(props: { entry: VaultEntryInfo; onClose: () => void }): JSX.Element {
    const [data] = createResource(() => props.entry.id, loadShares);
    const [pick, setPick] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const options = createMemo<SelectOption[]>(() => {
        const list = data()?.agents ?? [];
        const everyone = (data()?.shares ?? []).some((s) => s.agent === EVERY_AGENT);
        return [
            { value: "", label: t("vault.share.choose"), disabled: true },
            ...(everyone ? [] : [{ value: EVERY_AGENT, label: t("vault.share.every") }]),
            ...list.map((a) => ({ value: a.uid, label: a.name })),
        ];
    });
    const share = async () => {
        setBusy(true);
        try {
            await shareLogin(props.entry.id, pick());
            props.onClose();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
            setBusy(false);
        }
    };
    return (
        <Modal open={true} scope="pane" onClose={props.onClose} size="md" panelClass="vault-form">
            <ModalHeader title={t("vault.share.title", { label: props.entry.label })} description={t("vault.share.description")} />
            <ModalBody>
                <Field label={t("vault.share.with")} layout="stacked">
                    <Select value={pick()} onChange={setPick} options={options()} ariaLabel={t("vault.share.with")} disabled={busy() || data.loading} />
                </Field>
                <Show when={pick()}>
                    <ShareNote entry={props.entry} every={pick() === EVERY_AGENT} />
                </Show>
                <Show when={error() ?? (data.error ? String(data.error) : null)}>
                    {(message) => (
                        <div class="warden-manager-error" role="alert">
                            {message()}
                        </div>
                    )}
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.onClose} disabled={busy()}>
                    {t("vault.share.cancel")}
                </Button>
                <Button tone="accent" busy={busy()} disabled={!pick()} onClick={() => void share()}>
                    {t("vault.share.confirm")}
                </Button>
            </ModalFooter>
        </Modal>
    );
}

/** The row's Share button, which opens its own dialog. */
export function ShareButton(props: { entry: VaultEntryInfo }): JSX.Element {
    const [open, setOpen] = createSignal(false);
    return (
        <>
            <IconButton icon="share" label={t("vault.share.button")} density="compact" onClick={() => setOpen(true)} />
            <Show when={open()}>
                <VaultShareDialog entry={props.entry} onClose={() => setOpen(false)} />
            </Show>
        </>
    );
}

/** How a share has been used: the number of sign-ins and the last one. */
function usedLine(uses: number, last: number): string {
    if (uses === 0) return t("vault.share.notUsed");
    return t("vault.share.used", { count: uses, when: formatWhen(last) });
}

/** A login's shares under its label: each agent it is shared with, how it was used, and a button to stop. */
export function EntryShares(props: { entry: VaultEntryInfo }): JSX.Element {
    const [problem, setProblem] = createSignal<string | null>(null);
    // Read again when the login changes (a share is a change to it) and when it is used.
    const [data, { refetch }] = createResource(
        () => (isShared(props.entry) ? `${props.entry.id}|${props.entry.updated_at}|${props.entry.last_used_at}` : false),
        () => loadShares(props.entry.id)
    );
    const stop = async (agent: string) => {
        try {
            await unshareLogin(props.entry.id, agent);
            setProblem(null);
            void refetch();
        } catch (e) {
            setProblem(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <Show when={isShared(props.entry) && (data()?.shares.length ?? 0) > 0}>
            <div class="vault-grants" aria-label={t("vault.share.listLabel", { label: props.entry.label })}>
                <span class="warden-manager-dim">{t("vault.share.sharedWith")} </span>
                <For each={data()?.shares ?? []}>
                    {(s) => (
                        <span class="vault-grant" data-agent={s.agent}>
                            {s.name} <span class="warden-manager-dim">({usedLine(s.uses, s.last_used_ms)})</span>
                            <CloseButton size="sm" label={t("vault.share.stop", { label: props.entry.label, name: s.name })} onClick={() => void stop(s.agent)} />
                        </span>
                    )}
                </For>
                <Show when={problem()}>
                    <span class="warden-manager-error" role="alert">
                        {problem()}
                    </span>
                </Show>
            </div>
        </Show>
    );
}
