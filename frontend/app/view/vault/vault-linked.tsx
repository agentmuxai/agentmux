// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A linked entry: one that holds no value and says where a key lives, who reads it, when it last changed and where
// else it is kept (docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §2). It is marked
// by the plain field `link` set to `only`; srv (crates/srv-core/src/backend/vault/linked.rs) is the source for what
// its fields may hold. Nothing here is a value.

import { For, Show, type JSX } from "solid-js";
import type { SetStoreFunction } from "solid-js/store";

import { Field, IconButton, Pill, Switch, TextArea, TextInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import { writeText } from "@/util/clipboard";

/** A linked entry nobody has checked for this many days is marked unchecked (srv's `UNCHECKED_DAYS`). */
export const UNCHECKED_DAYS = 30;
const DAY_MS = 24 * 60 * 60 * 1000;
const AWS = "aws-secret";

/** Whether the entry with these fields is a link: it holds no value. */
export function isLinked(fields: Record<string, string>): boolean {
    return fields.link === "only";
}

/** The lines of a field that holds one thing a line (`readers`, `copies`). */
export function linesOf(text: string | undefined): string[] {
    return (text ?? "")
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean);
}

/** The link's fields as they are saved: the mark, and what is said of the key. A date of last change is a link's own. */
export function linkFields(fields: Record<string, string>): Record<string, string> {
    const linked = isLinked(fields);
    const kept: [string, string][] = [
        ["link", linked ? "only" : ""],
        ["readers", linesOf(fields.readers).join("\n")],
        ["rotated", linked ? (fields.rotated ?? "").trim() : ""],
        ["copies", linesOf(fields.copies).join("\n")],
    ];
    return Object.fromEntries(kept.filter(([, v]) => v !== ""));
}

/** How many days a linked entry has gone unchecked, once that is more than `UNCHECKED_DAYS`; null for one checked
 *  since, one never checked (its line says so), and an entry that is not a link. */
export function uncheckedDays(e: Pick<VaultEntryInfo, "fields" | "source_checked">, now = Date.now()): number | null {
    if (!isLinked(e.fields) || !e.source_checked) return null;
    const days = Math.floor((now - e.source_checked.at) / DAY_MS);
    return days > UNCHECKED_DAYS ? days : null;
}

/** The two badges of a linked entry: that it holds no value, and that the key is read without asking the Vault. */
export function LinkedBadges(props: { entry: VaultEntryInfo }): JSX.Element {
    return (
        <Show when={isLinked(props.entry.fields)}>
            <span class="vault-linked-badges">
                <Pill tone="accent" icon="link" title={t("vault.linked.badgeTip")}>
                    {t("vault.linked.badge")}
                </Pill>
                <Pill tone="warn" icon="eye" title={t("vault.linked.outsideTip")}>
                    {t("vault.linked.outside")}
                </Pill>
            </span>
        </Show>
    );
}

/** What an entry says of its key beside the place: who reads it, when it last changed, where else it is kept. */
export function LinkedDetails(props: { entry: VaultEntryInfo }): JSX.Element {
    const said = () =>
        [
            { name: "readers", label: t("vault.linked.readers"), lines: linesOf(props.entry.fields.readers) },
            { name: "rotated", label: t("vault.linked.rotated"), lines: linesOf(props.entry.fields.rotated) },
            { name: "copies", label: t("vault.linked.copies"), lines: linesOf(props.entry.fields.copies) },
        ].filter((s) => s.lines.length > 0);
    return (
        <For each={said()}>
            {(s) => (
                <div class="vault-linked-said" data-said={s.name}>
                    {s.label} <span class={s.name === "readers" ? undefined : "warden-manager-mono vault-import-id"}>{s.lines.join("; ")}</span>
                </div>
            )}
        </For>
    );
}

/** Copies where the key lives, so the person can find it there: the place's location, never a link into a console. */
export function CopyPlaceButton(props: { entry: VaultEntryInfo; onError: (message: string | null) => void }): JSX.Element {
    const copy = async () => {
        try {
            await writeText(props.entry.fields.source_location ?? "");
            props.onError(null);
        } catch (err) {
            props.onError(err instanceof Error ? err.message : String(err));
        }
    };
    return <IconButton icon="copy" label={t("vault.linked.copyPlace")} density="compact" onClick={() => void copy()} />;
}

/** The form's inputs for what an entry says of its key: on a new entry, whether it is a link at all; then who reads
 *  the key, where else it is kept and, for a link, when it last changed. Shown once a place is chosen. */
export function LinkedFields(props: { fields: Record<string, string>; setFields: SetStoreFunction<Record<string, string>>; isNew: boolean }): JSX.Element {
    const lines = (name: string, rows: number, placeholder: string) => (
        <TextArea value={props.fields[name] ?? ""} rows={rows} spellcheck={false} placeholder={placeholder} onInput={(e) => props.setFields(name, e.currentTarget.value)} />
    );
    const setOnly = (on: boolean) => {
        props.setFields("link", on ? "only" : "");
        // A link is its place: start it at the kind of place a check can describe.
        if (on && !props.fields.source_kind) props.setFields("source_kind", AWS);
    };
    return (
        <>
            <Show when={props.isNew}>
                <Field label={t("vault.linked.only")} layout="stacked" description={t("vault.linked.onlyDescription")}>
                    <Switch checked={isLinked(props.fields)} onChange={setOnly} />
                </Field>
            </Show>
            <Show when={props.fields.source_kind}>
                <Field label={t("vault.linked.readersLabel")} layout="stacked" description={t("vault.linked.readersDescription")}>
                    {lines("readers", 3, "")}
                </Field>
                <Show when={isLinked(props.fields)}>
                    <Field label={t("vault.linked.rotatedLabel")} layout="stacked" description={t("vault.linked.rotatedDescription")}>
                        <TextInput value={props.fields.rotated ?? ""} placeholder={t("vault.linked.rotatedPlaceholder")} spellcheck={false} autocomplete="off" onInput={(e) => props.setFields("rotated", e.currentTarget.value)} />
                    </Field>
                </Show>
                <Field label={t("vault.linked.copiesLabel")} layout="stacked" description={t("vault.linked.copiesDescription")}>
                    {lines("copies", 2, "")}
                </Field>
            </Show>
        </>
    );
}
