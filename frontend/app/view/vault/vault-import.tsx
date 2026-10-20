// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Import from a `.vault` file
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.4, §8.5).
// The host asks which file; srv decrypts it into memory and describes it with no
// values; nothing is written until the person presses Import, and then all of it
// is written or none of it.

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, Field, SegmentedControl, TextInput } from "@/app/element/ui";
import type { ImportPreview } from "@/types/rpc/ImportPreview";
import type { ImportResult } from "@/types/rpc/ImportResult";
import { SECTION_LABELS, vaultKind } from "./vault-kinds";
import { cancelImport, commitImport, pickImport, type Duplicates, type ImportStep } from "./vault-store";

const DUPLICATE_CHOICES: { value: Duplicates; label: string }[] = [
    { value: "skip", label: "Skip them" },
    { value: "replace", label: "Replace mine" },
    { value: "keep_both", label: "Keep both" },
];

export function VaultImportDialog(props: { onClose: () => void }): JSX.Element {
    const [step, setStep] = createSignal<ImportStep | null>(null);
    const [pass, setPass] = createSignal("");
    const [duplicates, setDuplicates] = createSignal<Duplicates>("skip");
    const [busy, setBusy] = createSignal(true);
    const [error, setError] = createSignal<string | null>(null);
    const [result, setResult] = createSignal<ImportResult | null>(null);
    let token = "";

    const apply = (s: ImportStep | null) => {
        if (s === null) {
            // The person closed the file dialog.
            props.onClose();
            return;
        }
        setStep(s);
        if (s.kind === "preview") token = s.preview.token;
    };

    const run = async (what: () => Promise<ImportStep | null>) => {
        setBusy(true);
        setError(null);
        try {
            apply(await what());
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };

    // Opening the dialog asks which file.
    onMount(() => void run(() => pickImport()));
    // Closing without importing drops what srv holds in memory.
    onCleanup(() => {
        if (token && !result()) void cancelImport(token);
    });

    const unlock = () => {
        const s = step();
        if (s?.kind !== "needs_passphrase") return;
        const typed = pass();
        setPass("");
        return run(() => pickImport(s.pickId, typed));
    };

    const commit = async (preview: ImportPreview) => {
        setBusy(true);
        setError(null);
        try {
            setResult(await commitImport(preview.token, duplicates()));
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };

    const title = () => {
        const s = step();
        return s ? `Import ${s.kind === "preview" ? s.preview.file_name : s.fileName}` : "Import into the Vault";
    };

    return (
        <Modal open={true} scope="pane" onClose={props.onClose} size="md" closeOnBackdropClick={false} panelClass="vault-form">
            <ModalHeader title={title()} description="Nothing is added until you press Import." />
            <ModalBody>
                <Show when={result()}>
                    {(r) => (
                        <div role="status">
                            <p>
                                Imported: {r().added} added, {r().replaced} replaced, {r().skipped} skipped.
                            </p>
                            <p class="warden-manager-dim">The file stays where it is. Delete it if you don't need it.</p>
                        </div>
                    )}
                </Show>
                <Show when={!result()}>
                    <Show when={busy() && !step()}>
                        <p class="warden-manager-dim">Choose the file in the window that opened…</p>
                    </Show>
                    <Show when={step()?.kind === "needs_passphrase"}>
                        <form
                            onSubmit={(e) => {
                                e.preventDefault();
                                void unlock();
                            }}
                        >
                            <Field label="Passphrase" layout="stacked" description="This file is encrypted. Type the passphrase it was exported with.">
                                <TextInput type="password" autocomplete="new-password" spellcheck={false} autofocus value={pass()} onInput={(e) => setPass(e.currentTarget.value)} />
                            </Field>
                            <input type="submit" hidden />
                        </form>
                    </Show>
                    <Show when={step()?.kind === "preview" ? (step() as Extract<ImportStep, { kind: "preview" }>).preview : null}>
                        {(preview) => (
                            <div class="vault-import-preview">
                                <p>
                                    <strong>{preview().total}</strong> {preview().total === 1 ? "entry" : "entries"} to import.
                                </p>
                                <For each={preview().sections}>
                                    {(s) => (
                                        <div class="vault-import-section">
                                            <strong>{SECTION_LABELS[s.section]}</strong>
                                            <span class="warden-manager-dim"> ({s.count})</span>
                                            <ul>
                                                <For each={s.first}>
                                                    {(e) => (
                                                        <li>
                                                            {e.label} <span class="warden-manager-dim">{vaultKind(e.kind)?.label ?? e.kind}</span>
                                                            <Show when={e.duplicate}>
                                                                <span class="vault-dup"> · already in your Vault</span>
                                                            </Show>
                                                        </li>
                                                    )}
                                                </For>
                                                <Show when={s.count > s.first.length}>
                                                    <li class="warden-manager-dim">and {s.count - s.first.length} more</li>
                                                </Show>
                                            </ul>
                                        </div>
                                    )}
                                </For>
                                <Show when={preview().duplicates > 0}>
                                    <Field label={`${preview().duplicates} already in your Vault`} layout="stacked" description="The same value (or the same site and username, for a login).">
                                        <SegmentedControl options={DUPLICATE_CHOICES} value={duplicates()} onChange={setDuplicates} ariaLabel="What to do with entries already in the Vault" />
                                    </Field>
                                </Show>
                                <Show when={preview().skipped.length > 0}>
                                    <div class="vault-import-skipped">
                                        <strong>{preview().skipped.length} can't be imported</strong>
                                        <ul>
                                            <For each={preview().skipped}>
                                                {(k) => (
                                                    <li>
                                                        {k.label} <span class="warden-manager-dim">{k.reason}</span>
                                                    </li>
                                                )}
                                            </For>
                                        </ul>
                                    </div>
                                </Show>
                            </div>
                        )}
                    </Show>
                </Show>
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.onClose} disabled={busy() && !!result()}>
                    {result() ? "Close" : "Cancel"}
                </Button>
                <Show when={!result() && step()?.kind === "needs_passphrase"}>
                    <Button tone="accent" busy={busy()} disabled={!pass()} onClick={() => void unlock()}>
                        Open file
                    </Button>
                </Show>
                <Show when={!result() && step()?.kind === "preview" ? (step() as Extract<ImportStep, { kind: "preview" }>).preview : null}>
                    {(preview) => (
                        <Button tone="accent" busy={busy()} disabled={preview().total === 0} onClick={() => void commit(preview())}>
                            Import
                        </Button>
                    )}
                </Show>
            </ModalFooter>
        </Modal>
    );
}
