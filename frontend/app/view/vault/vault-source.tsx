// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An entry's machine copy: where a copy of its value lives outside the Vault (a
// secret store a service reads, a CI secret), and what the newest check of it
// found. The link is plain fields (`source_*`) every kind of entry may have;
// srv (crates/srv-core/src/backend/vault/source_link.rs) is the source for what they
// may hold. Nothing here is a value.

import { Show, type JSX } from "solid-js";
import type { SetStoreFunction } from "solid-js/store";

import { Field, Select, TextInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { formatWhen } from "./vault-format";
import { isLinked, uncheckedDays } from "./vault-linked";

export const SOURCE_FIELDS = ["source_kind", "source_location", "source_key", "source_region", "source_profile"] as const;

const AWS = "aws-secret";
const GITHUB = "github-secret";

const KIND_OPTIONS = [
    { value: "", label: "None" },
    { value: AWS, label: "AWS Secrets Manager" },
    { value: GITHUB, label: "GitHub Actions secret" },
    { value: "other", label: "Somewhere else" },
];

/** The sections whose form offers a machine copy: an ID number or a card has none. */
const OFFERED_IN: VaultSection[] = ["api-keys", "logins", "notes"];

/** Whether the form shows the machine copy's inputs: the section offers them, or the entry already has a link. */
export function offersSource(section: VaultSection | undefined, fields: Record<string, string>): boolean {
    return (!!section && OFFERED_IN.includes(section)) || !!fields.source_kind;
}

/** The link's fields as they are saved: none without a kind, and a region and profile only for an AWS secret. */
export function sourceFields(fields: Record<string, string>): Record<string, string> {
    const kind = (fields.source_kind ?? "").trim();
    if (!kind) return {};
    const names = kind === AWS ? SOURCE_FIELDS : SOURCE_FIELDS.slice(0, 3);
    return Object.fromEntries(names.map((n) => [n, (fields[n] ?? "").trim()]).filter(([, v]) => v !== ""));
}

/** The place in a line, as a person reads it; empty when the entry has no link. */
export function machineCopy(fields: Record<string, string>): string {
    const { source_kind: kind, source_location: location = "", source_key: key = "" } = fields;
    if (!kind) return "";
    if (kind === AWS) return `AWS secret ${location}${key ? `, key ${key}` : ""}`;
    if (kind === GITHUB) return `GitHub secret ${key} of ${location}`;
    return key ? `${location}, ${key}` : location;
}

export interface SourceCheck {
    /** `same`, `different`, or `unread` for a place that could not be read; for a linked entry, whose place is
     *  described and not read, `present`, `changed` or `missing`. */
    state: "same" | "different" | "unread" | "present" | "changed" | "missing";
    /** What it found and when, and for a place that could not be read, why: the line says it, not a tooltip. */
    text: string;
}

/** What the newest check of the entry's machine copy found, and when; null when it was never checked. */
export function sourceCheck(e: Pick<VaultEntryInfo, "source_checked">, now = Date.now()): SourceCheck | null {
    const checked = e.source_checked;
    if (!checked) return null;
    const when = formatWhen(checked.at, now);
    if (checked.outcome === "same") return { state: "same", text: `same as the Vault's value, checked ${when}` };
    if (checked.outcome === "different") return { state: "different", text: `different from the Vault's value, checked ${when}` };
    if (checked.outcome === "present") return { state: "present", text: t("vault.linked.present", { when }) };
    for (const [word, state, key] of [
        ["changed since recorded", "changed", "vault.linked.changed"],
        ["missing", "missing", "vault.linked.missing"],
    ] as const) {
        if (!checked.outcome.startsWith(word)) continue;
        const said = checked.outcome.slice(word.length).replace(/^:\s*/, "");
        return { state, text: `${t(key, { when })}${said ? `: ${said}` : ""}` };
    }
    const why = checked.outcome.replace(/^could not read:?\s*/, "");
    return { state: "unread", text: `could not be read, checked ${when}${why ? `: ${why}` : ""}` };
}

/**
 * The entry's machine copy and its newest check, under its other details; `children` is the line's own action,
 * which stays beside the text however the text wraps. A copy that differs and one that could not be read are both
 * marked: neither is known to hold the entry's value.
 */
export function MachineCopyLine(props: { entry: VaultEntryInfo; children?: JSX.Element }): JSX.Element {
    return (
        <Show when={machineCopy(props.entry.fields)}>
            {(place) => (
                <div class="vault-machine-copy">
                    <span class="vault-machine-copy-text">
                        {isLinked(props.entry.fields) ? t("vault.linked.keptAt") : t("vault.machineCopy.line")} <span class="warden-manager-mono vault-import-id">{place()}</span>
                        <Show when={sourceCheck(props.entry)} fallback={<span class="vault-machine-copy-check"> · {t("vault.machineCopy.notChecked")}</span>}>
                            {(check) => (
                                <span class={["same", "present"].includes(check().state) ? "vault-machine-copy-check" : "vault-machine-copy-check vault-expiry-warn"} data-check={check().state}>
                                    {" "}
                                    · {check().text}
                                </span>
                            )}
                        </Show>
                        <Show when={uncheckedDays(props.entry)}>
                            {(days) => (
                                <span class="vault-machine-copy-check vault-expiry-warn" data-unchecked={days()}>
                                    {" "}
                                    · {t("vault.linked.unchecked", { count: days() })}
                                </span>
                            )}
                        </Show>
                    </span>
                    {props.children}
                </div>
            )}
        </Show>
    );
}

const LOCATION: Record<string, { label: string; placeholder: string }> = {
    [AWS]: { label: "Secret", placeholder: "its name or ARN, e.g. staging/app" },
    [GITHUB]: { label: "Repository", placeholder: "owner/repo, or owner/repo@environment" },
    other: { label: "Where", placeholder: "e.g. the build server's settings page" },
};
const KEY: Record<string, { label: string; placeholder: string }> = {
    [AWS]: { label: "Key in the secret", placeholder: "e.g. db.password; empty for the whole secret" },
    [GITHUB]: { label: "Secret name", placeholder: "e.g. DEPLOY_TOKEN" },
    other: { label: "Name there", placeholder: "optional" },
};

/** The form's inputs for the link: the kind of place, then what that kind needs. */
export function MachineCopyFields(props: { fields: Record<string, string>; setFields: SetStoreFunction<Record<string, string>> }): JSX.Element {
    const kind = () => props.fields.source_kind ?? "";
    const input = (name: string, placeholder: string) => (
        <TextInput
            value={props.fields[name] ?? ""}
            placeholder={placeholder}
            spellcheck={false}
            autocomplete="off"
            onInput={(e) => props.setFields(name, e.currentTarget.value)}
        />
    );
    return (
        <>
            <Field
                label={t("vault.machineCopy.label")}
                layout="stacked"
                description={t("vault.machineCopy.description")}
            >
                <Select value={kind()} onChange={(v) => props.setFields("source_kind", v)} options={KIND_OPTIONS} />
            </Field>
            <Show when={kind()}>
                <Field label={(LOCATION[kind()] ?? LOCATION.other).label} layout="stacked">
                    {input("source_location", (LOCATION[kind()] ?? LOCATION.other).placeholder)}
                </Field>
                <Field label={(KEY[kind()] ?? KEY.other).label} layout="stacked">
                    {input("source_key", (KEY[kind()] ?? KEY.other).placeholder)}
                </Field>
                <Show when={kind() === AWS}>
                    <Field label={t("vault.machineCopy.region")} layout="stacked">
                        {input("source_region", "e.g. us-east-1; empty for this computer's default")}
                    </Field>
                    <Field label={t("vault.machineCopy.profile")} layout="stacked" description={t("vault.machineCopy.profileDescription")}>
                        {input("source_profile", "optional")}
                    </Field>
                </Show>
            </Show>
        </>
    );
}
