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
import { exportVault, suggestPassphrase, vaultEntries } from "./vault-store";

export const MIN_PASSPHRASE = 8;

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

    const groups = createMemo(() =>
        ORDER.map((section) => ({ section, entries: entries().filter((e) => e.section === section) })).filter((g) => g.entries.length > 0)
    );
    const selected = () => entries().filter((e) => chosen[e.id]);
    const strength = createMemo(() => passphraseStrength(pass()));
    const matches = () => pass() === again();
    const ready = () => selected().length > 0 && pass().length >= MIN_PASSPHRASE && matches();

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
            const all = selected().length === entries().length;
            const today = new Date().toISOString().slice(0, 10);
            const res = await exportVault({ ids: all ? undefined : selected().map((e) => e.id), passphrase: pass(), defaultName: `agentmux-${today}` });
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
        <Modal open={true} scope="pane" onClose={props.onClose} size="md" closeOnBackdropClick={false} panelClass="vault-form">
            <ModalHeader
                title="Export the Vault"
                description="Saves a password-protected backup. Anyone with the file and its passphrase can read every secret in it."
            />
            <ModalBody>
                <Show
                    when={!done()}
                    fallback={
                        <div class="vault-export-done" role="status">
                            <p>
                                Saved {done()!.count} {done()!.count === 1 ? "entry" : "entries"} to <strong>{done()!.file_name}</strong>.
                            </p>
                            <p class="warden-manager-dim">
                                {"Keep the passphrase somewhere safe: nothing can recover the file without it. It's an age file, so `age -d "}
                                {done()!.file_name}
                                {"` opens it without AgentMux."}
                            </p>
                        </div>
                    }
                >
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
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.onClose} disabled={busy()}>
                    {done() ? "Close" : "Cancel"}
                </Button>
                <Show when={!done()}>
                    <Button tone="accent" busy={busy()} disabled={!ready()} onClick={() => void save()}>
                        Choose where to save…
                    </Button>
                </Show>
            </ModalFooter>
        </Modal>
    );
}
