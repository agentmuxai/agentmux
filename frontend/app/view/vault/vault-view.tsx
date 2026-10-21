// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault, Warden's Vault section (vault-in-warden.ts): the person's secrets, which agents use by name without
// seeing them (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5).
// A rail of sections like Warden's; agents' requests for new entries on top.

import { createEffect, createSignal, For, on, onCleanup, onMount, Show, type JSX } from "solid-js";

import { ModalLayer } from "@/app/element/ModalLayer";
import { Button, TabbedPane, type TabItem } from "@/app/element/ui";
import { openWarden } from "@/app/view/warden/open-warden";
import type { VaultActivity } from "@/types/rpc/VaultActivity";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import type { VaultSection } from "@/types/rpc/VaultSection";
import { getPlatform } from "@/util/platformutil";
import { VaultEntries } from "./vault-entries";
import { VaultEntryForm, type VaultFormTarget } from "./vault-entry-form";
import { VaultExportDialog } from "./vault-export";
import { formatWhen } from "./vault-format";
import { VaultImportDialog } from "./vault-import";
import { VAULT_SECTIONS, vaultKind, type VaultPaneSection } from "./vault-kinds";
import type { VaultViewModel } from "./vault-model";
import { vaultPromptNotes } from "./vault-prompts";
import { VaultShown } from "./vault-reveal";
import {
    declineRequest,
    followVault,
    loadActivity,
    loadKept,
    refreshVault,
    setVaultOnScreen,
    vaultAvailable,
    vaultChangedMs,
    vaultError,
    vaultProblem,
    vaultRequests,
    type KeptSecret,
} from "./vault-store";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./vault-view.scss";

const ENTRY_SECTIONS: VaultSection[] = ["personal", "cards", "api-keys", "logins", "notes"];

const RAIL: TabItem<VaultPaneSection>[] = [...VAULT_SECTIONS.items];

const MANAGED_IN: Record<string, string> = {
    accounts: "the Accounts pane",
    muxbus: "Settings, under cloud messages",
    tower: "Tower",
};

function VaultRequests(props: { onFill: (r: VaultRequest) => void }): JSX.Element {
    const [error, setError] = createSignal<string | null>(null);
    const dismiss = async (r: VaultRequest) => {
        try {
            await declineRequest(r.id);
            setError(null);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <Show when={vaultRequests().length > 0}>
            <div class="vault-requests" role="region" aria-label="Agents' requests">
                <For each={vaultRequests()}>
                    {(r) => (
                        <div class="vault-request" data-request={r.id}>
                            <div class="vault-request-text">
                                <strong>{r.agent}</strong> asks for a {vaultKind(r.kind)?.label.toLowerCase() ?? r.kind}
                                : <strong>{r.label}</strong>
                                <Show when={r.why}>
                                    <div class="warden-manager-dim">{r.why}</div>
                                </Show>
                            </div>
                            <Button tone="attention" density="compact" onClick={() => props.onFill(r)}>
                                Fill in
                            </Button>
                            <Button tone="quiet" density="compact" onClick={() => void dismiss(r)}>
                                Dismiss
                            </Button>
                        </div>
                    )}
                </For>
                <Show when={error()}>
                    <div class="warden-manager-error">⚠ {error()}</div>
                </Show>
            </div>
        </Show>
    );
}

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
            <Button tone="quiet" density="compact" onClick={props.onClose}>
                Got it
            </Button>
        </div>
    );
}

function VaultKept(props: { visible: boolean }): JSX.Element {
    const [kept, setKept] = createSignal<KeptSecret[] | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    createEffect(() => {
        if (props.visible && kept() === null) {
            loadKept()
                .then(setKept)
                .catch((e) => setError(String(e)));
        }
    });
    return (
        <div class="warden-manager-body">
            <p class="warden-manager-summary">
                {
                    "Secrets AgentMux keeps for itself, in the same keychain. They're managed where they're used; the Vault only lists them."
                }
            </p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show
                when={(kept() ?? []).length > 0}
                fallback={<div class="warden-section-stub">{kept() === null ? "Loading…" : "None yet."}</div>}
            >
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
            <p class="warden-manager-footnote">{"Sign-ins saved by browser panes aren't listed here yet."}</p>
        </div>
    );
}

function VaultActivityLog(props: { visible: boolean }): JSX.Element {
    const [rows, setRows] = createSignal<VaultActivity[] | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const load = () =>
        loadActivity()
            .then((r) => {
                setRows(r);
                setError(null);
            })
            .catch((e) => setError(String(e)));
    // Loads when first shown, and again whenever the Vault changes while shown.
    createEffect(
        on([() => props.visible, vaultChangedMs], ([visible]) => {
            if (visible) void load();
        })
    );
    return (
        <div class="warden-manager-body">
            <p class="warden-manager-summary">
                Every change to the Vault and every use of an entry: who, what, where. Never a value.{" "}
                <Button density="compact" onClick={() => void openWarden("audit")}>
                    See agents' use in Warden
                </Button>
            </p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show
                when={(rows() ?? []).length > 0}
                fallback={<div class="warden-section-stub">{rows() === null ? "Loading…" : "Nothing yet."}</div>}
            >
                <table class="warden-manager-table">
                    <thead>
                        <tr>
                            <th>When</th>
                            <th>Who</th>
                            <th>What</th>
                            <th>Entry</th>
                            <th>Where</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={rows()}>
                            {(a) => (
                                <tr data-outcome={a.outcome}>
                                    <td class="warden-manager-dim">{formatWhen(a.at)}</td>
                                    <td>{a.actor === "you" ? "You" : a.actor}</td>
                                    <td>{a.action}</td>
                                    <td>{a.entry_label}</td>
                                    <td class="warden-manager-dim">{a.target}</td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
            </Show>
        </div>
    );
}

export function VaultView(props: { model: VaultViewModel }): JSX.Element {
    const model = props.model;
    const section = model.sectionAtom;
    const setSection = (id: VaultPaneSection) => model.setMeta({ "vault:section": id });
    const [form, setForm] = createSignal<VaultFormTarget | null>(null);

    const fill = (r: VaultRequest) => {
        const kind = vaultKind(r.kind);
        if (kind) setSection(kind.section);
        setForm({ mode: "new", section: kind?.section ?? "personal", request: r });
    };

    onMount(() => {
        if (!vaultAvailable()) return;
        followVault();
        void refreshVault();
    });

    // While shown, renew srv's note that a Vault pane is on screen every 10 s
    // (it lasts 30 s); clear it when hidden or closed. Notes go one at a time,
    // so a quick hide and show can't land as show-then-clear.
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
        // data-agent-hidden: agents' UI captures and queries leave the Vault out.
        <VaultShown.Provider value={model.shown}>
            <ModalLayer scope="pane">
                <div class="vault-container" data-agent-hidden="vault">
                    <div class="vault-view" style={{ zoom: model.zoomAtom() }}>
                        <Show
                            when={vaultAvailable()}
                            fallback={
                                <div class="warden-section-stub vault-unavailable">
                                    The Vault needs the AgentMux desktop app.
                                </div>
                            }
                        >
                            <Show when={model.promptsOpen()}>
                                <VaultPromptsNote onClose={() => model.setMeta({ "vault:prompts-read": true })} />
                            </Show>
                            <VaultRequests onFill={fill} />
                            <Show when={vaultError() ?? vaultProblem()}>
                                <div class="warden-manager-error vault-problem" role="alert">
                                    ⚠ {vaultError() ?? vaultProblem()}
                                </div>
                            </Show>
                            <TabbedPane
                                items={RAIL}
                                value={section()}
                                onChange={setSection}
                                ariaLabel="Vault section"
                                panelClass="bundle-manager-section"
                            >
                                <For each={ENTRY_SECTIONS}>
                                    {(s) => (
                                        <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== s }}>
                                            <VaultEntries section={s} />
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
                            <Show when={form()}>
                                {(target) => <VaultEntryForm target={target()} onClose={() => setForm(null)} />}
                            </Show>
                            <Show when={model.transfer() === "export"}>
                                <VaultExportDialog onClose={() => model.openTransfer(null)} />
                            </Show>
                            <Show when={model.transfer() === "import"}>
                                <VaultImportDialog onClose={() => model.openTransfer(null)} />
                            </Show>
                        </Show>
                    </div>
                </div>
            </ModalLayer>
        </VaultShown.Provider>
    );
}

VaultView.displayName = "VaultView";
