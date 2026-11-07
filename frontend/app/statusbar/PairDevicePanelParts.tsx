// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pairing panel's views, over a state from PairDevicePanelState.ts: the buttons, the notes and the QR.

import { Button } from "@/app/element/ui";
import { Show, type Accessor, type JSX } from "solid-js";
import type { PairDeviceState } from "./PairDevicePanelState";

export function PairButtons(props: { s: PairDeviceState; lan: Accessor<boolean> }): JSX.Element {
    const { s, lan } = props;
    return (
        <div class="status-bar-popover-row" style={{ "padding-left": "12px" }}>
            <Show
                when={s.pairing()}
                fallback={
                    <Button density="compact" disabled={!lan() || s.busy()} onClick={() => void s.start()}>
                        Pair a device
                    </Button>
                }
            >
                <Button density="compact" onClick={s.hide}>
                    Hide
                </Button>
                <Button density="compact" disabled={s.busy()} onClick={() => void s.start()}>
                    New code
                </Button>
                <Button density="compact" disabled={!s.live()} onClick={s.copyLink}>
                    {s.copied() ? "Copied" : "Copy link"}
                </Button>
            </Show>
        </div>
    );
}

export function PairNotes(props: { s: PairDeviceState; lan: Accessor<boolean> }): JSX.Element {
    const { s, lan } = props;
    return (
        <>
            <Show when={!lan()}>
                <div class="status-bar-pair-note">
                    Turn on LAN discovery to pair a device: AgentMux Mobile connects to this computer over your local
                    network.
                </div>
            </Show>
            <Show when={s.error()}>
                <div class="status-bar-pair-note status-bar-pair-note--warn" role="alert">
                    ⚠ {s.error()}
                </div>
            </Show>
            <Show when={s.paired()}>
                <div class="status-bar-pair-note" role="status">
                    Paired {s.paired()}. Settings › Paired devices lists it.
                </div>
            </Show>
        </>
    );
}

function QrPanel(props: { s: PairDeviceState }): JSX.Element {
    const s = props.s;
    return (
        <div class="status-bar-qr-panel">
            <canvas ref={s.setCanvas} class="status-bar-qr-canvas" aria-label="Pairing QR code" />
            <Show when={s.qrFailed()}>
                <div class="status-bar-pair-note status-bar-pair-note--warn" role="alert">
                    ⚠ The QR code couldn't be drawn. Copy link still works.{" "}
                    <Button density="compact" onClick={s.retryQr}>
                        Try again
                    </Button>
                </div>
            </Show>
            <div class="status-bar-qr-note" data-testid="pair-countdown">
                Scan with AgentMux Mobile. Works once, for {s.left()}.
            </div>
            <div class="status-bar-qr-note">
                Pairing lets that device watch this computer's agents, read-only (and its processes, if you share them
                in Tower). Show this code only to your own devices. Another AgentMux computer pairs with the copied
                link, in its Tower.
            </div>
        </div>
    );
}

/** The code on screen: the QR with its notes, or the line saying it expired. */
export function PairCode(props: { s: PairDeviceState }): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.pairing()}>
            <Show
                when={s.live()}
                fallback={
                    <div class="status-bar-pair-note" role="status">
                        This code expired. Choose New code to show another.
                    </div>
                }
            >
                <QrPanel s={s} />
            </Show>
        </Show>
    );
}
