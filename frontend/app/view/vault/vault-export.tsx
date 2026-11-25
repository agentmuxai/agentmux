// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Export to a password-protected `.vault` file
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.2).
// The person picks which entries and a passphrase, typed twice; the host asks
// where to save it, and srv writes the encrypted file. The file is an age file:
// `age -d backup.vault` opens it without AgentMux.

import { createMemo, createSignal, For, Show, type Accessor, type JSX, type Setter } from "solid-js";
import { createStore, type SetStoreFunction } from "solid-js/store";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, Field, TextInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { SECTION_LABELS } from "./vault-kinds";
import { isLinked } from "./vault-linked";
import {
    cancelPlainPlace,
    choosePlainPlace,
    exportPlain,
    exportVault,
    suggestPassphrase,
    vaultEntries,
    type PlainFormat,
    type PlainPlace,
} from "./vault-store";

export const MIN_PASSPHRASE = 8;

/** The word typed to confirm an unencrypted export. */
export const PLAIN_CONFIRM_WORD = "export";

export type Strength = { level: 0 | 1 | 2 | 3; label: string };

/** A rough strength for the meter: length matters most; mixed kinds of characters, or
 *  several words, add a little. It only helps the person choose: srv enforces the minimum. */
export function passphraseStrength(p: string): Strength {
    if (p.length < MIN_PASSPHRASE) return { level: 0, label: p ? t("vault.export.strength.short") : "" };
    const kinds = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((r) => r.test(p)).length;
    const words = p.split(/[\s-]+/).filter(Boolean).length;
    const score = p.length + (kinds - 1) * 3 + (words >= 4 ? 8 : 0);
    if (score < 16) return { level: 1, label: t("vault.export.strength.weak") };
    if (score < 24) return { level: 2, label: t("vault.export.strength.fair") };
    return { level: 3, label: t("vault.export.strength.strong") };
}

const ORDER: VaultSection[] = ["personal", "cards", "api-keys", "logins", "notes"];

// `links` is a `.vault` file of the linked entries only: no value is in it.
type Format = "vault" | "links" | "kdbx" | PlainFormat;

type Group = { section: VaultSection; entries: VaultEntryInfo[] };

/** What was saved. `whole`: the links alone were asked for and a whole export was written, values and all. */
type Saved = { count: number; file_name: string; whole?: boolean };

/** The dialog's signals and the entries it can export: which are chosen, the passphrase typed twice, the format, and for an unencrypted file the place and the word. */
interface ExportState {
    onClose: () => void;
    entries: () => VaultEntryInfo[];
    /** The entries the chosen format can hold: the linked ones for a file of links only, else all. */
    held: () => VaultEntryInfo[];
    chosen: Record<string, boolean>;
    setChosen: SetStoreFunction<Record<string, boolean>>;
    pass: Accessor<string>;
    setPass: Setter<string>;
    again: Accessor<string>;
    setAgain: Setter<string>;
    show: Accessor<boolean>;
    setShow: Setter<boolean>;
    busy: Accessor<boolean>;
    setBusy: Setter<boolean>;
    error: Accessor<string | null>;
    setError: Setter<string | null>;
    done: Accessor<Saved | null>;
    setDone: Setter<Saved | null>;
    // An AgentMux `.vault` file, or a KeePass database to take to another manager.
    format: Accessor<Format>;
    setFormat: Setter<Format>;
    place: Accessor<PlainPlace | null>;
    setPlace: Setter<PlainPlace | null>;
    typed: Accessor<string>;
    setTyped: Setter<string>;
    groups: Accessor<Group[]>;
    strength: Accessor<Strength>;
}

function createExportState(props: { onClose: () => void }): ExportState {
    const entries = () => vaultEntries() ?? [];
    const [chosen, setChosen] = createStore<Record<string, boolean>>(
        Object.fromEntries(entries().map((e) => [e.id, true]))
    );
    const [pass, setPass] = createSignal("");
    const [again, setAgain] = createSignal("");
    const [show, setShow] = createSignal(false);
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const [done, setDone] = createSignal<Saved | null>(null);
    const [format, setFormat] = createSignal<Format>("vault");
    const [place, setPlace] = createSignal<PlainPlace | null>(null);
    const [typed, setTyped] = createSignal("");
    const held = () => (format() === "links" ? entries().filter((e) => isLinked(e.fields)) : entries());
    const groups = createMemo(() =>
        ORDER.map((section) => ({ section, entries: held().filter((e) => e.section === section) })).filter(
            (g) => g.entries.length > 0
        )
    );
    const strength = createMemo(() => passphraseStrength(pass()));
    return {
        onClose: props.onClose,
        entries,
        held,
        chosen,
        setChosen,
        pass,
        setPass,
        again,
        setAgain,
        show,
        setShow,
        busy,
        setBusy,
        error,
        setError,
        done,
        setDone,
        format,
        setFormat,
        place,
        setPlace,
        typed,
        setTyped,
        groups,
        strength,
    };
}

/** An unencrypted CSV or JSON: no passphrase, but the place is chosen first and the person types a word. */
const plainFormat = (s: ExportState): PlainFormat | null =>
    s.format() === "csv" || s.format() === "json" ? (s.format() as PlainFormat) : null;
const selected = (s: ExportState) => s.held().filter((e) => s.chosen[e.id]);
const matches = (s: ExportState) => s.pass() === s.again();
const ready = (s: ExportState) =>
    plainFormat(s) ? selected(s).length > 0 : selected(s).length > 0 && s.pass().length >= MIN_PASSPHRASE && matches(s);
const confirmed = (s: ExportState) => s.typed().trim().toLowerCase() === PLAIN_CONFIRM_WORD;
const today = () => new Date().toISOString().slice(0, 10);
const chosenIds = (s: ExportState) =>
    selected(s).length === s.held().length ? undefined : selected(s).map((e) => e.id);
const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

function pickFormat(s: ExportState, f: Format) {
    // Changing format forgets a place chosen for another.
    const was = plainFormat(s);
    if (was && s.place()) void cancelPlainPlace(was);
    s.setPlace(null);
    s.setTyped("");
    s.setFormat(f);
}

async function choosePlace(s: ExportState) {
    const f = plainFormat(s);
    if (!f) return;
    s.setBusy(true);
    s.setError(null);
    try {
        s.setPlace(await choosePlainPlace({ format: f, defaultName: `agentmux-${today()}` }));
    } catch (e) {
        s.setError(errorText(e));
    } finally {
        s.setBusy(false);
    }
}

async function exportUnencrypted(s: ExportState) {
    const f = plainFormat(s);
    const at = s.place();
    if (!f || !at || !confirmed(s)) return;
    s.setBusy(true);
    s.setError(null);
    try {
        s.setDone(await exportPlain({ format: f, pickId: at.pickId, ids: chosenIds(s) }));
        s.setPlace(null);
        s.setTyped("");
    } catch (e) {
        s.setError(errorText(e));
    } finally {
        s.setBusy(false);
    }
}

function close(s: ExportState) {
    const f = plainFormat(s);
    if (f && s.place()) void cancelPlainPlace(f);
    s.onClose();
}

const setSection = (s: ExportState, section: VaultSection, on: boolean) =>
    s
        .entries()
        .filter((e) => e.section === section)
        .forEach((e) => s.setChosen(e.id, on));
const allOn = (s: ExportState, list: VaultEntryInfo[]) => list.every((e) => s.chosen[e.id]);

async function suggest(s: ExportState) {
    s.setError(null);
    try {
        const p = await suggestPassphrase();
        s.setPass(p);
        s.setAgain(p);
        s.setShow(true);
    } catch (e) {
        s.setError(errorText(e));
    }
}

async function save(s: ExportState) {
    s.setBusy(true);
    s.setError(null);
    const links = s.format() === "links";
    try {
        const res = await exportVault({
            ids: chosenIds(s),
            passphrase: s.pass(),
            defaultName: links ? `agentmux-links-${today()}` : `agentmux-${today()}`,
            format: links ? "vault" : (s.format() as "vault" | "kdbx"),
            linksOnly: links,
        });
        if (res) {
            // "No value is in the file" is said on srv's word that it wrote the links alone, not on what was asked
            // for. Without it the file is a whole export, and the dialog says that.
            const whole = links && !res.links_only;
            if (whole) s.setFormat("vault");
            s.setDone({ count: res.count, file_name: res.file_name, whole });
            s.setPass("");
            s.setAgain("");
        }
    } catch (e) {
        s.setError(errorText(e));
    } finally {
        s.setBusy(false);
    }
}

export function VaultExportDialog(props: { onClose: () => void }): JSX.Element {
    const s = createExportState(props);
    return (
        <Modal
            open={true}
            scope="pane"
            onClose={() => close(s)}
            size="md"
            closeOnBackdropClick={false}
            panelClass="vault-form"
        >
            <ModalHeader
                title={t("vault.export.title")}
                description={
                    plainFormat(s)
                        ? t("vault.export.aboutPlain")
                        : s.format() === "links"
                          ? t("vault.export.linksAbout")
                          : t("vault.export.aboutVault")
                }
            />
            <ModalBody>
                <Show when={!s.done()} fallback={<ExportDone s={s} />}>
                    {/* The list is laid out by the dialog's own width (`vault-dialog-scope`), which chrome zoom narrows. */}
                    <div class="vault-dialog-scope">
                        <FormatPicker s={s} />
                    </div>
                    <EntryChoices s={s} />
                    <Show when={!plainFormat(s)}>
                        <PassphraseFields s={s} />
                    </Show>
                    <Show when={plainFormat(s)}>
                        <PlaceChoice s={s} />
                    </Show>
                </Show>
                <Show when={s.error()}>
                    <div class="warden-manager-error" role="alert">
                        {s.error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={() => close(s)} disabled={s.busy()}>
                    {s.done() ? t("vault.export.close") : t("vault.export.cancel")}
                </Button>
                <Show when={!s.done()}>
                    <ExportButton s={s} />
                </Show>
            </ModalFooter>
        </Modal>
    );
}

/** What was saved, and what to do with the file now. */
function ExportDone(props: { s: ExportState }): JSX.Element {
    const s = props.s;
    return (
        <div class="vault-export-done" role="status">
            <p>
                <Rich k="vault.export.saved" params={{ count: s.done()!.count, file: s.done()!.file_name }} />
            </p>
            <Show when={s.done()!.whole}>
                <p class="warden-manager-error" role="alert">
                    {t("vault.export.linksNotWritten")}
                </p>
            </Show>
            <Show when={plainFormat(s)}>
                <p class="warden-manager-error" role="alert">
                    {t("vault.export.plainDone")}
                    {plainFormat(s) === "csv" ? ` ${t("vault.export.csvWarn")}` : ""}
                </p>
            </Show>
            {/* A plain file has no passphrase and isn't an age file: only the warning above applies. */}
            <Show when={!plainFormat(s)}>
                <p class="warden-manager-dim">
                    <Show
                        when={s.format() === "kdbx"}
                        fallback={
                            <>{t("vault.export.ageDone", { file: s.done()!.file_name })}</>
                        }
                    >
                        {t("vault.export.kdbxDone")}
                    </Show>
                </p>
            </Show>
        </div>
    );
}

function FormatOption(props: { s: ExportState; value: Format; name: string; hint: string }): JSX.Element {
    return (
        <label class="vault-check">
            <input
                type="radio"
                name="vault-export-format"
                checked={props.s.format() === props.value}
                onChange={() => pickFormat(props.s, props.value)}
            />
            <strong>{props.name}</strong> <span class="warden-manager-dim">({props.hint})</span>
        </label>
    );
}

function FormatPicker(props: { s: ExportState }): JSX.Element {
    return (
        <div class="vault-export-format" role="radiogroup" aria-label={t("vault.export.formatAria")}>
            <FormatOption
                s={props.s}
                value="vault"
                name={t("vault.export.fmt.vault")}
                hint={t("vault.export.fmt.vaultHint")}
            />
            <Show when={props.s.entries().some((e) => isLinked(e.fields))}>
                <FormatOption s={props.s} value="links" name={t("vault.export.linksName")} hint={t("vault.export.linksHint")} />
            </Show>
            <FormatOption
                s={props.s}
                value="kdbx"
                name={t("vault.export.fmt.kdbx")}
                hint={t("vault.export.fmt.kdbxHint")}
            />
            <FormatOption
                s={props.s}
                value="csv"
                name={t("vault.export.fmt.csv")}
                hint={t("vault.export.fmt.csvHint")}
            />
            <FormatOption
                s={props.s}
                value="json"
                name={t("vault.export.fmt.json")}
                hint={t("vault.export.fmt.jsonHint")}
            />
        </div>
    );
}

/** The entries to export, by section, each with a box. */
function EntryChoices(props: { s: ExportState }): JSX.Element {
    const s = props.s;
    return (
        <div class="vault-export-entries" role="group" aria-label={t("vault.export.whatAria")}>
            <For each={s.groups()}>
                {(g) => (
                    <div class="vault-export-group">
                        <label class="vault-check">
                            <input
                                type="checkbox"
                                checked={allOn(s, g.entries)}
                                onChange={(e) => setSection(s, g.section, e.currentTarget.checked)}
                            />
                            <strong>{SECTION_LABELS[g.section]}</strong>
                            <span class="warden-manager-dim"> ({g.entries.length})</span>
                        </label>
                        <For each={g.entries}>
                            {(e) => (
                                <label class="vault-check vault-check-entry">
                                    <input
                                        type="checkbox"
                                        checked={!!s.chosen[e.id]}
                                        onChange={(ev) => s.setChosen(e.id, ev.currentTarget.checked)}
                                    />
                                    {e.label}
                                </label>
                            )}
                        </For>
                    </div>
                )}
            </For>
            <Show when={s.entries().length === 0}>
                <p class="warden-manager-dim">{t("vault.export.empty")}</p>
            </Show>
        </div>
    );
}

/** The passphrase typed twice, with its meter. */
function PassphraseFields(props: { s: ExportState }): JSX.Element {
    const s = props.s;
    return (
        <>
            <Field
                label={t("vault.export.passphrase")}
                layout="stacked"
                hint={s.strength().label ? t("vault.export.strengthIs", { label: s.strength().label }) : t("vault.export.atLeast", { min: MIN_PASSPHRASE })}
            >
                <div class="vault-part">
                    <TextInput
                        type={s.show() ? "text" : "password"}
                        autocomplete="new-password"
                        spellcheck={false}
                        value={s.pass()}
                        onInput={(e) => s.setPass(e.currentTarget.value)}
                    />
                    <Button density="compact" tone="quiet" icon="wand-magic-sparkles" onClick={() => suggest(s)}>
                        {t("vault.export.suggest")}
                    </Button>
                </div>
                <meter
                    class="vault-strength"
                    min={0}
                    max={3}
                    value={s.strength().level}
                    aria-label={t("vault.export.strengthAria")}
                />
            </Field>
            <Field
                label={t("vault.export.again")}
                layout="stacked"
                error={s.again() && !matches(s) ? t("vault.export.noMatch") : undefined}
            >
                <TextInput
                    type={s.show() ? "text" : "password"}
                    autocomplete="new-password"
                    spellcheck={false}
                    value={s.again()}
                    onInput={(e) => s.setAgain(e.currentTarget.value)}
                />
            </Field>
            <label class="vault-check">
                <input type="checkbox" checked={s.show()} onChange={(e) => s.setShow(e.currentTarget.checked)} />{" "}
                {t("vault.export.show")}
            </label>
        </>
    );
}

/** Where an unencrypted file will be saved, and the word that confirms it. */
function PlaceChoice(props: { s: ExportState }): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.place()}>
            {(at) => (
                <div class="vault-export-place" role="group" aria-label={t("vault.export.placeAria")}>
                    <p>
                        <Rich k="vault.export.willSave" params={{ file: at().fileName, folder: at().folder }} />{" "}
                        <Button density="compact" tone="quiet" disabled={s.busy()} onClick={() => void choosePlace(s)}>
                            {t("vault.export.otherPlace")}
                        </Button>
                    </p>
                    <Show when={at().syncedFolder}>
                        {(service) => (
                            <p class="warden-manager-error" role="alert">
                                {t("vault.export.synced", { service: service() })}
                            </p>
                        )}
                    </Show>
                    <Field
                        label={t("vault.export.typeWord", { word: PLAIN_CONFIRM_WORD })}
                        layout="stacked"
                        description={t("vault.export.osConfirm")}
                    >
                        <TextInput
                            autocomplete="off"
                            spellcheck={false}
                            value={s.typed()}
                            onInput={(e) => s.setTyped(e.currentTarget.value)}
                        />
                    </Field>
                </div>
            )}
        </Show>
    );
}

/** The button that goes on: choose where to save, or for an unencrypted file with its place chosen, write it. */
function ExportButton(props: { s: ExportState }): JSX.Element {
    const s = props.s;
    return (
        <Show
            when={plainFormat(s) && s.place()}
            fallback={
                <Button
                    tone="accent"
                    busy={s.busy()}
                    disabled={!ready(s)}
                    onClick={() => void (plainFormat(s) ? choosePlace(s) : save(s))}
                >
                    {t("vault.export.chooseWhere")}
                </Button>
            }
        >
            <Button
                tone="danger"
                busy={s.busy()}
                disabled={!ready(s) || !confirmed(s)}
                onClick={() => exportUnencrypted(s)}
            >
                {t("vault.export.unencrypted")}
            </Button>
        </Show>
    );
}
