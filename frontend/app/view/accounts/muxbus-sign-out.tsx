// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one Sign out for AgentMux Cloud (MuxBus), used wherever it is offered.
 * While MuxBus can't be reached it asks first, in place: the stored sign-in
 * would reconnect by itself, and once deleted it can't be replaced until
 * MuxBus is reachable again.
 */

import { Button } from "@/app/element/ui";
import { createEffect, createSignal, Show, type JSX } from "solid-js";

import { muxbusDelivery, muxbusSignOutWarning } from "@/store/muxbus-delivery";
import type { MuxBusController } from "./AgentMuxConnectPanel";

import "./muxbus-sign-out.scss";

export interface MuxBusSignOutButtonProps {
    muxbus: MuxBusController;
    /** The button's text; "Sign out" by default. */
    label?: string;
    class?: string;
    title?: string;
}

export function MuxBusSignOutButton(props: MuxBusSignOutButtonProps): JSX.Element {
    const [confirming, setConfirming] = createSignal(false);
    const warning = () => muxbusSignOutWarning(muxbusDelivery());
    // Back online while asking: nothing left to warn about.
    createEffect(() => {
        if (!warning()) setConfirming(false);
    });
    const signOut = () => {
        setConfirming(false);
        void props.muxbus.disconnect();
    };

    return (
        <Show
            when={confirming() && warning()}
            fallback={
                <Button
                    class={props.class}
                    title={props.title}
                    busy={props.muxbus.loading()}
                    onClick={() => (warning() ? setConfirming(true) : signOut())}
                >
                    {props.label ?? "Sign out"}
                </Button>
            }
        >
            {(text) => (
                <div class="muxbus-signout-confirm" role="alert">
                    <span class="muxbus-signout-warning">{text()}</span>
                    <div class="muxbus-signout-actions">
                        <Button tone="accent" onClick={() => setConfirming(false)}>
                            Keep my sign-in
                        </Button>
                        <Button tone="danger" busy={props.muxbus.loading()} onClick={signOut}>
                            Sign out anyway
                        </Button>
                    </div>
                </div>
            )}
        </Show>
    );
}
