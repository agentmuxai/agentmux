// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Import from a `.vault` file
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.4, §8.5).
// The host asks which file; srv decrypts it into memory and describes it with no
// values; nothing is written until the person presses Import, and then all of it
// is written or none of it.

import { createSignal, For, onCleanup, onMount, Show, type Accessor, type JSX, type Setter } from "solid-js";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, Field, SegmentedControl, Select, TextInput } from "@/app/element/ui";
import type { ImportPreview } from "@/types/rpc/ImportPreview";
import type { ImportResult } from "@/types/rpc/ImportResult";
import { SECTION_LABELS, VAULT_KINDS, vaultKind } from "./vault-kinds";
import {
    cancelImport,
    commitImport,
    deleteImportFile,
    pickImport,
    type CsvMapping,
    type Duplicates,
    type ImportStep,
} from "./vault-store";
import { VaultField } from "./vault-vaults";

/** Files that are encrypted but can't be a KeePass database, which is the only kind with a key file. The
 *  host recognises a database by its contents, so a renamed one (`backup.bin`) still gets the key-file row. */
const KNOWN_NOT_KEEPASS = /\.(vault|json)$/i;

/** The kinds a row of an unknown CSV can become, each with what its main column is called. */
const MAPPABLE = VAULT_KINDS.filter((k) => k.name !== "card" && k.name !== "bank-account").map((k) => ({
    value: k.name,
    label: k.label,
    main: k.parts[0].label,
}));

const DUPLICATE_CHOICES: { value: Duplicates; label: string }[] = [
    { value: "skip", label: "Skip them" },
    { value: "replace", label: "Replace mine" },
    { value: "keep_both", label: "Keep both" },
];

/** What to do with entries the file shares with the Vault; nothing is asked when there are none. */
function DuplicatesChoice(props: { count: number; value: Duplicates; onChange: (d: Duplicates) => void }): JSX.Element {
    return (
        <Show when={props.count > 0}>
            <Field
                label={`${props.count} already in your Vault`}
                layout="stacked"
                description="The same value (or the same site and username, for a login)."
            >
                <SegmentedControl
                    options={DUPLICATE_CHOICES}
                    value={props.value}
                    onChange={props.onChange}
                    ariaLabel="What to do with entries already in the Vault"
                />
            </Field>
        </Show>
    );
}

type Step<K extends ImportStep["kind"]> = Extract<ImportStep, { kind: K }>;

/** The dialog's signals, the column choices for a CSV no reader knows, and the token srv holds the file by. */
interface ImportState {
    onClose: () => void;
    step: Accessor<ImportStep | null>;
    setStep: Setter<ImportStep | null>;
    pass: Accessor<string>;
    setPass: Setter<string>;
    duplicates: Accessor<Duplicates>;
    setDuplicates: Setter<Duplicates>;
    into: Accessor<string>;
    setInto: Setter<string>;
    busy: Accessor<boolean>;
    setBusy: Setter<boolean>;
    error: Accessor<string | null>;
    setError: Setter<string | null>;
    result: Accessor<ImportResult | null>;
    setResult: Setter<ImportResult | null>;
    deleted: Accessor<boolean>;
    setDeleted: Setter<boolean>;
    mapKind: Accessor<string>;
    setMapKind: Setter<string>;
    mapLabel: Accessor<string>;
    setMapLabel: Setter<string>;
    mapValue: Accessor<string>;
    setMapValue: Setter<string>;
    mapSite: Accessor<string>;
    setMapSite: Setter<string>;
    mapUser: Accessor<string>;
    setMapUser: Setter<string>;
    token: string;
}

function createImportState(props: { onClose: () => void; into?: string }): ImportState {
    const [step, setStep] = createSignal<ImportStep | null>(null);
    const [pass, setPass] = createSignal("");
    const [duplicates, setDuplicates] = createSignal<Duplicates>("skip");
    const [into, setInto] = createSignal(props.into || "personal");
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
    return {
        onClose: props.onClose,
        step,
        setStep,
        pass,
        setPass,
        duplicates,
        setDuplicates,
        into,
        setInto,
        busy,
        setBusy,
        error,
        setError,
        result,
        setResult,
        deleted,
        setDeleted,
        mapKind,
        setMapKind,
        mapLabel,
        setMapLabel,
        mapValue,
        setMapValue,
        mapSite,
        setMapSite,
        mapUser,
        setMapUser,
        token: "",
    };
}

/** The step when it is of this kind. */
function stepOf<K extends ImportStep["kind"]>(s: ImportState, kind: K): Step<K> | null {
    const cur = s.step();
    return cur?.kind === kind ? (cur as Step<K>) : null;
}

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

function applyStep(s: ImportState, next: ImportStep | null) {
    if (next === null) {
        // The person closed the file dialog.
        s.onClose();
        return;
    }
    s.setStep(next);
    if (next.kind === "preview") s.token = next.preview.token;
}

async function run(s: ImportState, what: () => Promise<ImportStep | null>) {
    s.setBusy(true);
    s.setError(null);
    try {
        applyStep(s, await what());
    } catch (e) {
        s.setError(errorText(e));
    } finally {
        s.setBusy(false);
    }
}

function unlock(s: ImportState) {
    const cur = stepOf(s, "needs_passphrase");
    if (!cur) return;
    const typed = s.pass();
    s.setPass("");
    return run(s, () => pickImport(cur.pickId, typed));
}

// A KeePass database may need a key file as well as, or instead of, its password: the host
// asks which in its own window, and only the file's name comes back here.
function keyFile(s: ImportState, what: "choose" | "clear") {
    const cur = stepOf(s, "needs_passphrase");
    if (!cur) return;
    return run(s, () => pickImport(cur.pickId, undefined, undefined, what));
}

function map(s: ImportState) {
    const cur = stepOf(s, "needs_mapping");
    if (!cur) return;
    const mapping: CsvMapping = { kind: s.mapKind(), label: Number(s.mapLabel()), value: Number(s.mapValue()) };
    if (s.mapSite() !== "") mapping.site = Number(s.mapSite());
    if (s.mapUser() !== "") mapping.username = Number(s.mapUser());
    return run(s, () => pickImport(cur.pickId, undefined, mapping));
}

async function removeFile(s: ImportState) {
    const cur = s.step();
    if (!cur) return;
    try {
        await deleteImportFile(cur.pickId);
        s.setDeleted(true);
    } catch (e) {
        s.setError(errorText(e));
    }
}

async function commit(s: ImportState, preview: ImportPreview) {
    s.setBusy(true);
    s.setError(null);
    try {
        s.setResult(await commitImport(preview.token, s.duplicates(), s.into()));
    } catch (e) {
        s.setError(errorText(e));
    } finally {
        s.setBusy(false);
    }
}

function title(s: ImportState): string {
    const cur = s.step();
    return cur ? `Import ${cur.kind === "preview" ? cur.preview.file_name : cur.fileName}` : "Import into the Vault";
}

/** `into` is the Vault the new entries go to when the dialog opens (Personal when left out). */
export function VaultImportDialog(props: { onClose: () => void; into?: string }): JSX.Element {
    const s = createImportState(props);

    // Opening the dialog asks which file.
    onMount(() => void run(s, () => pickImport()));
    // Closing without importing drops what srv holds in memory.
    onCleanup(() => {
        if (s.token && !s.result()) void cancelImport(s.token);
    });

    return (
        <Modal
            open={true}
            scope="pane"
            onClose={props.onClose}
            size="md"
            closeOnBackdropClick={false}
            panelClass="vault-form"
        >
            <ModalHeader title={title(s)} description="Nothing is added until you press Import." />
            <ModalBody>
                <Show when={s.result()}>{(r) => <ImportedView s={s} r={r} />}</Show>
                <Show when={!s.result()}>
                    <Show when={s.busy() && !s.step()}>
                        <p class="warden-manager-dim">Choose the file in the window that opened…</p>
                    </Show>
                    <Show when={stepOf(s, "needs_passphrase")}>{(cur) => <PassphraseStep s={s} cur={cur} />}</Show>
                    <Show when={stepOf(s, "needs_mapping")}>{(cur) => <MappingStep s={s} cur={cur} />}</Show>
                    <Show when={stepOf(s, "preview")?.preview}>
                        {(preview) => <PreviewStep s={s} preview={preview} />}
                    </Show>
                </Show>
                <Show when={s.error()}>
                    <div class="warden-manager-error" role="alert">
                        {s.error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.onClose} disabled={s.busy() && !!s.result()}>
                    {s.result() ? "Close" : "Cancel"}
                </Button>
                <FooterActions s={s} />
            </ModalFooter>
        </Modal>
    );
}

/** What came of the import, and what to do about a file that holds every secret unencrypted. */
function ImportedView(props: { s: ImportState; r: Accessor<ImportResult> }): JSX.Element {
    const s = props.s;
    const previewStep = () => stepOf(s, "preview");
    return (
        <div role="status">
            <p>
                Imported: {props.r().added} added, {props.r().replaced} replaced, {props.r().skipped} skipped.
            </p>
            <Show
                when={previewStep()?.preview.plaintext_file}
                fallback={<p class="warden-manager-dim">The file stays where it is. Delete it if you don't need it.</p>}
            >
                <p class="warden-manager-dim">
                    {
                        "That file holds every secret in it unencrypted, so anything that can read your disk can read it. Delete it once you've checked the import."
                    }
                </p>
                <Show when={previewStep()?.syncedFolder}>
                    <p class="warden-manager-error" role="alert">
                        It is in your {previewStep()!.syncedFolder} folder: a copy may already be in the cloud, and in
                        that service's file history. Deleting it here doesn't remove those.
                    </p>
                </Show>
                <Show
                    when={!s.deleted()}
                    fallback={
                        <p role="status">
                            Deleted. (A deleted file isn't securely erased: on an SSD an overwrite doesn't reliably
                            reach the old blocks.)
                        </p>
                    }
                >
                    <Button tone="danger" density="compact" icon="trash" onClick={() => void removeFile(s)}>
                        Delete the file
                    </Button>
                </Show>
            </Show>
        </div>
    );
}

function PassphraseStep(props: { s: ImportState; cur: Accessor<Step<"needs_passphrase">> }): JSX.Element {
    const s = props.s;
    return (
        <>
            <form
                onSubmit={(e) => {
                    e.preventDefault();
                    void unlock(s);
                }}
            >
                <Field
                    label="Passphrase"
                    layout="stacked"
                    description="This file is encrypted. Type the passphrase it was exported with: a Vault file's, a KeePass database's master password, or the password a Bitwarden export was protected with."
                >
                    <TextInput
                        type="password"
                        autocomplete="new-password"
                        spellcheck={false}
                        autofocus
                        value={s.pass()}
                        onInput={(e) => s.setPass(e.currentTarget.value)}
                    />
                </Field>
                <input type="submit" hidden />
            </form>
            <Show when={!KNOWN_NOT_KEEPASS.test(props.cur().fileName)}>
                <KeyFileRow s={s} cur={props.cur} />
            </Show>
        </>
    );
}

function KeyFileRow(props: { s: ImportState; cur: Accessor<Step<"needs_passphrase">> }): JSX.Element {
    const s = props.s;
    return (
        <div class="vault-keyfile">
            <span class="warden-manager-dim">A KeePass database can also need a key file.</span>{" "}
            <Show
                when={props.cur().keyFileName}
                fallback={
                    <Button density="compact" disabled={s.busy()} onClick={() => void keyFile(s, "choose")}>
                        Choose a key file…
                    </Button>
                }
            >
                {(name) => (
                    <>
                        <span class="warden-manager-mono">{name()}</span>{" "}
                        <Button density="compact" disabled={s.busy()} onClick={() => void keyFile(s, "clear")}>
                            Remove
                        </Button>
                    </>
                )}
            </Show>
        </div>
    );
}

/** The columns of a CSV as choices, a column with no heading called by its place. */
const columnOptions = (columns: string[]) =>
    columns.map((c, i) => ({ value: String(i), label: c || `Column ${i + 1}` }));

function MappingStep(props: { s: ImportState; cur: Accessor<Step<"needs_mapping">> }): JSX.Element {
    const s = props.s;
    const columns = () => columnOptions(props.cur().columns);
    return (
        <div class="vault-import-mapping">
            <p>
                {"AgentMux doesn't know this file's format. Say what each column is: it has "}
                <strong>{props.cur().rows}</strong> rows.
            </p>
            <Field label="Each row becomes" layout="stacked">
                <Select
                    value={s.mapKind()}
                    onChange={s.setMapKind}
                    options={MAPPABLE.map((k) => ({ value: k.value, label: k.label }))}
                />
            </Field>
            <Field label="Label (its name)" layout="stacked">
                <Select value={s.mapLabel()} onChange={s.setMapLabel} options={columns()} />
            </Field>
            <Field label={MAPPABLE.find((k) => k.value === s.mapKind())?.main ?? "Value"} layout="stacked">
                <Select value={s.mapValue()} onChange={s.setMapValue} options={columns()} />
            </Field>
            <Show when={s.mapKind() === "login"}>
                <Field label="Site (optional)" layout="stacked">
                    <Select
                        value={s.mapSite()}
                        onChange={s.setMapSite}
                        options={[{ value: "", label: "none" }, ...columns()]}
                    />
                </Field>
                <Field label="Username (optional)" layout="stacked">
                    <Select
                        value={s.mapUser()}
                        onChange={s.setMapUser}
                        options={[{ value: "", label: "none" }, ...columns()]}
                    />
                </Field>
            </Show>
        </div>
    );
}

function PreviewStep(props: { s: ImportState; preview: Accessor<ImportPreview> }): JSX.Element {
    const s = props.s;
    const preview = props.preview;
    return (
        <div class="vault-import-preview">
            <p>
                <strong>{preview().total}</strong> {preview().total === 1 ? "entry" : "entries"} to import, from{" "}
                {preview().source_label}.
            </p>
            <For each={preview().sections}>{(section) => <SectionPreview section={section} />}</For>
            <VaultField
                label="Import into"
                value={s.into()}
                onChange={s.setInto}
                description="New entries go to this Vault. One you replace stays in the Vault it is in."
            />
            <DuplicatesChoice count={preview().duplicates} value={s.duplicates()} onChange={s.setDuplicates} />
            <Show when={preview().skipped.length > 0}>
                <SkippedList skipped={preview().skipped} />
            </Show>
        </div>
    );
}

function SectionPreview(props: { section: ImportPreview["sections"][number] }): JSX.Element {
    const sec = () => props.section;
    return (
        <div class="vault-import-section">
            <strong>{SECTION_LABELS[sec().section]}</strong>
            <span class="warden-manager-dim"> ({sec().count})</span>
            <ul>
                <For each={sec().first}>
                    {(e) => (
                        <li>
                            {e.label} <span class="warden-manager-dim">{vaultKind(e.kind)?.label ?? e.kind}</span>
                            <Show when={e.duplicate}>
                                <span class="vault-dup"> · already in your Vault</span>
                            </Show>
                        </li>
                    )}
                </For>
                <Show when={sec().count > sec().first.length}>
                    <li class="warden-manager-dim">and {sec().count - sec().first.length} more</li>
                </Show>
            </ul>
        </div>
    );
}

function SkippedList(props: { skipped: ImportPreview["skipped"] }): JSX.Element {
    return (
        <div class="vault-import-skipped">
            <strong>{props.skipped.length} can't be imported</strong>
            <ul>
                <For each={props.skipped}>
                    {(k) => (
                        <li>
                            {k.label} <span class="warden-manager-dim">{k.reason}</span>
                        </li>
                    )}
                </For>
            </ul>
        </div>
    );
}

/** The button that goes on to the next step, for the step the dialog is at. */
function FooterActions(props: { s: ImportState }): JSX.Element {
    const s = props.s;
    return (
        <>
            <Show when={!s.result() && s.step()?.kind === "needs_mapping"}>
                <Button tone="accent" busy={s.busy()} onClick={() => void map(s)}>
                    Continue
                </Button>
            </Show>
            <Show when={!s.result() && stepOf(s, "needs_passphrase")}>
                {(cur) => (
                    <Button
                        tone="accent"
                        busy={s.busy()}
                        disabled={!s.pass() && !cur().keyFileName}
                        onClick={() => void unlock(s)}
                    >
                        Open file
                    </Button>
                )}
            </Show>
            <Show when={!s.result() && stepOf(s, "preview")?.preview}>
                {(preview) => (
                    <Button
                        tone="accent"
                        busy={s.busy()}
                        disabled={preview().total === 0}
                        onClick={() => void commit(s, preview())}
                    >
                        Import
                    </Button>
                )}
            </Show>
        </>
    );
}
