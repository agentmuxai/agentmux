// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Adding and changing a Vault entry. Secret parts are typed hidden and never
// shown back: on a change, an empty part keeps its value, and Clear empties
// one. The form lives inside the pane (scope "pane"), so the pane's own
// protections cover it.

import { type Accessor, createMemo, createSignal, For, type JSX, type Setter, Show } from "solid-js";
import { createStore, type SetStoreFunction } from "solid-js/store";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, Field, TextArea as KitTextArea, Select, TextInput } from "@/app/element/ui";
import { browserProfiles, ensureBrowserProfiles } from "@/app/view/browser/browser-profiles";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { getPlatform } from "@/util/platformutil";
import clsx from "clsx";
import { kindsIn, type VaultInput, vaultKind, type VaultKind } from "./vault-kinds";
import { firstSaveNotice } from "./vault-prompts";
import { createEntry, updateEntry, vaultEntries } from "./vault-store";
import { VaultField } from "./vault-vaults";
// Loaded with the form, which an agent's pane opens without a Vault pane: a
// secret note is masked as it is typed (`.vault-secret-text`).
import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./vault-view.scss";

export type VaultFormTarget =
    | { mode: "new"; section: VaultSection; request?: VaultRequest; vault?: string }
    | { mode: "edit"; entry: VaultEntryInfo };

function TextArea(props: {
    value: string;
    onInput: (v: string) => void;
    secret?: boolean;
    placeholder?: string;
    label: string;
}): JSX.Element {
    return (
        <KitTextArea
            variant="plain"
            class={clsx("ui-input vault-textarea", props.secret && "vault-secret-text")}
            rows={4}
            value={props.value}
            placeholder={props.placeholder}
            aria-label={props.label}
            spellcheck={false}
            autocomplete="off"
            onInput={(e) => props.onInput(e.currentTarget.value)}
        />
    );
}

/** Personal, and the named profiles; a deleted one still shows while an entry names it (`current`). */
function profileChoices(current: string | undefined) {
    const named = browserProfiles().map((p) => ({ value: `profile:${p.id}`, label: p.name }));
    const gone =
        current && current !== "personal" && !named.some((o) => o.value === current)
            ? [{ value: current, label: "A deleted profile" }]
            : [];
    return [{ value: "personal", label: "Personal" }, ...named, ...gone];
}

interface FormProps {
    target: VaultFormTarget;
    onClose: (savedId?: string) => void;
}

interface FormState {
    props: FormProps;
    t: VaultFormTarget;
    editing: VaultEntryInfo | null;
    request: VaultRequest | undefined;
    kindName: Accessor<string>;
    setKindName: Setter<string>;
    kind: Accessor<VaultKind | undefined>;
    label: Accessor<string>;
    setLabel: Setter<string>;
    fields: Record<string, string>;
    setFields: SetStoreFunction<Record<string, string>>;
    parts: Record<string, string>;
    setParts: SetStoreFunction<Record<string, string>>;
    clear: Record<string, boolean>;
    setClear: SetStoreFunction<Record<string, boolean>>;
    error: Accessor<string | null>;
    setError: Setter<string | null>;
    busy: Accessor<boolean>;
    setBusy: Setter<boolean>;
    vault: Accessor<string>;
    setVault: Setter<string>;
    kindOptions: () => { value: string; label: string }[];
    isSet: (part: string) => boolean;
    main: () => string;
}

function createFormState(props: FormProps): FormState {
    const t = props.target;
    const editing = t.mode === "edit" ? t.entry : null;
    const request = t.mode === "new" ? t.request : undefined;
    const initialKind = editing?.kind ?? request?.kind ?? (t.mode === "new" ? kindsIn(t.section)[0]?.name : "") ?? "";
    const [kindName, setKindName] = createSignal(initialKind);
    const kind = createMemo(() => vaultKind(kindName()));
    const [label, setLabel] = createSignal(editing?.label ?? request?.label ?? "");
    const [fields, setFields] = createStore<Record<string, string>>({ ...(editing?.fields ?? {}) });
    const [parts, setParts] = createStore<Record<string, string>>({});
    const [clear, setClear] = createStore<Record<string, boolean>>({});
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    // Where a new entry goes: the Vault the lists are on, the one an agent asked for, or Personal.
    const [vault, setVault] = createSignal((t.mode === "new" && (t.vault || t.request?.vault)) || "personal");

    ensureBrowserProfiles();
    const kindOptions = () =>
        (t.mode === "new" ? kindsIn(kind()?.section ?? t.section) : []).map((k) => ({ value: k.name, label: k.label }));
    const isSet = (part: string) => editing?.parts_set.includes(part) ?? false;
    const main = () => kind()?.parts[0]?.name ?? "";
    return {
        props,
        t,
        editing,
        request,
        kindName,
        setKindName,
        kind,
        label,
        setLabel,
        fields,
        setFields,
        parts,
        setParts,
        clear,
        setClear,
        error,
        setError,
        busy,
        setBusy,
        vault,
        setVault,
        kindOptions,
        isSet,
        main,
    };
}

/** Only this kind's inputs: switching kinds leaves the others' typing behind. */
function pick(src: Record<string, string>, inputs: VaultInput[]): Record<string, string> {
    return Object.fromEntries(inputs.map((i) => [i.name, src[i.name] ?? ""]).filter(([, v]) => v !== ""));
}

async function save(s: FormState) {
    const k = s.kind();
    if (!k) return;
    s.setBusy(true);
    s.setError(null);
    try {
        if (s.editing) {
            await updateEntry({
                id: s.editing.id,
                label: s.label(),
                fields: pick(s.fields, k.fields),
                tags: s.editing.tags,
                parts: pick(s.parts, k.parts),
                clear: k.parts.filter((p) => s.clear[p.name]).map((p) => p.name),
            });
            s.props.onClose(s.editing.id);
        } else {
            const res = await createEntry({
                kind: k.name,
                label: s.label(),
                fields: pick(s.fields, k.fields),
                parts: pick(s.parts, k.parts),
                request_id: s.request?.id,
                vault: s.vault(),
            });
            s.props.onClose(res.id);
        }
    } catch (e) {
        s.setError(e instanceof Error ? e.message : String(e));
    } finally {
        s.setBusy(false);
        // Typed values don't outlive the form's answer.
        for (const p of Object.keys(s.parts)) s.setParts(p, "");
    }
}

export function VaultEntryForm(props: FormProps): JSX.Element {
    const s = createFormState(props);
    const { editing, request } = s;
    const title = editing
        ? `Change ${editing.label}`
        : request
          ? `${request.agent} asks for: ${request.label}`
          : "Add to the Vault";

    return (
        <Modal
            open={true}
            scope="pane"
            onClose={() => props.onClose()}
            size="md"
            closeOnBackdropClick={false}
            panelClass="vault-form"
        >
            <ModalHeader
                title={title}
                description={
                    request?.why
                        ? `Why: ${request.why}`
                        : editing
                          ? "Values aren't shown. Type one to replace it; leave it empty to keep it."
                          : undefined
                }
            />
            <ModalBody>
                <FormFields s={s} />
                <FirstSaveNote s={s} />
                <Show when={s.error()}>
                    <div class="warden-manager-error" role="alert">
                        {s.error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={() => props.onClose()} disabled={s.busy()}>
                    Cancel
                </Button>
                <Button tone="accent" busy={s.busy()} onClick={() => void save(s)}>
                    {editing ? "Save changes" : "Save to Vault"}
                </Button>
            </ModalFooter>
        </Modal>
    );
}

function FormFields({ s }: { s: FormState }) {
    return (
        <form
            class="vault-form-fields"
            autocomplete="off"
            onSubmit={(e) => {
                e.preventDefault();
                void save(s);
            }}
        >
            <Show when={!s.editing && s.kindOptions().length > 1}>
                <Field label="Kind" layout="stacked">
                    <Select value={s.kindName()} onChange={s.setKindName} options={s.kindOptions()} />
                </Field>
            </Show>
            <Show when={!s.editing}>
                <VaultField
                    value={s.vault()}
                    onChange={s.setVault}
                    description="Which agents may use it is the Vault's setting."
                />
            </Show>
            <Field
                label="Label"
                layout="stacked"
                description="How you and agents name it. Agents see labels, never values."
            >
                <TextInput
                    value={s.label()}
                    onInput={(e) => s.setLabel(e.currentTarget.value)}
                    autofocus={!s.request}
                    maxLength={200}
                />
            </Field>
            <For each={s.kind()?.parts ?? []}>{(p) => <PartRow s={s} p={p} />}</For>
            <For each={(s.kind()?.fields ?? []).filter((f) => !f.kept && (!f.profile || !!s.fields.site?.trim()))}>
                {(f) => <FieldRow s={s} f={f} />}
            </For>
            {/* Enter in a field saves. */}
            <input type="submit" hidden />
        </form>
    );
}

function partHint(s: FormState, p: VaultInput): string | undefined {
    if (s.editing && s.isSet(p.name)) return s.clear[p.name] ? "Will be cleared." : "Set. Type to replace it.";
    return p.name === s.main() ? "Required." : undefined;
}

function PartRow({ s, p }: { s: FormState; p: VaultInput }) {
    return (
        <Field label={p.label} layout="stacked" hint={partHint(s, p)}>
            <div class="vault-part">
                <PartControl s={s} p={p} />
                <Show when={s.editing && s.isSet(p.name) && p.name !== s.main()}>
                    <Button
                        density="compact"
                        tone="quiet"
                        pressed={!!s.clear[p.name]}
                        onClick={() => s.setClear(p.name, !s.clear[p.name])}
                    >
                        Clear
                    </Button>
                </Show>
            </div>
        </Field>
    );
}

function PartControl({ s, p }: { s: FormState; p: VaultInput }) {
    return (
        <Show
            when={p.multiline}
            fallback={
                <TextInput
                    type="password"
                    autocomplete="new-password"
                    spellcheck={false}
                    value={s.parts[p.name] ?? ""}
                    placeholder={s.editing && s.isSet(p.name) ? "••••••••" : p.placeholder}
                    disabled={s.clear[p.name]}
                    autofocus={!!s.request && p.name === s.main()}
                    onInput={(e) => s.setParts(p.name, e.currentTarget.value)}
                />
            }
        >
            <TextArea
                secret
                label={p.label}
                value={s.parts[p.name] ?? ""}
                onInput={(v) => s.setParts(p.name, v)}
                placeholder={s.editing && s.isSet(p.name) ? "Set. Type to replace it." : p.placeholder}
            />
        </Show>
    );
}

function FieldRow({ s, f }: { s: FormState; f: VaultInput }) {
    return (
        <Field
            label={f.label}
            layout="stacked"
            description={f.profile ? "Only this profile's tabs are offered it." : undefined}
        >
            <Show
                when={!f.profile}
                fallback={
                    <Select
                        value={s.fields[f.name] || "personal"}
                        onChange={(v) => s.setFields(f.name, v)}
                        options={profileChoices(s.fields.profile)}
                    />
                }
            >
                <Show
                    when={f.multiline}
                    fallback={
                        <TextInput
                            value={s.fields[f.name] ?? ""}
                            placeholder={f.placeholder}
                            onInput={(e) => s.setFields(f.name, e.currentTarget.value)}
                        />
                    }
                >
                    <TextArea label={f.label} value={s.fields[f.name] ?? ""} onInput={(v) => s.setFields(f.name, v)} />
                </Show>
            </Show>
        </Field>
    );
}

/** The first save creates the Vault key: say what will ask, before it does. */
function FirstSaveNote({ s }: { s: FormState }) {
    return (
        <Show when={!s.editing && (vaultEntries() ?? []).length === 0 && firstSaveNotice(getPlatform())}>
            {(notice) => (
                <p class="vault-prompt-note" role="note">
                    {notice()}
                </p>
            )}
        </Show>
    );
}
