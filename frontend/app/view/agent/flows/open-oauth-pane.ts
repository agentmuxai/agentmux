// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Open a provider login / OAuth URL for an agent re-auth.
 *
 * SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20: the user asked for "a browser window
 * + url as backup". The idiomatic, in-app "browser window" in AgentMux is a
 * **browser pane** (`createBlock({ meta: { view: "browser", url } })`), which
 * splits the current tab and renders a real CEF browser. A modal-embedded CEF
 * browser is architecturally impossible (the pane needs a native HWND child
 * window that can't be CSS-positioned inside a modal overlay).
 *
 * We split (not magnify) so the agent pane's AuthUrlBox — the URL text + the
 * paste-the-code input — stays visible beside the browser. If the in-app pane
 * can't be created (no layout model, RPC failure) we fall back to the system
 * browser via `openExternal`. The AuthUrlBox is the URL backup in either case.
 *
 * Never throws: login UX must degrade gracefully, never crash the launch flow.
 */

import { createBlock, getApi } from "@/app/store/global";

export type OAuthOpenResult = "pane" | "external" | "failed";

export async function openOAuthBrowserPane(url: string): Promise<OAuthOpenResult> {
    try {
        // In-app browser pane — the primary "browser window". Split beside the
        // agent pane (magnified=false) so the AuthUrlBox paste-code input stays
        // reachable for providers that hand back a code instead of redirecting.
        await createBlock({ meta: { view: "browser", url } });
        return "pane";
    } catch {
        // Pane creation failed — fall back to the system browser. openExternal
        // is fire-and-forget and swallows its own errors, so this won't throw.
        try {
            getApi().openExternal(url);
            return "external";
        } catch {
            return "failed";
        }
    }
}
