// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// "Pair a device" in the host popover: a QR the AgentMux Mobile app scans to
// pair with this computer's viewer (crates/srv/src/backend/viewer). The QR
// carries a one-time code and the fingerprint of this computer's certificate,
// never this instance's auth key. It works for 2 minutes; "New code" cancels
// it and shows another. Pairing needs LAN discovery on, because the device
// connects over this network.

import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { viewerPairedAtom } from "@/store/global";
import QRCode from "qrcode";
import { createEffect, createMemo, createSignal, on, onCleanup, Show, type Accessor, type JSX } from "solid-js";

type Pairing = { url: string; expiresMs: number };

/** `m:ss` left until `expiresMs`, or null once it has passed. */
export function countdown(expiresMs: number, nowMs: number): string | null {
    const left = Math.ceil((expiresMs - nowMs) / 1000);
    if (left <= 0) return null;
    return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
}

export function PairDevicePanel(props: { lanDiscoveryEnabled: Accessor<boolean> }): JSX.Element {
    const [pairing, setPairing] = createSignal<Pairing | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    // "Copy link" said so; a new code needs copying again.
    const [copied, setCopied] = createSignal(false);
    createEffect(on(pairing, () => setCopied(false)));
    const [paired, setPaired] = createSignal<string | null>(null);
    const [now, setNow] = createSignal(Date.now());
    let canvas: HTMLCanvasElement | undefined;

    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    onCleanup(() => window.clearInterval(timer));

    const left = () => {
        const p = pairing();
        return p ? countdown(p.expiresMs, now()) : null;
    };
    const live = createMemo(() => left() !== null);

    const start = async () => {
        setBusy(true);
        setError(null);
        setPaired(null);
        try {
            const r = await RpcApi.ViewerPairStartCommand(TabRpcClient);
            setPairing({ url: r.url, expiresMs: r.expires_ms });
            setNow(Date.now());
        } catch (e) {
            setPairing(null);
            setError(String(e instanceof Error ? e.message : e));
        } finally {
            setBusy(false);
        }
    };

    // A device paired while the QR is up: say so and take the QR down (its
    // code is spent).
    createEffect(
        on(
            viewerPairedAtom,
            (p) => {
                if (!p || !pairing()) return;
                setPairing(null);
                setPaired(p.deviceName);
            },
            { defer: true }
        )
    );

    // LAN turned off: the listener is gone, so is the QR's use.
    createEffect(() => {
        if (!props.lanDiscoveryEnabled()) setPairing(null);
    });

    createEffect(() => {
        const p = pairing();
        if (!p || !live() || !canvas) return;
        QRCode.toCanvas(canvas, p.url, { width: 176, margin: 1 }, (err) => {
            // Never log the URL: it carries the pairing code.
            if (err) console.error("[PairDevicePanel] failed to render the pairing QR code:", err.message);
        });
    });

    return (
        <div class="status-bar-pair" data-testid="pair-device">
            <div class="status-bar-popover-row" style={{ "padding-left": "12px" }}>
                <Show
                    when={pairing()}
                    fallback={
                        <Button
                            density="compact"
                            disabled={!props.lanDiscoveryEnabled() || busy()}
                            onClick={() => void start()}
                        >
                            Pair a device
                        </Button>
                    }
                >
                    <Button density="compact" onClick={() => setPairing(null)}>
                        Hide
                    </Button>
                    <Button density="compact" disabled={busy()} onClick={() => void start()}>
                        New code
                    </Button>
                    <Button
                        density="compact"
                        disabled={!live()}
                        onClick={() => {
                            const p = pairing();
                            if (p) void navigator.clipboard.writeText(p.url).then(() => setCopied(true));
                        }}
                    >
                        {copied() ? "Copied" : "Copy link"}
                    </Button>
                </Show>
            </div>
            <Show when={!props.lanDiscoveryEnabled()}>
                <div class="status-bar-pair-note">
                    Turn on LAN discovery to pair a device: AgentMux Mobile connects to this computer over your local
                    network.
                </div>
            </Show>
            <Show when={error()}>
                <div class="status-bar-pair-note status-bar-pair-note--warn" role="alert">
                    ⚠ {error()}
                </div>
            </Show>
            <Show when={paired()}>
                <div class="status-bar-pair-note" role="status">
                    Paired {paired()}. Settings › Paired devices lists it.
                </div>
            </Show>
            <Show when={pairing()}>
                <Show
                    when={live()}
                    fallback={
                        <div class="status-bar-pair-note" role="status">
                            This code expired. Choose New code to show another.
                        </div>
                    }
                >
                    <div class="status-bar-qr-panel">
                        <canvas ref={canvas} class="status-bar-qr-canvas" aria-label="Pairing QR code" />
                        <div class="status-bar-qr-note" data-testid="pair-countdown">
                            Scan with AgentMux Mobile. Works once, for {left()}.
                        </div>
                        <div class="status-bar-qr-note">
                            Pairing lets that device watch this computer's agents, read-only (and its processes, if you
                            share them in Tower). Show this code only to your own devices. Another AgentMux computer
                            pairs with the copied link, in its Tower.
                        </div>
                    </div>
                </Show>
            </Show>
        </div>
    );
}
