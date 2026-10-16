// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// MuxBus: whether cloud messages (other computers, GitHub reviews and CI)
// reach this channel's agents, in one line, with Sign in or Sign out. The
// same state the status bar indicator shows (`@/store/muxbus-delivery`).

import { Button } from "@/app/element/ui";
import { onMount, Show, type JSX } from "solid-js";

import { useMuxBusStatus } from "@/app/view/accounts/AgentMuxConnectPanel";
import { MuxBusSignInChoices } from "@/app/view/accounts/muxbus-sign-in-choices";
import { muxbusDelivery, muxbusDeliveryLine, muxbusOffersSignIn } from "@/store/muxbus-delivery";
import type { SettingsIndexEntry } from "../settings-model";

export const MUXBUS_SIGN_IN_SETTING = {
    id: "devices.muxbus",
    label: "MuxBus",
    description:
        "MuxBus carries messages to this computer's agents from your other computers and from GitHub reviews and CI.",
    section: "devices",
    keywords: ["muxbus", "cloud", "sign in", "sign out", "login", "messages", "github", "reviews", "relay", "account"],
} satisfies SettingsIndexEntry;

export function MuxBusSignIn(): JSX.Element {
    const muxbus = useMuxBusStatus();
    onMount(() => void muxbus.refresh());
    const offersSignIn = () => muxbusOffersSignIn(muxbusDelivery());

    return (
        <div id={`setting-${MUXBUS_SIGN_IN_SETTING.id}`} class="setting-row">
            <div class="setting-devices-description">{MUXBUS_SIGN_IN_SETTING.description}</div>
            <div class="setting-presence">
                <span class="setting-presence-line" role="status">
                    {muxbusDeliveryLine(muxbusDelivery())}
                </span>
                <Show when={!offersSignIn()}>
                    <Button busy={muxbus.loading()} onClick={() => void muxbus.disconnect()}>
                        Sign out
                    </Button>
                </Show>
            </div>
            <Show when={offersSignIn()}>
                <div class="setting-presence-note">
                    <MuxBusSignInChoices
                        identityProviders={muxbus.identityProviders()}
                        loading={muxbus.loading()}
                        disabled={!muxbus.isConfigured()}
                        label="Sign in"
                        cancelLabel="Cancel sign-in"
                        onSignIn={(provider) => void muxbus.connect(provider)}
                        onCancel={() => void muxbus.cancel()}
                    />
                </div>
            </Show>
            <Show when={muxbus.error()}>
                {(error) => (
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                )}
            </Show>
        </div>
    );
}
