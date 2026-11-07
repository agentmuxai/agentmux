// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pairing panel's state: the one-time code and how long it lives, the note a pairing leaves, copying the link,
// and drawing the code as a QR.

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { viewerPairedAtom } from "@/store/global";
import { writeText } from "@/util/clipboard";
import { createEffect, createMemo, createSignal, on, onCleanup, type Accessor, type Setter } from "solid-js";

type Pairing = { url: string; expiresMs: number };

export type PairDeviceState = ReturnType<typeof createPairDevice>;

/** `m:ss` left until `expiresMs`, or null once it has passed. */
export function countdown(expiresMs: number, nowMs: number): string | null {
    const left = Math.ceil((expiresMs - nowMs) / 1000);
    if (left <= 0) return null;
    return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
}

/** Draws the live code on the canvas; the QR library loads with the first code, not at startup. */
function drawQr(args: {
    pairing: Accessor<Pairing | null>;
    live: Accessor<boolean>;
    canvas: () => HTMLCanvasElement | undefined;
    attempt: Accessor<number>;
    setFailed: Setter<boolean>;
}): void {
    const { pairing, live, canvas, attempt, setFailed } = args;
    createEffect(() => {
        const p = pairing();
        attempt();
        const target = canvas();
        if (!p || !live() || !target) return;
        // Loaded when a code is first drawn, not at startup
        // (SPEC_FRONTEND_BUNDLE_SPLITTING_2026_10_21.md §5).
        import("qrcode")
            .then(({ default: QRCode }) => {
                // Pairing may have ended, or a new code replaced this one, meanwhile.
                if (pairing() !== p || !live()) return;
                setFailed(false);
                QRCode.toCanvas(target, p.url, { width: 176, margin: 1 }, (err) => {
                    // Never log the URL: it carries the pairing code.
                    if (err) console.error("[PairDevicePanel] failed to render the pairing QR code:", err.message);
                });
            })
            .catch((err) => {
                console.error("[PairDevicePanel] failed to load the QR code library:", err?.message ?? err);
                setFailed(true);
            });
    });
}

export function createPairDevice(props: { lanDiscoveryEnabled: Accessor<boolean> }) {
    const [pairing, setPairing] = createSignal<Pairing | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    // "Copy link" said so; a new code needs copying again.
    const [copied, setCopied] = createSignal(false);
    createEffect(on(pairing, () => setCopied(false)));
    const [paired, setPaired] = createSignal<string | null>(null);
    const [now, setNow] = createSignal(Date.now());
    let canvas: HTMLCanvasElement | undefined;
    // The QR library loads with the first code; if it can't, the panel says so
    // and offers to try again (bumping `qrAttempt` re-runs the draw).
    const [qrFailed, setQrFailed] = createSignal(false);
    const [qrAttempt, setQrAttempt] = createSignal(0);

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

    drawQr({ pairing, live, canvas: () => canvas, attempt: qrAttempt, setFailed: setQrFailed });

    const copyLink = () => {
        const p = pairing();
        if (!p) return;
        // Through the host: CEF blocks navigator.clipboard.
        writeText(p.url).then(
            () => setCopied(true),
            () => setError("Couldn't copy the link to the clipboard.")
        );
    };

    return {
        pairing,
        error,
        busy,
        copied,
        paired,
        qrFailed,
        left,
        live,
        start,
        copyLink,
        hide: () => setPairing(null),
        retryQr: () => setQrAttempt((n) => n + 1),
        setCanvas: (el: HTMLCanvasElement) => {
            canvas = el;
        },
    };
}
