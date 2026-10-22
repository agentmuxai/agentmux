// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Export to a password-protected `.vault` file
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.2).
// The person picks which entries and a passphrase, typed twice; the host asks
// where to save it, and srv writes the encrypted file. The file is an age file:
// `age -d backup.vault` opens it without AgentMux.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { createStore } from "solid-js/store";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button, Field, TextInput } from "@/app/element/ui";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { SECTION_LABELS } from "./vault-kinds";
import { cancelPlainPlace, choosePlainPlace, exportPlain, exportVault, suggestPassphrase, vaultEntries, type PlainFormat, type PlainPlace } from "./vault-store";

export const MIN_PASSPHRASE = 8;

/** The word typed to confirm an unencrypted export. */
export const PLAIN_CONFIRM_WORD = "export";

export type Strength = { level: 0 | 1 | 2 | 3; label: string };

/** A rough strength for the meter: length matters most; mixed kinds of characters, or
 *  several words, add a little. It only helps the person choose: srv enforces the minimum. */
export function passphraseStrength(p: string): Strength {
    if (p.length < MIN_PASSPHRASE) return { level: 0, label: p ? "Too short" : "" };
    const kinds = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((r) => r.test(p)).length;
    const words = p.split(/[\s-]+/).filter(Boolean).length;
    const score = p.length + (kinds - 1) * 3 + (words >= 4 ? 8 : 0);
    if (score < 16) return { level: 1, label: "Weak" };
    if (score < 24) return { level: 2, label: "Fair" };
    return { level: 3, label: "Strong" };
}

const ORDER: VaultSection[] = ["personal", "cards", "api-keys", "logins", "notes"];

export function VaultExportDialog(props: { onClose: () => void }): JSX.Element {
    const entries = () => vaultEntries() ?? [];
    const [chosen, setChosen] = createStore<Record<string, boolean>>(Object.fromEntries(entries().map((e) => [e.id, true])));
    const [pass, setPass] = createSignal("");
    const [again, setAgain] = createSignal("");
    const [show, setShow] = createSignal(false);
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const [done, setDone] = createSignal<{ count: number; file_name: string } | null>(null);
    // An AgentMux `.vault` file, or a KeePass database to take to another manager.
    const [format, setFormat] = createSignal<"vault" | "kdbx" | PlainFormat>("vault");
    // An unencrypted CSV or JSON: no passphrase, but the place is chosen first and the person types a word.
    const plainFormat = (): PlainFormat | null => (format() === "csv" || format() === "json" ? (format() as PlainFormat) : null);
    const [place, setPlace] = createSignal<PlainPlace | null>(null);
    const [typed, setTyped] = createSignal("");

    const groups = createMemo(() =>
        ORDER.map((section) => ({ section, entries: entries().filter((e) => e.section === section) })).filter((g) => g.entries.length > 0)
    );
    const selected = () => entries().filter((e) => chosen[e.id]);
    const strength = createMemo(() => passphraseStrength(pass()));
    const matches = () => pass() === again();
    const ready = () => (plainFormat() ? selected().length > 0 : selected().length > 0 && pass().length >= MIN_PASSPHRASE && matches());
    const confirmed = () => typed().trim().toLowerCase() === PLAIN_CONFIRM_WORD;
    const today = () => new Date().toISOString().slice(0, 10);
    const chosenIds = () => (selected().length === entries().length ? undefined : selected().map((e) => e.id));

    const pickFormat = (f: "vault" | "kdbx" | PlainFormat) => {
        // Changing format forgets a place chosen for another.
        const was = plainFormat();
        if (was && place()) void cancelPlainPlace(was);
        setPlace(null);
        setTyped("");
        setFormat(f);
    };

    const choosePlace = async () => {
        const f = plainFormat();
        if (!f) return;
        setBusy(true);
        setError(null);
        try {
            setPlace(await choosePlainPlace({ format: f, defaultName: `agentmux-${today()}` }));
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };

    const exportUnencrypted = async () => {
        const f = plainFormat();
        const at = place();
        if (!f || !at || !confirmed()) return;
        setBusy(true);
        setError(null);
        try {
            setDone(await exportPlain({ format: f, pickId: at.pickId, ids: chosenIds() }));
            setPlace(null);
            setTyped("");
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };

    const close = () => {
        const f = plainFormat();
        if (f && place()) void cancelPlainPlace(f);
        props.onClose();
    };

    const setSection = (section: VaultSection, on: boolean) =>
        entries()
            .filter((e) => e.section === section)
            .forEach((e) => setChosen(e.id, on));
    const allOn = (list: VaultEntryInfo[]) => list.every((e) => chosen[e.id]);

    const suggest = async () => {
        setError(null);
        try {
            const p = await suggestPassphrase();
            setPass(p);
            setAgain(p);
            setShow(true);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };

    const save = async () => {
        setBusy(true);
        setError(null);
        try {
            const res = await exportVault({ ids: chosenIds(), passphrase: pass(), defaultName: `agentmux-${today()}`, format: format() as "vault" | "kdbx" });
            if (res) {
                setDone(res);
                setPass("");
                setAgain("");
            }
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <Modal open={true} scope="pane" onClose={close} size="md" closeOnBackdropClick={false} panelClass="vault-form">
            <ModalHeader
                title="Export the Vault"
                description={
                    plainFormat()
                        ? "Saves a file with every secret in it, unencrypted, for another password manager. Anyone who can open the file can read them."
                        : "Saves a password-protected backup. Anyone with the file and its passphrase can read every secret in it."
                }
            />
            <ModalBody>
                <Show
                    when={!done()}
                    fallback={
                        <div class="vault-export-done" role="status">
                            <p>
                                Saved {done()!.count} {done()!.count === 1 ? "entry" : "entries"} to <strong>{done()!.file_name}</strong>.
                            </p>
                            <Show when={plainFormat()}>
                                <p class="warden-manager-error" role="alert">
                                    This file isn't encrypted. Import it into the other manager, then delete it, and empty the trash or recycle bin.
                                    {plainFormat() === "csv" ? " Don't open it in a spreadsheet and save it: a spreadsheet changes values it takes for numbers or formulas." : ""}
                                </p>
                            </Show>
                            {/* A plain file has no passphrase and isn't an age file: only the warning above applies. */}
                            <Show when={!plainFormat()}>
                                <p class="warden-manager-dim">
                                    <Show
                                        when={format() === "kdbx"}
                                        fallback={
                                            <>
                                                {"Keep the passphrase somewhere safe: nothing can recover the file without it. It's an age file, so `age -d "}
                                                {done()!.file_name}
                                                {"` opens it without AgentMux."}
                                            </>
                                        }
                                    >
                                        {"Open it in KeePassXC, KeePassDX or Strongbox with that password. Keep the password somewhere safe: nothing can recover the file without it. It is a KDBX 4.1 file, so a KeePass app from before 2022 can't open it."}
                                    </Show>
                                </p>
                            </Show>
                        </div>
                    }
                >
                    <div class="vault-export-format" role="radiogroup" aria-label="File format">
                        <label class="vault-check">
                            <input type="radio" name="vault-export-format" checked={format() === "vault"} onChange={() => pickFormat("vault")} />
                            <strong>AgentMux Vault file</strong> <span class="warden-manager-dim">(.vault, recommended: every kind, part and detail)</span>
                        </label>
                        <label class="vault-check">
                            <input type="radio" name="vault-export-format" checked={format() === "kdbx"} onChange={() => pickFormat("kdbx")} />
                            <strong>KeePass database</strong> <span class="warden-manager-dim">(.kdbx, to open in KeePassXC, KeePassDX or Strongbox)</span>
                        </label>
                        <label class="vault-check">
                            <input type="radio" name="vault-export-format" checked={format() === "csv"} onChange={() => pickFormat("csv")} />
                            <strong>CSV, unencrypted</strong> <span class="warden-manager-dim">(.csv, Bitwarden's layout, which most managers import)</span>
                        </label>
                        <label class="vault-check">
                            <input type="radio" name="vault-export-format" checked={format() === "json"} onChange={() => pickFormat("json")} />
                            <strong>JSON, unencrypted</strong> <span class="warden-manager-dim">(.json, every kind and detail; AgentMux imports it back)</span>
                        </label>
                    </div>
                    <div class="vault-export-entries" role="group" aria-label="What to export">
                        <For each={groups()}>
                            {(g) => (
                                <div class="vault-export-group">
                                    <label class="vault-check">
                                        <input type="checkbox" checked={allOn(g.entries)} onChange={(e) => setSection(g.section, e.currentTarget.checked)} />
                                        <strong>{SECTION_LABELS[g.section]}</strong>
                                        <span class="warden-manager-dim"> ({g.entries.length})</span>
                                    </label>
                                    <For each={g.entries}>
                                        {(e) => (
                                            <label class="vault-check vault-check-entry">
                                                <input type="checkbox" checked={!!chosen[e.id]} onChange={(ev) => setChosen(e.id, ev.currentTarget.checked)} />
                                                {e.label}
                                            </label>
                                        )}
                                    </For>
                                </div>
                            )}
                        </For>
                        <Show when={entries().length === 0}>
                            <p class="warden-manager-dim">The Vault is empty: there is nothing to export.</p>
                        </Show>
                    </div>
                    <Show when={!plainFormat()}>
                        <Field label="Passphrase" layout="stacked" hint={strength().label ? `Strength: ${strength().label}` : `At least ${MIN_PASSPHRASE} characters.`}>
                            <div class="vault-part">
                                <TextInput type={show() ? "text" : "password"} autocomplete="new-password" spellcheck={false} value={pass()} onInput={(e) => setPass(e.currentTarget.value)} />
                                <Button density="compact" tone="quiet" icon="wand-magic-sparkles" onClick={() => void suggest()}>
                                    Suggest one
                                </Button>
                            </div>
                            <meter class="vault-strength" min={0} max={3} value={strength().level} aria-label="Passphrase strength" />
                        </Field>
                        <Field label="Type it again" layout="stacked" error={again() && !matches() ? "The two don't match." : undefined}>
                            <TextInput type={show() ? "text" : "password"} autocomplete="new-password" spellcheck={false} value={again()} onInput={(e) => setAgain(e.currentTarget.value)} />
                        </Field>
                        <label class="vault-check">
                            <input type="checkbox" checked={show()} onChange={(e) => setShow(e.currentTarget.checked)} /> Show the passphrase
                        </label>
                    </Show>
                    <Show when={plainFormat()}>
                        <Show when={place()}>
                            {(at) => (
                                <div class="vault-export-place" role="group" aria-label="Where it will be saved">
                                    <p>
                                        It will be saved as <strong>{at().fileName}</strong> in <strong>{at().folder}</strong>.{" "}
                                        <Button density="compact" tone="quiet" disabled={busy()} onClick={() => void choosePlace()}>
                                            Choose another place
                                        </Button>
                                    </p>
                                    <Show when={at().syncedFolder}>
                                        {(service) => (
                                            <p class="warden-manager-error" role="alert">
                                                That folder is synced by {service()}: a copy may reach the cloud, and stay in its file history, even after you delete the file here.
                                            </p>
                                        )}
                                    </Show>
                                    <Field label={`Type "${PLAIN_CONFIRM_WORD}" to confirm`} layout="stacked" description="Windows or macOS then asks you to confirm it is you, as it does for Show and Copy.">
                                        <TextInput autocomplete="off" spellcheck={false} value={typed()} onInput={(e) => setTyped(e.currentTarget.value)} />
                                    </Field>
                                </div>
                            )}
                        </Show>
                    </Show>
                </Show>
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={close} disabled={busy()}>
                    {done() ? "Close" : "Cancel"}
                </Button>
                <Show when={!done()}>
                    <Show
                        when={plainFormat() && place()}
                        fallback={
                            <Button tone="accent" busy={busy()} disabled={!ready()} onClick={() => void (plainFormat() ? choosePlace() : save())}>
                                Choose where to save…
                            </Button>
                        }
                    >
                        <Button tone="danger" busy={busy()} disabled={!ready() || !confirmed()} onClick={() => void exportUnencrypted()}>
                            Export unencrypted
                        </Button>
                    </Show>
                </Show>
            </ModalFooter>
        </Modal>
    );
}
