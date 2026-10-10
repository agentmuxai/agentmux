// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window a provider login opens in, reserved at the user's click.
 *
 * A login's URL arrives seconds after the click: srv starts the CLI and reads
 * the URL from its output. A browser opens a new window only during a user
 * gesture, so a host whose "open externally" is `window.open` (a browser
 * host) must open the window at the click and point it at the URL later.
 * `reserveLoginWindow()` runs at the click, before the handler's first
 * `await`; `openOAuthBrowserPane` (and the launch modal's URL effect) navigate
 * the reserved window when the URL arrives; `runProviderLogin` releases
 * (closes) it if the flow ended without using it.
 *
 * A host that opens URLs in the system browser (the desktop) reserves
 * nothing (`reserveExternalWindow()` returns null), and every caller keeps
 * its usual path. One slot: a login reserves anew, closing an unused one.
 */
import { getApi } from "@/app/store/app-api";

/** A reservation older than this is closed rather than used: its login is long gone. */
const MAX_AGE_MS = 5 * 60_000;

let reserved: { win: ExternalWindow; at: number } | null = null;

/** Reserve a window for a login starting now, at a click. Never throws. */
export function reserveLoginWindow(): void {
    releaseLoginWindow();
    try {
        const win = getApi().reserveExternalWindow();
        if (win) reserved = { win, at: Date.now() };
    } catch (e) {
        console.warn(`[login-window] reserve failed: ${(e as Error)?.message ?? String(e)}`);
    }
}

/** The reserved window, handed over once; null when there's none (or it's stale). */
export function takeLoginWindow(): ExternalWindow | null {
    const r = reserved;
    reserved = null;
    if (!r) return null;
    if (Date.now() - r.at > MAX_AGE_MS) {
        closeQuietly(r.win);
        return null;
    }
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

/** Closes the reserved window if nothing used it. */
export function releaseLoginWindow(): void {
    const r = reserved;
    reserved = null;
    if (r) closeQuietly(r.win);
}

function closeQuietly(win: ExternalWindow): void {
    try {
        win.close();
    } catch {
        // Already closed by the user: nothing to do.
    }
}
