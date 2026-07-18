// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Open a provider login / OAuth URL for an agent re-auth.
 *
 * The system's default browser is the primary target: it already carries the
 * user's existing session/cookies for most providers, so OAuth there is far
 * more likely to auto-complete than in a fresh, cookie-less in-app pane
 * (retro-agent-login-browser-2026-07-18 — the in-app pane was also fighting a
 * separate rendering issue, but reusing the logged-in system browser is the
 * better outcome either way). If the system browser can't be opened, we fall
 * back to an in-app **browser pane** (`createBlock({ meta: { view: "browser",
 * url } })`), which splits the current tab and renders a real CEF browser. The
 * AuthUrlBox above the composer stays visible as a URL backup in both cases.
 *
 * Never throws: login UX must degrade gracefully, never crash the launch flow.
 */

import { createBlock } from "@/app/store/global";
import { invokeCommand } from "@/app/platform/ipc";

export type OAuthOpenResult = "pane" | "external" | "failed";

export async function openOAuthBrowserPane(url: string): Promise<OAuthOpenResult> {
    try {
        // Awaited (unlike getApi().openExternal's fire-and-forget form) so a
        // real failure — no default browser handler, disallowed scheme, spawn
        // error — falls through to the in-app pane instead of silently
        // reporting "external" with nothing having opened.
        await invokeCommand("open_external", { url });
        return "external";
    } catch {
        try {
            await createBlock({ meta: { view: "browser", url } });
            return "pane";
        } catch {
            return "failed";
        }
    }
}
