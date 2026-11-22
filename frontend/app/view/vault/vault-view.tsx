// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault pane (vault.tsx): the person's secrets, which agents use by name without
// seeing them (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5).
// Agents' requests for new entries on top, then the list (vault-list.tsx).

import { createEffect, createSignal, For, on, onCleanup, Show, type Accessor, type JSX } from "solid-js";

import { useOnResize } from "@/app/hook/useDimensions";
import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import { VaultList } from "./vault-list";
import { VaultNarrowActions } from "./vault-narrow-actions";
import { VaultPicker } from "./vault-picker";
import { ALL_VAULTS } from "./vault-vaults";
import { VaultRequestsList } from "./vault-requests-list";
import { VaultShown } from "./vault-reveal";
import { VaultExportDialog } from "./vault-export";
import { VaultImportDialog } from "./vault-import";
import { VaultEntryForm, type VaultFormTarget } from "./vault-entry-form";
import { vaultKind, type VaultPaneSection } from "./vault-kinds";
import { vaultPromptNotes } from "./vault-prompts";
import { getPlatform } from "@/util/platformutil";
import type { VaultViewModel } from "./vault-model";
import {
    followVault,
    holdVaultShown,
    setVaultOnScreen,
    vaultAvailable,
    vaultError,
    vaultProblem,
    vaultEntries,
    vaultList,
    vaultRequests,
} from "./vault-store";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./vault-view.scss";

/** What may ask the person while they use the Vault, said before it does. */
function VaultPromptsNote(props: { onClose: () => void }): JSX.Element {
    return (
        <div class="vault-prompts" role="note" aria-label="What the Vault may ask you">
            <div class="vault-prompts-text">
                <strong>What the Vault may ask you</strong>
                <ul>
                    <For each={vaultPromptNotes(getPlatform())}>{(line) => <li>{line}</li>}</For>
                </ul>
            </div>
            <Button tone="accent" density="compact" onClick={props.onClose}>
                Got it
            </Button>
        </div>
    );
}

/** The request the Vault was opened on (from a parked agent row or the status bar), to mark in the
 *  list; the meta is cleared so a later reopen does not scroll back to it. */
function focusedRequest(model: VaultViewModel): Accessor<string> {
    const [focused, setFocused] = createSignal("");
    createEffect(
        on(model.focusAtom, (id) => {
            if (!id) return;
            setFocused(id);
            model.setMeta({ "vault:focus": null });
        })
    );
    return focused;
}

/** While shown, renews srv's note that a Vault pane is on screen every 10 s (it lasts 30 s) and clears
 *  it when hidden or closed. Notes go one at a time, so a quick hide and show can't land as show-then-clear. */
function keepOnScreenNoted(model: VaultViewModel): void {
    let sending: Promise<unknown> = Promise.resolve();
    const note = (on: boolean) => {
        sending = sending
            .then(() => setVaultOnScreen(model.blockId, on))
            .catch((e) => console.log("vault: on-screen note failed", e));
    };
    createEffect(() => {
        if (!vaultAvailable() || !model.shown()) return;
        note(true);
        const timer = setInterval(() => note(true), 10_000);
        onCleanup(() => {
            clearInterval(timer);
            note(false);
        });
    });
}

/** Opened on an entry by its link id (a deep link): its section, then its row scrolled into view. The meta is cleared
 *  once the entries are known; an id no entry has leaves the Vault as it is (the opener says the entry is gone). */
function revealLinkedEntry(model: VaultViewModel, setSection: (id: VaultPaneSection) => void, root: { current: HTMLElement | null }): void {
    createEffect(
        on([model.entryLinkAtom, vaultEntries], ([link, entries]) => {
            if (!link || !entries) return;
            model.setMeta({ "vault:entry-link": null });
            const entry = entries.find((e) => e.link_id === link);
            if (!entry) return;
            setSection(entry.section as VaultPaneSection);
            requestAnimationFrame(() => root.current?.querySelector(`[data-entry="${CSS.escape(entry.id)}"]`)?.scrollIntoView?.({ block: "nearest" }));
        })
    );
}

export function VaultView(props: { model: VaultViewModel }): JSX.Element {
    const model = props.model;
    // Names the section the list opens and scrolls to (vault-list.tsx).
    const setSection = (id: VaultPaneSection) => model.setMeta({ "vault:section": id });
    const [form, setForm] = createSignal<VaultFormTarget | null>(null);
    // The Vault the lists are narrowed to; all of them when none is chosen or the chosen one is gone.
    const vault = () => (vaultList().some((v) => v.id === model.vaultAtom()) ? model.vaultAtom() : ALL_VAULTS);

    const fill = (r: VaultRequest) => {
        const kind = vaultKind(r.kind);
        if (kind) setSection(kind.section);
        setForm({ mode: "new", section: kind?.section ?? "personal", request: r });
    };

    // Only while shown: the list decrypts every entry, which can make the OS ask for the Vault key,
    // so a Vault that is hidden (another tab) or closed never reads it.
    createEffect(() => {
        if (!vaultAvailable() || !model.shown()) return;
        followVault();
        onCleanup(holdVaultShown());
    });

    keepOnScreenNoted(model);

    // The pane's own width decides whether the header keeps its buttons, never the window's.
    const container: { current: HTMLDivElement | null } = { current: null };
    useOnResize(container, (rect) => model.setWidth(rect.width));
    revealLinkedEntry(model, setSection, container);

    const focused = focusedRequest(model);

    // Opened from an agent's request prompt: open its form once the request is known.
    createEffect(
        on([model.requestAtom, vaultRequests], ([id, list]) => {
            if (!id) return;
            const r = list.find((x) => x.id === id);
            if (!r) return;
            model.setMeta({ "vault:request": null });
            fill(r);
        })
    );

    return (
        // data-agent-hidden: agents' UI captures and queries leave the Vault out. VaultShown
        // lets an open Reveal close when the Vault stops being shown.
        <VaultShown.Provider value={model.shown}>
        <div class="vault-container" data-agent-hidden="vault" ref={(el) => (container.current = el)}>
            <div class="vault-view" style={{ zoom: model.zoomAtom() }}>
                {/* Shown, with everything else hidden, below the smallest width the Vault supports (vault-view.scss). */}
                <div class="vault-too-narrow" role="status">
                    {t("vault.tooNarrow")}
                </div>
                <Show
                    when={vaultAvailable()}
                    fallback={<div class="warden-section-stub vault-unavailable">The Vault needs the AgentMux desktop app.</div>}
                >
                    <Show when={model.promptsOpen()}>
                        <VaultPromptsNote onClose={() => model.setMeta({ "vault:prompts-read": true })} />
                    </Show>
                    <VaultRequestsList onFill={fill} focused={focused} />
                    <VaultPicker
                        value={vault()}
                        onChange={(id) => model.setMeta({ "vault:vault": id || null })}
                        extra={<Show when={model.narrow()}><VaultNarrowActions model={model} /></Show>}
                    />
                    <Show when={vaultError() ?? vaultProblem()}>
                        <div class="warden-manager-error vault-problem" role="alert">
                            ⚠ {vaultError() ?? vaultProblem()}
                        </div>
                    </Show>
                    <VaultList model={model} vault={vault()} />
                    <Show when={form()}>{(target) => <VaultEntryForm target={target()} onClose={() => setForm(null)} />}</Show>
                    <Show when={model.transfer() === "export"}>
                        <VaultExportDialog onClose={() => model.openTransfer(null)} />
                    </Show>
                    <Show when={model.transfer() === "import"}>
                        <VaultImportDialog into={vault() || undefined} onClose={() => model.openTransfer(null)} />
                    </Show>
                </Show>
            </div>
        </div>
        </VaultShown.Provider>
    );
}

VaultView.displayName = "VaultView";
