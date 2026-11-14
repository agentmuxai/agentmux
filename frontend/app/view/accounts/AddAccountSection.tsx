// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The section that opens under a brand tile when the person adds an account in Connectors: the same
// caret-topped section a connected account's details use (CardGrid's `renderDetail`), not a dialog
// over the pane. It asks how to connect (a sign-in or a key), then the sign-in or the form runs in
// it, and it collapses when the account is saved or the person cancels.

import { Button, CloseButton } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { AccountForm } from "@/app/view/identity/identity-account-form";
import type { AccountKind, IdentityViewModel } from "@/app/view/identity/identity-model";
import { ProviderLogo } from "@/element/ProviderLogo";
import { createEffect, createSignal, For, on, onCleanup, Show, type JSX } from "solid-js";
import { modeLabel, type AuthMode, type ServiceTile } from "./accounts-catalog";
import { AgentMuxConnectBody, type MuxBusController } from "./AgentMuxConnectPanel";
import { ClaudeLoginPanel } from "./ClaudeLoginPanel";
import { githubStatusLine } from "./github-account-copy";

export function AddAccountSection(props: {
    tile: ServiceTile;
    model: IdentityViewModel;
    muxbus: MuxBusController;
    /** Collapse the section. */
    onClose: () => void;
}): JSX.Element {
    const { tile, model } = props;
    // Anthropic's sign-in is the CLI login (its own panel); every other choice is the account form,
    // preset to the provider and kind.
    const [mode, setMode] = createSignal<AuthMode | null>(null);
    const claudeSignIn = () => tile.id === "anthropic" && mode() === "oauth";
    // Whether this computer has gh-agent, which srv knows and the frontend can't: it is GitHub's status then.
    const [ghAgentHere, setGhAgentHere] = createSignal(false);
    if (tile.id === "github") {
        void RpcApi.GithubConnectionCommand(TabRpcClient, {})
            .then((c) => setGhAgentHere(c.ghAgentOnThisComputer === true))
            .catch(() => {});
    }

    let openedForm = false;
    const pick = (picked: AuthMode) => {
        if (!(tile.id === "anthropic" && picked === "oauth")) {
            const kind: AccountKind = picked === "oauth" ? "oauth" : tile.keyKind;
            model.openAddFormFor(tile.id, kind);
            openedForm = true;
        }
        setMode(picked);
    };
    // A tile with one way to connect goes straight to it.
    if (tile.authModes.length === 1 && tile.id !== "agentmux") pick(tile.authModes[0]);

    // The form closing (saved or cancelled) collapses the section; collapsing it closes the form.
    createEffect(on(model.formOpenAtom, (open, wasOpen) => wasOpen && !open && props.onClose(), { defer: true }));
    // Only the form this section opened: when the person picks another tile, the new section may
    // have opened its own form before this one is torn down.
    onCleanup(() => openedForm && model.formOpenAtom() && model.addPresetAtom()?.provider === tile.id && model.cancelForm());

    return (
        <div class="accounts-add-section" role="group" aria-label={`Connect ${tile.displayName}`}>
            <div class="accounts-add-header">
                <span class="account-tile-logo">
                    <ProviderLogo provider={tile.id} size={20} />
                </span>
                <span class="accounts-chooser-title">Connect {tile.displayName}</span>
                <CloseButton tooltip={false} onClick={() => props.onClose()} />
            </div>
            <Show when={tile.id === "agentmux"}>
                <AgentMuxConnectBody muxbus={props.muxbus} />
            </Show>
            <Show when={tile.id === "github"}>
                <div class="accounts-add-status">
                    <p>{githubStatusLine(model.accountsByProvider().get("github") ?? [], ghAgentHere())}</p>
                </div>
            </Show>
            <Show when={tile.id !== "agentmux" && mode() === null}>
                <div class="accounts-chooser accounts-chooser--inline">
                    <div class="accounts-chooser-modes">
                        <For each={tile.authModes}>
                            {(m) => {
                                const label = modeLabel(m, tile.id);
                                return (
                                    <Button tone="neutral" class="accounts-chooser-mode" onClick={() => pick(m)}>
                                        <span class="accounts-chooser-mode-title">{label.title}</span>
                                        <span class="accounts-chooser-mode-sub">{label.sub}</span>
                                    </Button>
                                );
                            }}
                        </For>
                    </div>
                </div>
            </Show>
            <Show when={claudeSignIn()}>
                <ClaudeLoginPanel inline onClose={() => props.onClose()} />
            </Show>
            <Show when={mode() !== null && !claudeSignIn() && model.formOpenAtom() && !model.editingAccountAtom()}>
                <AccountForm model={model} inline />
            </Show>
        </div>
    );
}

AddAccountSection.displayName = "AddAccountSection";
