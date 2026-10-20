// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Reveal and Copy (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.3).
// Each asks the OS to confirm the person first, through the host. Copy never
// brings the value into the page: the host puts it on the clipboard and clears
// it 30 seconds later. Reveal shows the value for a few seconds, and hides it
// when the dialog closes or the window loses focus.

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { createStore } from "solid-js/store";

import { Modal } from "@/app/element/modal";
import { ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal-parts";
import { Button } from "@/app/element/ui";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import { getPlatform } from "@/util/platformutil";
import { vaultKind } from "./vault-kinds";
import { copyVaultPart, revealVaultPart } from "./vault-store";

/** How long a revealed value stays on screen. */
export const REVEAL_SHOWN_MS = 15_000;

/** Whether Reveal and Copy can run on this OS: only Windows Hello is built. */
export function canConfirmPerson(platform: string = getPlatform()): boolean {
    return platform === "win32";
}

export const CONFIRM_UNAVAILABLE = "Reveal and Copy need Windows Hello for now: Touch ID and the desktop's prompt aren't built yet.";

export function VaultRevealDialog(props: { entry: VaultEntryInfo; onClose: () => void }): JSX.Element {
    const entry = props.entry;
    const kind = vaultKind(entry.kind);
    const parts = (kind?.parts ?? []).filter((p) => entry.parts_set.includes(p.name));
    const [shown, setShown] = createStore<Record<string, string>>({});
    const [busy, setBusy] = createSignal<string | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [notice, setNotice] = createSignal<string | null>(null);
    const timers = new Map<string, ReturnType<typeof setTimeout>>();
    const available = canConfirmPerson();

    const hide = (part: string) => {
        clearTimeout(timers.get(part));
        timers.delete(part);
        setShown(part, "");
    };
    // Bumped whenever everything is hidden (focus lost, dialog closed): a
    // Reveal that was still waiting for Windows Hello when that happened
    // finds it changed and shows nothing.
    let concealed = 0;
    const hideAll = () => {
        concealed++;
        parts.forEach((p) => hide(p.name));
    };
    onMount(() => window.addEventListener("blur", hideAll));
    onCleanup(() => {
        window.removeEventListener("blur", hideAll);
        hideAll();
    });

    // "cancelled" is the person dismissing the OS prompt: not an error.
    const run = async (key: string, what: () => Promise<void>) => {
        setBusy(key);
        setError(null);
        setNotice(null);
        try {
            await what();
        } catch (e) {
            const m = e instanceof Error ? e.message : String(e);
            if (!/\bcancelled\b/i.test(m)) setError(m);
        } finally {
            setBusy(null);
        }
    };

    const reveal = (part: string) =>
        run(part, async () => {
            const before = concealed;
            const value = await revealVaultPart(entry.id, part, entry.label);
            if (concealed !== before) return;
            setShown(part, value);
            timers.set(part, setTimeout(() => hide(part), REVEAL_SHOWN_MS));
        });

    const copy = (part: string) =>
        run(`copy:${part}`, async () => {
            const secs = await copyVaultPart(entry.id, part, entry.label);
            setNotice(`Copied. It clears from the clipboard in ${secs} seconds.`);
        });

    return (
        <Modal open={true} scope="pane" onClose={props.onClose} size="md" closeOnBackdropClick={false} panelClass="vault-form">
            <ModalHeader
                title={`Show or copy ${entry.label}`}
                description={
                    available
                        ? "Windows Hello confirms it's you each time. A revealed value hides after 15 seconds."
                        : CONFIRM_UNAVAILABLE
                }
            />
            <ModalBody>
                <div class="vault-reveal-parts">
                    <For each={parts}>
                        {(p) => (
                            <div class="vault-reveal-part" data-part={p.name}>
                                <span class="vault-reveal-label">{p.label}</span>
                                <span class="warden-manager-mono vault-reveal-value" data-revealed={shown[p.name] ? "" : undefined}>
                                    {shown[p.name] || "••••••••"}
                                </span>
                                <Show
                                    when={shown[p.name]}
                                    fallback={
                                        <Button
                                            density="compact"
                                            icon="eye"
                                            disabled={!available}
                                            busy={busy() === p.name}
                                            title={available ? undefined : CONFIRM_UNAVAILABLE}
                                            onClick={() => void reveal(p.name)}
                                        >
                                            Reveal
                                        </Button>
                                    }
                                >
                                    <Button density="compact" icon="eye-slash" onClick={() => hide(p.name)}>
                                        Hide
                                    </Button>
                                </Show>
                                <Button
                                    density="compact"
                                    icon="copy"
                                    disabled={!available}
                                    busy={busy() === `copy:${p.name}`}
                                    title={available ? undefined : CONFIRM_UNAVAILABLE}
                                    onClick={() => void copy(p.name)}
                                >
                                    Copy
                                </Button>
                            </div>
                        )}
                    </For>
                </div>
                <Show when={notice()}>
                    <p class="warden-manager-dim" role="status">
                        {notice()}
                    </p>
                </Show>
                <Show when={error()}>
                    <div class="warden-manager-error" role="alert">
                        {error()}
                    </div>
                </Show>
            </ModalBody>
            <ModalFooter>
                <Button data-modal-dismiss onClick={props.onClose}>
                    Close
                </Button>
            </ModalFooter>
        </Modal>
    );
}
