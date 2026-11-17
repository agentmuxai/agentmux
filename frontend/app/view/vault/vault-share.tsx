// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Sharing a login with an agent (docs/specs/SPEC_VAULT_AGENTS_ENTER_CREDENTIALS_WHEREVER_ASKED_2026_11_16.md §5): the
// Share button's dialog, and a login's list of who it is shared with, each with a button to stop. Only the person
// shares: an agent can only ask (`Vault` `share`), and the person answers that on a card.

import { createMemo, createResource, createSignal, For, Show, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, CloseButton, Field, Select, type SelectOption } from "@/app/element/ui";
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
            {props.every ? "Any agent" : "The agent"} will be able to ask to sign in to <strong>{siteOf(props.entry.fields.site ?? "")}</strong> with this login, from its own browser profile. It never sees the user name or the password, and
            you are asked the first time it signs in{props.every ? ", for each agent" : ""}. You can stop sharing from the login's row.
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
            { value: "", label: "Choose an agent…", disabled: true },
            ...(everyone ? [] : [{ value: EVERY_AGENT, label: "Every agent" }]),
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
            <ModalHeader title={`Share ${props.entry.label}`} description="Let an agent sign in to this site with this login, from its own browser profile." />
            <ModalBody>
                <Field label="Share with" layout="stacked">
                    <Select value={pick()} onChange={setPick} options={options()} ariaLabel="Share with" disabled={busy() || data.loading} />
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
                    Cancel
                </Button>
                <Button tone="accent" busy={busy()} disabled={!pick()} onClick={() => void share()}>
                    Share
                </Button>
            </ModalFooter>
        </Modal>
    );
}

/** How a share has been used: the number of sign-ins and the last one. */
function usedLine(uses: number, last: number): string {
    if (uses === 0) return "not used yet";
    return `${uses} sign-in${uses === 1 ? "" : "s"}, last ${formatWhen(last)}`;
}

/** A login's shares under its label: each agent it is shared with, how it was used, and a button to stop. */
export function EntryShares(props: { entry: VaultEntryInfo; onError: (message: string | null) => void }): JSX.Element {
    // Read again when the login changes (a share is a change to it) and when it is used.
    const [data, { refetch }] = createResource(
        () => (isShared(props.entry) ? `${props.entry.id}|${props.entry.updated_at}|${props.entry.last_used_at}` : false),
        () => loadShares(props.entry.id)
    );
    const stop = async (agent: string) => {
        try {
            await unshareLogin(props.entry.id, agent);
            props.onError(null);
            void refetch();
        } catch (e) {
            props.onError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <Show when={isShared(props.entry) && (data()?.shares.length ?? 0) > 0}>
            <div class="vault-grants" aria-label={`Agents ${props.entry.label} is shared with`}>
                <span class="warden-manager-dim">Shared with </span>
                <For each={data()?.shares ?? []}>
                    {(s) => (
                        <span class="vault-grant" data-agent={s.agent}>
                            {s.name} <span class="warden-manager-dim">({usedLine(s.uses, s.last_used_ms)})</span>
                            <CloseButton size="sm" label={`Stop sharing ${props.entry.label} with ${s.name}`} onClick={() => void stop(s.agent)} />
                        </span>
                    )}
                </For>
            </div>
        </Show>
    );
}
