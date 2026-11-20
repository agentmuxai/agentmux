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
import { PROVIDERS } from "@/app/view/agent/providers/catalog";
import { ProviderGuideDetails, ProviderGuideInside } from "./ProviderGuideDetails";
import { GoogleServiceConnect } from "./GoogleServiceConnect";
import { SERVICES, type GoogleService } from "./google-services";
import { ProviderLoginPanel } from "./ProviderLoginPanel";
import { guideFor, optionForMode, shown } from "./provider-guide";
import { githubStatusLine } from "./github-account-copy";
import "./accounts-gallery.scss";

/** The ways to connect, when there is more than one: a button for each, with what it means. */
function ModeChooser(props: {
    modes: AuthMode[];
    label: (mode: AuthMode) => { title: string; sub: string };
    onPick: (mode: AuthMode) => void;
}): JSX.Element {
    return (
        <div class="accounts-chooser accounts-chooser--inline">
            <div class="accounts-chooser-modes">
                <For each={props.modes}>
                    {(m) => {
                        const label = props.label(m);
                        return (
                            <Button tone="neutral" class="accounts-chooser-mode" onClick={() => props.onPick(m)}>
                                <span class="accounts-chooser-mode-title">{label.title}</span>
                                <span class="accounts-chooser-mode-sub">{label.sub}</span>
                            </Button>
                        );
                    }}
                </For>
            </div>
        </div>
    );
}

export function AddAccountSection(props: {
    tile: ServiceTile;
    /** What the card is called, when it is not the tile's name (a Google service). */
    name?: string;
    /** A Google service's card: the sign-in asks for just that service, instead of choosing how to connect. */
    service?: GoogleService;
    model: IdentityViewModel;
    muxbus: MuxBusController;
    /** Collapse the section. */
    onClose: () => void;
}): JSX.Element {
    const { tile, model } = props;
    const name = () => props.name ?? tile.displayName;
    const scoped = props.service !== undefined;
    // A provider CLI's sign-in (Anthropic, OpenAI) is its own panel; every other choice is the account form,
    // preset to the provider and kind.
    const [mode, setMode] = createSignal<AuthMode | null>(null);
    const signInProvider = () => (tile.signIn && mode() === "oauth" ? PROVIDERS[tile.signIn.provider] : undefined);
    const guide = guideFor(tile.id);
    // A way to connect says what it is in the guide's words; a tile with none keeps the generic two.
    const labelFor = (m: AuthMode) => {
        const o = optionForMode(guide, m);
        const what = shown(o?.what);
        return o && what ? { title: o.title, sub: what.value } : modeLabel(m, tile.id);
    };
    // Whether this computer has gh-agent, which srv knows and the frontend can't: it is GitHub's status then.
    const [ghAgentHere, setGhAgentHere] = createSignal(false);
    if (tile.id === "github") {
        void RpcApi.GithubConnectionCommand(TabRpcClient, {})
            .then((c) => setGhAgentHere(c.ghAgentOnThisComputer === true))
            .catch(() => {});
    }

    let openedForm = false;
    const pick = (picked: AuthMode) => {
        if (!(tile.signIn && picked === "oauth")) {
            const kind: AccountKind = picked === "oauth" ? "oauth" : tile.keyKind;
            model.openAddFormFor(tile.id, kind);
            openedForm = true;
        }
        setMode(picked);
    };
    // A tile with one way to connect goes straight to it.
    if (!scoped && tile.authModes.length === 1 && tile.id !== "agentmux") pick(tile.authModes[0]);

    // The form closing (saved or cancelled) collapses the section; collapsing it closes the form.
    createEffect(on(model.formOpenAtom, (open, wasOpen) => wasOpen && !open && props.onClose(), { defer: true }));
    // Only the form this section opened: when the person picks another tile, the new section may
    // have opened its own form before this one is torn down.
    onCleanup(() => openedForm && model.formOpenAtom() && model.addPresetAtom()?.provider === tile.id && model.cancelForm());

    return (
        <div class="accounts-add-section" role="group" aria-label={`Connect ${name()}`}>
            <div class="accounts-add-header">
                <span class="account-tile-logo">
                    <ProviderLogo provider={tile.id} size={20} />
                </span>
                <span class="accounts-chooser-title">Connect {name()}</span>
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
            <Show when={guide}>{(g) => <ProviderGuideInside guide={g()} />}</Show>
            {/* A Google service asks for its own scopes and nothing else: no choice of how to connect. */}
            <Show when={SERVICES.find((x) => x.service === props.service)}>
                {(info) => <GoogleServiceConnect info={info()} onClose={() => props.onClose()} />}
            </Show>
            <Show when={!scoped && tile.id !== "agentmux" && tile.authModes.length > 0 && mode() === null}>
                <ModeChooser modes={tile.authModes} label={labelFor} onPick={pick} />
            </Show>
            <Show when={signInProvider()}>
                {(p) => (
                    // A sign-in that worked collapses the section at once: no "Signed in" step to dismiss (§4.9); the card says it.
                    <ProviderLoginPanel
                        inline
                        provider={p()}
                        brand={tile.signIn?.brand}
                        onSignedIn={() => props.onClose()}
                        onClose={() => props.onClose()}
                    />
                )}
            </Show>
            <Show when={mode() !== null && !signInProvider() && model.formOpenAtom() && !model.editingAccountAtom()}>
                <AccountForm model={model} inline />
            </Show>
            <Show when={guide}>{(g) => <ProviderGuideDetails guide={g()} />}</Show>
        </div>
    );
}

AddAccountSection.displayName = "AddAccountSection";
