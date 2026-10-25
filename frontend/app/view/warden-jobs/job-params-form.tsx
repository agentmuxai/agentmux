// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The form for a template's parameters, made from its params schema: a string, a number, a boolean,
// a choice, a list of strings (one per line) and a credential, which names a Vault entry. A value is
// never typed here for a credential: only the entry's id is kept (§7, "Secrets through Vault only").

import { For, Show, type JSX } from "solid-js";

import { Field, Select, Switch, TextInput } from "@/app/element/ui";
import type { JobTemplateManifest } from "@/types/rpc/JobTemplateManifest";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

import { paramKind, paramNames, type ParamValue } from "./job-draft";

export function JobParamsForm(props: {
    manifest: JobTemplateManifest;
    values: Record<string, ParamValue>;
    errors: Record<string, string>;
    onChange: (name: string, value: ParamValue) => void;
    /** The Vault's entries, when the Vault can be read here; `null` otherwise. */
    vault: readonly VaultEntryInfo[] | null;
}): JSX.Element {
    const text = (name: string) => (typeof props.values[name] === "string" ? (props.values[name] as string) : "");
    return (
        <For each={paramNames(props.manifest)}>
            {(name) => {
                const p = props.manifest.params.properties[name];
                const kind = paramKind(p);
                const required = props.manifest.params.required.includes(name);
                const label = `${p.title ?? name}${required ? "" : " (optional)"}`;
                return (
                    <Field
                        label={label}
                        description={p.description ?? undefined}
                        hint={
                            kind === "credential" && !props.vault
                                ? "The Vault can't be read here: type the entry's id, never a secret itself."
                                : undefined
                        }
                        layout="stacked"
                        error={props.errors[name]}
                    >
                        <Show when={kind === "boolean"}>
                            <Switch checked={props.values[name] === true} onChange={(on) => props.onChange(name, on)} />
                        </Show>
                        <Show when={kind === "enum"}>
                            <Select
                                value={text(name)}
                                onChange={(v) => props.onChange(name, v)}
                                options={[
                                    ...(required ? [] : [{ value: "", label: "—" }]),
                                    ...(p.enum ?? []).map((v) => ({ value: v, label: v })),
                                ]}
                            />
                        </Show>
                        <Show when={kind === "list"}>
                            <textarea
                                class="ui-input warden-jobs-textarea"
                                rows={3}
                                aria-label={label}
                                placeholder="One per line"
                                value={text(name)}
                                onInput={(e) => props.onChange(name, e.currentTarget.value)}
                            />
                        </Show>
                        <Show when={kind === "credential"}>
                            <Show
                                when={props.vault}
                                fallback={
                                    <TextInput
                                        value={text(name)}
                                        placeholder="Vault entry id"
                                        onInput={(e) => props.onChange(name, e.currentTarget.value)}
                                    />
                                }
                            >
                                {(entries) => (
                                    <Select
                                        value={text(name)}
                                        onChange={(v) => props.onChange(name, v)}
                                        options={[
                                            {
                                                value: "",
                                                label:
                                                    entries().length > 0
                                                        ? "Choose a Vault entry"
                                                        : "The Vault has no entries",
                                            },
                                            ...entries().map((e) => ({ value: e.id, label: e.label })),
                                            ...(text(name) && !entries().some((e) => e.id === text(name))
                                                ? [{ value: text(name), label: `${text(name)} (not in the Vault)` }]
                                                : []),
                                        ]}
                                    />
                                )}
                            </Show>
                        </Show>
                        <Show when={kind === "string" || kind === "number"}>
                            <TextInput
                                type={kind === "number" ? "number" : "text"}
                                value={text(name)}
                                onInput={(e) => props.onChange(name, e.currentTarget.value)}
                            />
                        </Show>
                    </Field>
                );
            }}
        </For>
    );
}
