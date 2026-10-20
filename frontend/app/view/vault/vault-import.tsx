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
import { Button, Field, SegmentedControl, Select, TextInput } from "@/app/element/ui";
import type { ImportPreview } from "@/types/rpc/ImportPreview";
import type { ImportResult } from "@/types/rpc/ImportResult";
import { SECTION_LABELS, VAULT_KINDS, vaultKind } from "./vault-kinds";
import { cancelImport, commitImport, deleteImportFile, pickImport, type CsvMapping, type Duplicates, type ImportStep } from "./vault-store";

/** The kinds a row of an unknown CSV can become, each with what its main column is called. */
const MAPPABLE = VAULT_KINDS.filter((k) => k.name !== "card" && k.name !== "bank-account").map((k) => ({ value: k.name, label: k.label, main: k.parts[0].label }));

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
    const [deleted, setDeleted] = createSignal(false);
    // The column choices for a CSV no reader knows.
    const [mapKind, setMapKind] = createSignal("api-key");
    const [mapLabel, setMapLabel] = createSignal("0");
    const [mapValue, setMapValue] = createSignal("1");
    const [mapSite, setMapSite] = createSignal("");
    const [mapUser, setMapUser] = createSignal("");
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

    const map = () => {
        const s = step();
        if (s?.kind !== "needs_mapping") return;
        const mapping: CsvMapping = { kind: mapKind(), label: Number(mapLabel()), value: Number(mapValue()) };
        if (mapSite() !== "") mapping.site = Number(mapSite());
        if (mapUser() !== "") mapping.username = Number(mapUser());
        return run(() => pickImport(s.pickId, undefined, mapping));
    };

    const removeFile = async () => {
        const s = step();
        if (!s) return;
        try {
            await deleteImportFile(s.pickId);
            setDeleted(true);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
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
    const previewStep = () => {
        const s = step();
        return s?.kind === "preview" ? s : null;
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
                            <Show when={previewStep()?.preview.plaintext_file} fallback={<p class="warden-manager-dim">The file stays where it is. Delete it if you don't need it.</p>}>
                                <p class="warden-manager-dim">
                                    {"That file holds every secret in it unencrypted, so anything that can read your disk can read it. Delete it once you've checked the import."}
                                </p>
                                <Show when={previewStep()?.syncedFolder}>
                                    <p class="warden-manager-error" role="alert">
                                        It is in your {previewStep()!.syncedFolder} folder: a copy may already be in the cloud, and in that service's file history. Deleting it here doesn't remove those.
                                    </p>
                                </Show>
                                <Show
                                    when={!deleted()}
                                    fallback={<p role="status">Deleted. (A deleted file isn't securely erased: on an SSD an overwrite doesn't reliably reach the old blocks.)</p>}
                                >
                                    <Button tone="danger" density="compact" icon="trash" onClick={() => void removeFile()}>
                                        Delete the file
                                    </Button>
                                </Show>
                            </Show>
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
                    <Show when={step()?.kind === "needs_mapping" ? (step() as Extract<ImportStep, { kind: "needs_mapping" }>) : null}>
                        {(s) => (
                            <div class="vault-import-mapping">
                                <p>
                                    {"AgentMux doesn't know this file's format. Say what each column is: it has "}
                                    <strong>{s().rows}</strong> rows.
                                </p>
                                <Field label="Each row becomes" layout="stacked">
                                    <Select value={mapKind()} onChange={setMapKind} options={MAPPABLE.map((k) => ({ value: k.value, label: k.label }))} />
                                </Field>
                                <Field label="Label (its name)" layout="stacked">
                                    <Select value={mapLabel()} onChange={setMapLabel} options={s().columns.map((c, i) => ({ value: String(i), label: c || `Column ${i + 1}` }))} />
                                </Field>
                                <Field label={MAPPABLE.find((k) => k.value === mapKind())?.main ?? "Value"} layout="stacked">
                                    <Select value={mapValue()} onChange={setMapValue} options={s().columns.map((c, i) => ({ value: String(i), label: c || `Column ${i + 1}` }))} />
                                </Field>
                                <Show when={mapKind() === "login"}>
                                    <Field label="Site (optional)" layout="stacked">
                                        <Select value={mapSite()} onChange={setMapSite} options={[{ value: "", label: "none" }, ...s().columns.map((c, i) => ({ value: String(i), label: c || `Column ${i + 1}` }))]} />
                                    </Field>
                                    <Field label="Username (optional)" layout="stacked">
                                        <Select value={mapUser()} onChange={setMapUser} options={[{ value: "", label: "none" }, ...s().columns.map((c, i) => ({ value: String(i), label: c || `Column ${i + 1}` }))]} />
                                    </Field>
                                </Show>
                            </div>
                        )}
                    </Show>
                    <Show when={step()?.kind === "preview" ? (step() as Extract<ImportStep, { kind: "preview" }>).preview : null}>
                        {(preview) => (
                            <div class="vault-import-preview">
                                <p>
                                    <strong>{preview().total}</strong> {preview().total === 1 ? "entry" : "entries"} to import, from {preview().source_label}.
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
                <Show when={!result() && step()?.kind === "needs_mapping"}>
                    <Button tone="accent" busy={busy()} onClick={() => void map()}>
                        Continue
                    </Button>
                </Show>
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
