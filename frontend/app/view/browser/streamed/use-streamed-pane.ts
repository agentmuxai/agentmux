// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A streamed pane's state and its socket: opened once the stage has a size and is on screen, closed when the pane
// goes or is hidden (a hidden pane streams nothing), reopened at the page it was on when it shows again.
// Sizes are sent as the stage changes, at most every RESIZE_MS. docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §6, §7.

import { createSignal, onCleanup, type Accessor } from "solid-js";
import { joinLease } from "./lease-step";
import { t } from "@/app/i18n/i18n";
import { writeText } from "@/util/clipboard";
import { openStreamedPane, screenModeAsked, type StreamedHandle } from "./streamed-client";
import type { ServerMsg, WindowInfo } from "./streamed-protocol";

/** The smallest and largest side srv accepts. */
const MIN_SIDE = 100;
const MAX_SIDE = 4096;
const RESIZE_MS = 100;

export type PaneStatus = "waiting" | "connecting" | "starting" | "live" | "failed" | "ended" | "paused";
export type NavState = Extract<ServerMsg, { t: "nav" }>;
/** The pane windows and which one is shown. */
export type WindowsState = { active: number; windows: WindowInfo[] };

export function clampSide(n: number): number {
    return Math.min(MAX_SIDE, Math.max(MIN_SIDE, Math.floor(n)));
}

/** An address as typed: `example.com` is `https://example.com`; anything with a scheme is left for srv to judge. */
export function addressToUrl(typed: string): string {
    const text = typed.trim();
    if (!text) return "";
    return /^[a-z][a-z0-9+.-]*:/i.test(text) && !/^[^/]*:\d+(\/|$)/.test(text) ? text : `https://${text}`;
}

export interface StreamedPane {
    status: Accessor<PaneStatus>;
    /** The page is in the full browser the person drives (screen mode), not the streamed one agents can drive. */
    fullBrowser: Accessor<boolean>;
    nav: Accessor<NavState | null>;
    /** The windows of the pane: more than one while a page has opened a window. */
    windows: Accessor<WindowsState | null>;
    error: Accessor<string>;
    /** What the server did on the person's behalf (closed a window, dismissed a message box), until they close it. */
    notice: Accessor<string>;
    send: (message: object) => void;
    /** The stage's size changed (CSS pixels). */
    resized: (w: number, h: number) => void;
    /** The stage is on screen or not. */
    shown: (visible: boolean) => void;
    reconnect: () => void;
    /** Move this page to the full browser the person drives: a new page there, at the address it was on. */
    openFullBrowser: () => void;
    /** The person closed the server's last complaint (a refused address). */
    dismissError: () => void;
    dismissNotice: () => void;
    /** Show one of the windows of the pane. */
    showWindow: (id: number) => void;
    /** Close a window a page opened. */
    closeWindow: (id: number) => void;
}

/** Showing and closing the windows of the pane: a message to the server naming the window's number. */
function windowActions(handle: () => StreamedHandle | null): Pick<StreamedPane, "showWindow" | "closeWindow"> {
    return {
        showWindow: (id) => handle()?.send({ t: "window", op: "focus", id }),
        closeWindow: (id) => handle()?.send({ t: "window", op: "close", id }),
    };
}

/** Where a message from the server lands. */
interface Sinks {
    setNav: (nav: NavState) => void;
    setError: (text: string) => void;
    setWindows: (state: WindowsState) => void;
    setNotice: (text: string) => void;
    setStatus: (status: PaneStatus) => void;
    remember: (url: string) => void;
    /** Put the text the person copied in the remote browser on their own clipboard. */
    putClipboard: (text: string) => void;
}

function applyServerMessage(m: ServerMsg, to: Sinks): void {
    if (m.t === "nav") {
        to.setNav(m);
        to.setError("");
        if (!m.loading && /^https?:/i.test(m.url)) to.remember(m.url);
    } else if (m.t === "windows") to.setWindows({ active: m.active, windows: m.windows });
    else if (m.t === "notice") to.setNotice(m.message);
    else if (m.t === "error") to.setError(m.message);
    else if (m.t === "clipboard") to.putClipboard(m.text);
    else if (m.t === "closed") to.setStatus("ended");
}

/**
 * Put the text the person copied in the remote browser on their own clipboard, through the app's clipboard helper (the desktop host's native
 * clipboard, not the page's, which the embedded browser may refuse); if it cannot, say so.
 */
function copyToClipboard(text: string, setNotice: (text: string) => void): void {
    writeText(text).catch(() => setNotice(t("browser.streamed.copyFailed")));
}

/** The message that tells the server the stage's size. */
function resizeMessage(size: { w: number; h: number }): object {
    return { t: "resize", w: clampSide(size.w), h: clampSide(size.h) };
}

/**
 * The socket ended. A server that would not hand out a browser says why (it stays readable next to Reconnect). A server without the full
 * browser answers "not found" to the request for it: the pane goes back to the ordinary browser and says so in words.
 */
function applyEnd(how: "failed" | "closed", reason: string | undefined, full: boolean, to: Pick<Sinks, "setStatus" | "setError"> & { setFull: (on: boolean) => void }): void {
    if (how === "failed" && full) {
        to.setFull(false);
        to.setStatus("failed");
        to.setError(t("browser.streamed.fullBrowserOff"));
        return;
    }
    to.setStatus(how === "failed" ? "failed" : "ended");
    if (reason) to.setError(reason);
}

/** The pane's state: what the page is doing, what the server said, and whether the page is in the full browser the person drives. */
function paneSignals() {
    const [status, setStatus] = createSignal<PaneStatus>("waiting");
    const [nav, setNav] = createSignal<NavState | null>(null);
    const [error, setError] = createSignal("");
    const [windows, setWindows] = createSignal<WindowsState | null>(null);
    const [notice, setNotice] = createSignal("");
    const [full, setFull] = createSignal(screenModeAsked());
    return { status, setStatus, nav, setNav, error, setError, windows, setWindows, notice, setNotice, full, setFull };
}

/** `draw` paints a frame; `startUrl` is where a new socket starts; `remember` keeps the page's address. */
export function useStreamedPane(draw: (jpeg: Uint8Array) => void, startUrl: () => string, remember: (url: string) => void): StreamedPane {
    const { status, setStatus, nav, setNav, error, setError, windows, setWindows, notice, setNotice, full, setFull } = paneSignals();
    let handle: StreamedHandle | null = null;
    const leave = joinLease();
    let size = { w: 0, h: 0 };
    let visible = false;
    /** An address asked for while the socket was still opening: kept, and sent as soon as it opens. */
    let pendingNav: object | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const close = () => {
        handle?.close();
        handle = null;
    };
    const open = () => {
        if (handle || !visible || size.w < MIN_SIDE || size.h < MIN_SIDE) return;
        setStatus("connecting");
        setError("");
        setNotice("");
        setWindows(null); // a new socket is a new browser page: its windows start over
        const at = nav()?.url || startUrl();
        const started = { w: clampSide(size.w), h: clampSide(size.h) };
        handle = openStreamedPane(
            { url: at && at !== "about:blank" ? at : "", ...started, screen: full() },
            {
                onFrame: draw,
                onOpen: () => {
                    setStatus("live");
                    if (pendingNav) {
                        handle?.send(pendingNav);
                        pendingNav = null;
                    }
                    // The pane may have been resized while the socket was opening: the server has the size it started with.
                    if (clampSide(size.w) !== started.w || clampSide(size.h) !== started.h) handle?.send(resizeMessage(size));
                },
                onMessage: (m) => applyServerMessage(m, { setNav, setError, setWindows, setNotice, setStatus, remember, putClipboard: (text) => copyToClipboard(text, setNotice) }),
                // The server's browser is being made for this pane: its own status, until the socket opens.
                onStarting: () => setStatus("starting"),
                onEnd: (how, reason) => {
                    handle = null;
                    applyEnd(how, reason, full(), { setFull, setStatus, setError });
                },
            }
        );
    };

    const reconnect = () => {
        close();
        open();
    };

    onCleanup(() => {
        clearTimeout(timer);
        close();
        // The pane is gone (not just hidden): when it is the page's last, a server that handed out a browser takes it back.
        leave();
    });

    return {
        status,
        fullBrowser: full,
        nav,
        windows,
        error,
        notice,
        send: (message) => {
            if (status() === "live") handle?.send(message);
            // An address typed while connecting is not lost; input and other moves are not for a page that is not there yet.
            else if ((message as { t?: string; go?: string }).t === "nav" && (message as { go?: string }).go === "url") pendingNav = message;
        },
        resized: (w, h) => {
            size = { w, h };
            // A throttle, not a debounce: during a drag the newest size goes out every RESIZE_MS, and once more when it settles.
            if (timer !== undefined) return;
            timer = setTimeout(() => {
                timer = undefined;
                if (handle) handle.send(resizeMessage(size));
                // A pane that has not opened yet (or was shown too small to open) opens here; a failed or ended one waits for Reconnect.
                else if (status() === "waiting" || status() === "paused") open();
            }, RESIZE_MS);
        },
        shown: (v) => {
            visible = v;
            if (v) open();
            else if (handle) {
                close();
                setStatus("paused");
            }
        },
        reconnect,
        openFullBrowser: () => {
            setFull(true);
            reconnect();
        },
        dismissError: () => setError(""),
        dismissNotice: () => setNotice(""),
        ...windowActions(() => handle),
    };
}
