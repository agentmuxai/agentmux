// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane's socket to srv: one per pane, opened when the pane mounts and closed when it goes. Frames and messages
// are handed to the caller; nothing is kept here. docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §4, §7.

import { newWebSocket } from "@/util/wsutil";
import { ensureLease, releaseOnPageHide } from "./lease-step";
import { frameBytes, parseServerMessage, type ServerMsg } from "./streamed-protocol";

export const STREAMED_BROWSER_PATH = "/agentmux/streamed-browser";

export interface StreamedEvents {
    onFrame(jpeg: Uint8Array): void;
    onMessage(message: ServerMsg): void;
    onOpen(): void;
    /** The socket ended: `failed` when it never opened, `closed` after it had. `reason` is the server's own words when it refused. */
    onEnd(how: "failed" | "closed", reason?: string): void;
    /** The server's browser is still starting (before the socket opens). */
    onStarting?(): void;
}

export interface StreamedHandle {
    send(message: object): void;
    close(): void;
}

/**
 * Screen mode (an ordinary Chrome on a virtual screen, nothing attached to it) is asked for with `?screen=1` on the page or the
 * local setting `streamedBrowserScreen` set to `1`. The server answers only when it has that mode on.
 */
export function screenModeAsked(): boolean {
    try {
        return new URLSearchParams(window.location.search).get("screen") === "1" || window.localStorage.getItem("streamedBrowserScreen") === "1";
    } catch {
        return false;
    }
}

export function streamedUrl(wsBase: string, start: { url: string; w: number; h: number }, screen = false): string {
    const q = new URLSearchParams({ w: String(start.w), h: String(start.h) });
    if (start.url) q.set("url", start.url);
    if (screen) q.set("mode", "screen");
    return `${wsBase}${STREAMED_BROWSER_PATH}?${q}`;
}

/**
 * Open a pane's socket at `start`: after the server has said the pane's browser is ready, when it hands one out per person
 * (lease-step.ts). `onStarting` is called while that wait goes on; a server that refuses ends the pane as `failed` with its words.
 */
export function openStreamedPane(start: { url: string; w: number; h: number }, events: StreamedEvents): StreamedHandle {
    let ws: WebSocket | null = null;
    let opened = false;
    let ended = false;
    const waiting = new AbortController();
    const end = (how: "failed" | "closed", reason?: string) => {
        if (ended) return;
        ended = true;
        events.onEnd(how, reason);
    };
    const connect = () => {
        const wsBase = window.__WAVE_SERVER_WS_ENDPOINT__ ?? window.location.origin.replace(/^http/, "ws");
        const socket = newWebSocket(streamedUrl(wsBase, start, screenModeAsked()), {});
        ws = socket;
        socket.binaryType = "arraybuffer";
        socket.onopen = () => {
            opened = true;
            events.onOpen();
        };
        socket.onmessage = (e: MessageEvent) => {
            if (typeof e.data === "string") {
                const m = parseServerMessage(e.data);
                if (m) events.onMessage(m);
                return;
            }
            const jpeg = e.data instanceof ArrayBuffer ? frameBytes(e.data) : null;
            if (jpeg) events.onFrame(jpeg);
        };
        socket.onerror = () => end(opened ? "closed" : "failed");
        socket.onclose = () => end(opened ? "closed" : "failed");
    };
    releaseOnPageHide();
    void ensureLease({ signal: waiting.signal, onStarting: () => events.onStarting?.() }).then((lease) => {
        if (ended || waiting.signal.aborted) return;
        if (lease.ok) connect();
        else end("failed", lease.message || undefined);
    });
    return {
        send(message) {
            if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(message));
        },
        close() {
            ended = true;
            waiting.abort();
            if (ws) {
                ws.onclose = null;
                ws.onerror = null;
                ws.close();
            }
        },
    };
}
