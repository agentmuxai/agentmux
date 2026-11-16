// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane's socket to srv: one per pane, opened when the pane mounts and closed when it goes. Frames and messages
// are handed to the caller; nothing is kept here. docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §4, §7.

import { newWebSocket } from "@/util/wsutil";
import { frameBytes, parseServerMessage, type ServerMsg } from "./streamed-protocol";

export const STREAMED_BROWSER_PATH = "/agentmux/streamed-browser";

export interface StreamedEvents {
    onFrame(jpeg: Uint8Array): void;
    onMessage(message: ServerMsg): void;
    onOpen(): void;
    /** The socket ended: `failed` when it never opened, `closed` after it had. */
    onEnd(how: "failed" | "closed"): void;
}

export interface StreamedHandle {
    send(message: object): void;
    close(): void;
}

export function streamedUrl(wsBase: string, start: { url: string; w: number; h: number }): string {
    const q = new URLSearchParams({ w: String(start.w), h: String(start.h) });
    if (start.url) q.set("url", start.url);
    return `${wsBase}${STREAMED_BROWSER_PATH}?${q}`;
}

/** Open a pane's socket at `start`. */
export function openStreamedPane(start: { url: string; w: number; h: number }, events: StreamedEvents): StreamedHandle {
    const wsBase = window.__WAVE_SERVER_WS_ENDPOINT__ ?? window.location.origin.replace(/^http/, "ws");
    const ws = newWebSocket(streamedUrl(wsBase, start), {});
    ws.binaryType = "arraybuffer";
    let opened = false;
    let ended = false;
    const end = (how: "failed" | "closed") => {
        if (ended) return;
        ended = true;
        events.onEnd(how);
    };
    ws.onopen = () => {
        opened = true;
        events.onOpen();
    };
    ws.onmessage = (e: MessageEvent) => {
        if (typeof e.data === "string") {
            const m = parseServerMessage(e.data);
            if (m) events.onMessage(m);
            return;
        }
        const jpeg = e.data instanceof ArrayBuffer ? frameBytes(e.data) : null;
        if (jpeg) events.onFrame(jpeg);
    };
    ws.onerror = () => end(opened ? "closed" : "failed");
    ws.onclose = () => end(opened ? "closed" : "failed");
    return {
        send(message) {
            if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(message));
        },
        close() {
            ended = true;
            ws.onclose = null;
            ws.onerror = null;
            ws.close();
        },
    };
}
