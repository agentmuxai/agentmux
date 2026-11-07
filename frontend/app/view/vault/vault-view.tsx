// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault pane (vault.tsx): the person's secrets, which agents use by name without
// seeing them (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5).
// A rail of sections; agents' requests for new entries on top.

import { createEffect, createSignal, For, on, onCleanup, Show, type Accessor, type JSX } from "solid-js";

import { Button, TabbedPane, type TabItem } from "@/app/element/ui";
import type { VaultActivity } from "@/types/rpc/VaultActivity";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { VaultEntries } from "./vault-entries";
import { VaultHistoryDialog, whoOf, whyOf } from "./vault-history";
import { VaultManageDialog } from "./vault-manage";
import { ALL_VAULTS, VaultPicker } from "./vault-vaults";
import { VaultRequestsList } from "./vault-requests-list";
import { VaultShown } from "./vault-reveal";
import { VaultExportDialog } from "./vault-export";
import { VaultImportDialog } from "./vault-import";
import { VaultEntryForm, type VaultFormTarget } from "./vault-entry-form";
import { formatWhen } from "./vault-format";
import { VAULT_SECTIONS, vaultKind, type VaultPaneSection } from "./vault-kinds";
import { vaultPromptNotes } from "./vault-prompts";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { getPlatform } from "@/util/platformutil";
import type { VaultViewModel } from "./vault-model";
import {
    followVault,
    holdVaultShown,
    loadActivity,
    loadKept,
    setVaultOnScreen,
    vaultAvailable,
    vaultChangedMs,
    vaultEntries,
    vaultError,
    vaultProblem,
    vaultList,
    vaultRequests,
    type KeptSecret,
} from "./vault-store";

import "@/app/view/section-pane/section-pane.scss";
import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./vault-view.scss";

const ENTRY_SECTIONS: VaultSection[] = ["personal", "cards", "api-keys", "logins", "notes"];

const RAIL: TabItem<VaultPaneSection>[] = [...VAULT_SECTIONS.items];

const MANAGED_IN: Record<string, string> = {
    accounts: "the Accounts pane",
    muxbus: "Settings, under cloud messages",
    tower: "Tower",
    browser: "The browser pane that asked for it",
};

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

function VaultKept(props: { visible: boolean }): JSX.Element {
    const [kept, setKept] = createSignal<KeptSecret[] | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    // Read again each time the tab is shown: the keychain changes where these secrets are managed, with no event.
    createEffect(
        on(
            () => props.visible,
            (visible) => {
                if (!visible) return;
                loadKept()
                    .then((list) => {
                        setKept(list);
                        setError(null);
                    })
                    .catch((e) => setError(String(e)));
            }
        )
    );
    return (
        <div class="warden-manager-body">
            <p class="warden-manager-summary">
                {"Secrets AgentMux keeps for itself, in the same keychain. They're managed where they're used; the Vault only lists them."}
            </p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show when={(kept() ?? []).length > 0} fallback={<div class="warden-section-stub">{kept() === null ? "Loading…" : "None yet."}</div>}>
                <table class="warden-manager-table">
                    <thead>
                        <tr>
                            <th>Name</th>
                            <th>What</th>
                            <th>Managed in</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={kept()}>
                            {(k) => (
                                <tr>
                                    <td>{k.label}</td>
                                    <td>{k.what}</td>
                                    <td class="warden-manager-dim">{MANAGED_IN[k.managed_in] ?? k.managed_in}</td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
            <p class="warden-manager-footnote">
                {"Only sign-ins a browser pane saves from now on are listed: the keychain can't be listed, so earlier ones aren't. The sessions a browser profile keeps for websites you signed in to (cookies) are not here: they belong to the profile."}
            </p>
        </div>
    );
}

/** One line of the Activity tab; an entry that still exists opens its History. */
function ActivityRow(props: { row: VaultActivity; onHistory: (a: VaultActivity) => void }): JSX.Element {
    const a = () => props.row;
    const exists = () => (vaultEntries() ?? []).some((e) => e.id === a().entry_id);
    return (
        <tr data-outcome={a().outcome}>
            <td class="warden-manager-dim">{formatWhen(a().at)}</td>
            <td>{whoOf(a())}</td>
            <td>{a().action}</td>
            <td>
                <Show when={exists()} fallback={a().entry_label}>
                    <Button density="compact" onClick={() => props.onHistory(a())}>
                        {a().entry_label}
                    </Button>
                </Show>
            </td>
            <td class="warden-manager-dim">{a().target}</td>
            <td class="warden-manager-dim">{whyOf(a())}</td>
        </tr>
    );
}

function VaultActivityLog(props: { visible: boolean }): JSX.Element {
    const [rows, setRows] = createSignal<VaultActivity[] | null>(null);
    const [historyOf, setHistoryOf] = createSignal<VaultActivity | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const load = () =>
        loadActivity()
            .then((r) => {
                setRows(r);
                setError(null);
            })
            .catch((e) => setError(String(e)));
    // Loads when first shown, and again whenever the Vault changes while shown.
    createEffect(on([() => props.visible, vaultChangedMs], ([visible]) => {
        if (visible) void load();
    }));
    return (
        <div class="warden-manager-body">
            <p class="warden-manager-summary">
                Every change to the Vault and every use of an entry: who, what, where. Never a value.{" "}
                <Button density="compact" onClick={() => void openOrFocusPaneByView("audit")}>
                    {"See agents' use in the Audit"}
                </Button>
            </p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show when={(rows() ?? []).length > 0} fallback={<div class="warden-section-stub">{rows() === null ? "Loading…" : "Nothing yet."}</div>}>
                <table class="warden-manager-table">
                    <thead>
                        <tr>
                            <th>When</th>
                            <th>Who</th>
                            <th>What</th>
                            <th>Entry</th>
                            <th>Where</th>
                            <th>Why</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={rows()}>{(a) => <ActivityRow row={a} onHistory={setHistoryOf} />}</For>
                    </tbody>
                </table>
            </Show>
            <Show when={historyOf()}>
                {(a) => <VaultHistoryDialog entryId={a().entry_id} label={a().entry_label} onClose={() => setHistoryOf(null)} />}
            </Show>
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

export function VaultView(props: { model: VaultViewModel }): JSX.Element {
    const model = props.model;
    const section = model.sectionAtom;
    const setSection = (id: VaultPaneSection) => model.setMeta({ "vault:section": id });
    const [form, setForm] = createSignal<VaultFormTarget | null>(null);
    const [managing, setManaging] = createSignal(false);
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
        <div class="vault-container" data-agent-hidden="vault">
            <div class="vault-view" style={{ zoom: model.zoomAtom() }}>
                <Show
                    when={vaultAvailable()}
                    fallback={<div class="warden-section-stub vault-unavailable">The Vault needs the AgentMux desktop app.</div>}
                >
                    <Show when={model.promptsOpen()}>
                        <VaultPromptsNote onClose={() => model.setMeta({ "vault:prompts-read": true })} />
                    </Show>
                    <VaultRequestsList onFill={fill} focused={focused} />
                    <VaultPicker value={vault()} onChange={(id) => model.setMeta({ "vault:vault": id || null })} onManage={() => setManaging(true)} />
                    <Show when={vaultError() ?? vaultProblem()}>
                        <div class="warden-manager-error vault-problem" role="alert">
                            ⚠ {vaultError() ?? vaultProblem()}
                        </div>
                    </Show>
                    <TabbedPane items={RAIL} value={section()} onChange={setSection} ariaLabel="Vault section" panelClass="bundle-manager-section">
                        <For each={ENTRY_SECTIONS}>
                            {(s) => (
                                <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== s }}>
                                    <VaultEntries section={s} vault={vault()} />
                                </div>
                            )}
                        </For>
                        <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "kept" }}>
                            <VaultKept visible={section() === "kept"} />
                        </div>
                        <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "activity" }}>
                            <VaultActivityLog visible={section() === "activity"} />
                        </div>
                    </TabbedPane>
                    <Show when={form()}>{(target) => <VaultEntryForm target={target()} onClose={() => setForm(null)} />}</Show>
                    <Show when={model.transfer() === "export"}>
                        <VaultExportDialog onClose={() => model.openTransfer(null)} />
                    </Show>
                    <Show when={model.transfer() === "import"}>
                        <VaultImportDialog into={vault() || undefined} onClose={() => model.openTransfer(null)} />
                    </Show>
                    <Show when={managing()}>
                        <VaultManageDialog onClose={() => setManaging(false)} />
                    </Show>
                </Show>
            </div>
        </div>
        </VaultShown.Provider>
    );
}

VaultView.displayName = "VaultView";
