// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Adding and changing a Vault entry. Secret parts are typed hidden and never
// shown back: on a change, an empty part keeps its value, and Clear empties
// one. The form lives inside the pane (scope "pane"), so the pane's own
// protections cover it.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { createStore } from "solid-js/store";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, Field, Select, TextInput } from "@/app/element/ui";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { kindsIn, vaultKind, type VaultInput } from "./vault-kinds";
import { firstSaveNotice } from "./vault-prompts";
import { createEntry, updateEntry, vaultEntries } from "./vault-store";
import { getPlatform } from "@/util/platformutil";
import { browserProfiles, ensureBrowserProfiles } from "@/app/view/browser/browser-profiles";

export type VaultFormTarget =
    | { mode: "new"; section: VaultSection; request?: VaultRequest }
    | { mode: "edit"; entry: VaultEntryInfo };

function TextArea(props: { value: string; onInput: (v: string) => void; secret?: boolean; placeholder?: string; label: string }): JSX.Element {
    return (
        <textarea
            class="ui-input vault-textarea"
            classList={{ "vault-secret-text": props.secret }}
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

export function VaultEntryForm(props: { target: VaultFormTarget; onClose: (savedId?: string) => void }): JSX.Element {
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

    ensureBrowserProfiles();
    // Personal, and the named profiles; a deleted one still shows while an entry names it.
    const profileOptions = () => {
        const named = browserProfiles().map((p) => ({ value: `profile:${p.id}`, label: p.name }));
        const now = fields.profile;
        const gone = now && now !== "personal" && !named.some((o) => o.value === now) ? [{ value: now, label: "A deleted profile" }] : [];
        return [{ value: "personal", label: "Personal" }, ...named, ...gone];
    };
    const kindOptions = () => (t.mode === "new" ? kindsIn(kind()?.section ?? t.section) : []).map((k) => ({ value: k.name, label: k.label }));
    const isSet = (part: string) => editing?.parts_set.includes(part) ?? false;
    const main = () => kind()?.parts[0]?.name ?? "";

    const save = async () => {
        const k = kind();
        if (!k) return;
        setBusy(true);
        setError(null);
        // Only this kind's inputs: switching kinds leaves the others' typing behind.
        const pick = (src: Record<string, string>, inputs: VaultInput[]) =>
            Object.fromEntries(inputs.map((i) => [i.name, src[i.name] ?? ""]).filter(([, v]) => v !== ""));
        try {
            if (editing) {
                await updateEntry({
                    id: editing.id,
                    label: label(),
                    fields: pick(fields, k.fields),
                    tags: editing.tags,
                    parts: pick(parts, k.parts),
                    clear: k.parts.filter((p) => clear[p.name]).map((p) => p.name),
                });
                props.onClose(editing.id);
            } else {
                const res = await createEntry({
                    kind: k.name,
                    label: label(),
                    fields: pick(fields, k.fields),
                    parts: pick(parts, k.parts),
                    request_id: request?.id,
                });
                props.onClose(res.id);
            }
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
            // Typed values don't outlive the form's answer.
            for (const p of Object.keys(parts)) setParts(p, "");
        }
    };

    const title = editing ? `Change ${editing.label}` : request ? `${request.agent} asks for: ${request.label}` : "Add to the Vault";

    return (
        <Modal open={true} scope="pane" onClose={() => props.onClose()} size="md" closeOnBackdropClick={false} panelClass="vault-form">
            <ModalHeader
                title={title}
                description={request?.why ? `Why: ${request.why}` : editing ? "Values aren't shown. Type one to replace it; leave it empty to keep it." : undefined}
            />
            <ModalBody>
                <form
                    class="vault-form-fields"
                    autocomplete="off"
                    onSubmit={(e) => {
                        e.preventDefault();
                        void save();
                    }}
                >
                    <Show when={!editing && kindOptions().length > 1}>
                        <Field label="Kind" layout="stacked">
                            <Select value={kindName()} onChange={setKindName} options={kindOptions()} />
                        </Field>
                    </Show>
                    <Field label="Label" layout="stacked" description="How you and agents name it. Agents see labels, never values.">
                        <TextInput value={label()} onInput={(e) => setLabel(e.currentTarget.value)} autofocus={!request} maxLength={200} />
                    </Field>
                    <For each={kind()?.parts ?? []}>
                        {(p) => (
                            <Field
                                label={p.label}
                                layout="stacked"
                                hint={
                                    editing && isSet(p.name)
                                        ? clear[p.name]
                                            ? "Will be cleared."
                                            : "Set. Type to replace it."
                                        : p.name === main()
                                          ? "Required."
                                          : undefined
                                }
                            >
                                <div class="vault-part">
                                    <Show
                                        when={p.multiline}
                                        fallback={
                                            <TextInput
                                                type="password"
                                                autocomplete="new-password"
                                                spellcheck={false}
                                                value={parts[p.name] ?? ""}
                                                placeholder={editing && isSet(p.name) ? "••••••••" : p.placeholder}
                                                disabled={clear[p.name]}
                                                autofocus={!!request && p.name === main()}
                                                onInput={(e) => setParts(p.name, e.currentTarget.value)}
                                            />
                                        }
                                    >
                                        <TextArea secret label={p.label} value={parts[p.name] ?? ""} onInput={(v) => setParts(p.name, v)} placeholder={editing && isSet(p.name) ? "Set. Type to replace it." : p.placeholder} />
                                    </Show>
                                    <Show when={editing && isSet(p.name) && p.name !== main()}>
                                        <Button density="compact" tone="quiet" pressed={!!clear[p.name]} onClick={() => setClear(p.name, !clear[p.name])}>
                                            Clear
                                        </Button>
                                    </Show>
                                </div>
                            </Field>
                        )}
                    </For>
                    <For each={(kind()?.fields ?? []).filter((f) => !f.kept && (!f.profile || !!fields.site?.trim()))}>
                        {(f) => (
                            <Field label={f.label} layout="stacked" description={f.profile ? "Only this profile's tabs are offered it." : undefined}>
                                <Show when={!f.profile} fallback={<Select value={fields[f.name] || "personal"} onChange={(v) => setFields(f.name, v)} options={profileOptions()} />}>
                                    <Show
                                        when={f.multiline}
                                        fallback={<TextInput value={fields[f.name] ?? ""} placeholder={f.placeholder} onInput={(e) => setFields(f.name, e.currentTarget.value)} />}
                                    >
                                        <TextArea label={f.label} value={fields[f.name] ?? ""} onInput={(v) => setFields(f.name, v)} />
                                    </Show>
                                </Show>
                            </Field>
                        )}
                    </For>
                    {/* Enter in a field saves. */}
                    <input type="submit" hidden />
                </form>
                {/* The first save creates the Vault key: say what will ask, before it does. */}
                <Show when={!editing && (vaultEntries() ?? []).length === 0 && firstSaveNotice(getPlatform())}>
                    {(notice) => (
                        <p class="vault-prompt-note" role="note">
                            {notice()}
                        </p>
                    )}
                </Show>
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={() => props.onClose()} disabled={busy()}>
                    Cancel
                </Button>
                <Button tone="accent" busy={busy()} onClick={() => void save()}>
                    {editing ? "Save changes" : "Save to Vault"}
                </Button>
            </ModalFooter>
        </Modal>
    );
}
