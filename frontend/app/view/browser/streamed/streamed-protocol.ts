// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a streamed Browser pane and srv say to each other, and how a mouse, wheel or key event of the page becomes a
// message. The same protocol as crates/srv/src/server/streamed_browser/protocol.rs;
// docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §4.

/** First byte of a binary message: a JPEG frame follows. */
export const FRAME_KIND = 1;

export type ServerMsg =
    | { t: "nav"; url: string; title: string; canGoBack: boolean; canGoForward: boolean; loading: boolean }
    | { t: "size"; w: number; h: number }
    | { t: "error"; message: string }
    | { t: "closed" };

/** The JPEG in a binary message, or null for any other kind (a client ignores kinds it does not know). */
export function frameBytes(data: ArrayBuffer): Uint8Array | null {
    const bytes = new Uint8Array(data);
    return bytes.length > 1 && bytes[0] === FRAME_KIND ? bytes.subarray(1) : null;
}

/** A text message from srv, or null when it is not one this pane knows. */
export function parseServerMessage(text: string): ServerMsg | null {
    let value: unknown;
    try {
        value = JSON.parse(text);
    } catch {
        return null;
    }
    if (typeof value !== "object" || value === null) return null;
    const m = value as Record<string, unknown>;
    switch (m.t) {
        case "nav":
            return {
                t: "nav",
                url: String(m.url ?? ""),
                title: String(m.title ?? ""),
                canGoBack: m.canGoBack === true,
                canGoForward: m.canGoForward === true,
                loading: m.loading === true,
            };
        case "size":
            return typeof m.w === "number" && typeof m.h === "number" ? { t: "size", w: m.w, h: m.h } : null;
        case "error":
            return { t: "error", message: String(m.message ?? "") };
        case "closed":
            return { t: "closed" };
        default:
            return null;
    }
}

export type Modifiers = { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean };

/** CDP's modifier bits: Alt 1, Ctrl 2, Meta 4, Shift 8. */
export function modifiers(e: Modifiers): number {
    return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
}

const BUTTONS = ["left", "middle", "right"] as const;

/** The button a `MouseEvent.button` names; `none` for a move or a button the page has no use for (back, forward). */
export function buttonName(button: number): "left" | "middle" | "right" | "none" {
    return BUTTONS[button] ?? "none";
}

type Point = { clientX: number; clientY: number };
type Box = { left: number; top: number };

export function mouseMessage(type: "down" | "up" | "move", e: Point & Modifiers & { button: number; buttons: number; detail: number }, box: Box) {
    return {
        t: "mouse",
        type,
        x: e.clientX - box.left,
        y: e.clientY - box.top,
        button: type === "move" ? "none" : buttonName(e.button),
        buttons: e.buttons,
        clicks: type === "move" ? 0 : Math.max(1, e.detail),
        mods: modifiers(e),
    };
}

/** Pixels per line and per page when a wheel reports lines or pages. */
const LINE_PX = 16;

export function wheelMessage(e: Point & Modifiers & { deltaX: number; deltaY: number; deltaMode: number }, box: Box, pageHeight: number) {
    const scale = e.deltaMode === 1 ? LINE_PX : e.deltaMode === 2 ? pageHeight : 1;
    return { t: "wheel", x: e.clientX - box.left, y: e.clientY - box.top, dx: e.deltaX * scale, dy: e.deltaY * scale, mods: modifiers(e) };
}

/** What the key types: one character, or Enter as a carriage return; nothing for a chord with Ctrl or Meta. */
export function typedText(e: { key: string; ctrlKey: boolean; metaKey: boolean }): string {
    if (e.ctrlKey || e.metaKey) return "";
    if (e.key === "Enter") return "\r";
    return [...e.key].length === 1 ? e.key : "";
}

export function keyMessage(type: "down" | "up", e: Modifiers & { key: string; code: string; keyCode: number }) {
    return { t: "key", type, key: e.key, code: e.code, text: typedText(e), vk: e.keyCode, mods: modifiers(e) };
}

/** Ctrl or Cmd with V: the browser's own paste event carries the text, so the key itself is not forwarded. */
export function isPasteChord(e: { key: string; ctrlKey: boolean; metaKey: boolean }): boolean {
    return (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "v";
}
