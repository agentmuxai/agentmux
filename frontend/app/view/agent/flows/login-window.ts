// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window a provider login opens in, reserved at the user's click.
 *
 * A login's URL arrives seconds after the click: srv starts the CLI and reads
 * the URL from its output. A browser opens a new window only during a user
 * gesture, so a host whose "open externally" is `window.open` (a browser
 * host) must open the window at the click and point it at the URL later.
 *
 * Each reservation has an owner, the flow the click started, which releases
 * it on every way out (`withLoginWindow`, or the release function
 * `reserveLoginWindow` returns). Releasing closes the window only if no URL
 * used it, and only that owner's: a newer login's reservation is never
 * closed by an older flow ending. A timer closes any reservation still unused
 * after `MAX_AGE_MS` (from the click), so nothing is left blank for good.
 *
 * `openOAuthBrowserPane` (and the launch modal's URL effect) navigate the
 * reserved window when the URL arrives. A host that opens URLs in the system
 * browser (the desktop) reserves nothing (`reserveExternalWindow()` returns
 * null), and every caller keeps its usual path. One slot: a login reserves
 * anew, closing an unused one.
 */
import { createEffect, onCleanup } from "solid-js";

import { getApi } from "@/app/store/app-api";

/**
 * How long, from the click, a reservation waits for its URL. Long enough for
 * the slowest step before a login prints its URL (installing the CLI first).
 */
const MAX_AGE_MS = 10 * 60_000;

let reserved: { win: ExternalWindow; id: number; timer: ReturnType<typeof setTimeout> } | null = null;
let nextId = 1;
const noop = () => {};

/**
 * Reserve a window for a login starting now, at a click; call it before the
 * click handler's first `await`. Returns the release for this reservation.
 * Never throws.
 */
export function reserveLoginWindow(): () => void {
    clearSlot(true);
    let win: ExternalWindow | null = null;
    try {
        win = getApi().reserveExternalWindow();
    } catch (e) {
        console.warn(`[login-window] reserve failed: ${(e as Error)?.message ?? String(e)}`);
    }
    if (!win) return noop;
    const id = nextId++;
    const timer = setTimeout(() => release(id), MAX_AGE_MS);
    reserved = { win, id, timer };
    return () => release(id);
}

/**
 * Runs a login flow started by a click: reserves its window first (for a
 * provider whose login prints a URL), and releases it however the flow ends.
 */
export function withLoginWindow<T>(
    provider: { headlessLoginUrlUnsupported?: boolean } | null | undefined,
    flow: () => Promise<T>,
): Promise<T> {
    const release = provider && !provider.headlessLoginUrlUnsupported ? reserveLoginWindow() : noop;
    let running: Promise<T>;
    try {
        running = flow();
    } catch (e) {
        release();
        throw e;
    }
    return running.finally(release);
}

/**
 * For a login whose URL can arrive after its start call settles (the launch
 * modal's Connect: the controller polls for the URL). `begin(provider)` runs
 * in the click: it reserves the window and returns the callback for when the
 * start call settles. The window is released once that connect is over: when
 * the start call settles with nothing `waiting`, or when the state leaves
 * `waiting` after having been there for this connect, or on cleanup. Call it
 * in a component (it uses an effect).
 */
export function createConnectWindow(kind: () => string): {
    begin(provider: { headlessLoginUrlUnsupported?: boolean }): () => void;
} {
    let current: { release: () => void; sawWaiting: boolean } | null = null;
    const done = (c: typeof current) => {
        if (!c) return;
        c.release();
        if (current === c) current = null;
    };
    createEffect(() => {
        // Read first, so the effect tracks the state from its first run
        // (`current` is a plain variable, which doesn't make it re-run).
        const k = kind();
        const c = current;
        if (!c) return;
        if (k === "waiting") c.sawWaiting = true;
        else if (c.sawWaiting) done(c);
    });
    onCleanup(() => done(current));
    return {
        begin(provider) {
            done(current);
            const c = { release: provider.headlessLoginUrlUnsupported ? noop : reserveLoginWindow(), sawWaiting: false };
            current = c;
            return () => {
                if (kind() !== "waiting") done(c);
            };
        },
    };
}

/** The reserved window, handed over once; null when there's none. */
export function takeLoginWindow(): ExternalWindow | null {
    const r = reserved;
    if (!r) return null;
    clearTimeout(r.timer);
    reserved = null;
    return r.win;
}

/** Shows `url` in the reserved window, if there is one; whether it did. */
export function navigateLoginWindow(url: string): boolean {
    const win = takeLoginWindow();
    if (!win) return false;
    try {
        win.navigate(url);
        return true;
    } catch (e) {
        console.warn(`[login-window] navigate failed: ${(e as Error)?.message ?? String(e)}`);
        closeQuietly(win);
        return false;
    }
}

/** Closes reservation `id` if it's still the unused one. */
function release(id: number): void {
    if (reserved?.id === id) clearSlot(true);
}

function clearSlot(close: boolean): void {
    const r = reserved;
    reserved = null;
    if (!r) return;
    clearTimeout(r.timer);
    if (close) closeQuietly(r.win);
}

function closeQuietly(win: ExternalWindow): void {
    try {
        win.close();
    } catch {
        // Already closed by the user: nothing to do.
    }
}
