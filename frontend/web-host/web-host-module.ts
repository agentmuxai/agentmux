// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The browser host's startup hooks: the `@host-module` of the web build
// (scripts/build-web.mjs, through AGENTMUX_HOST_MODULE; the seam is
// docs/specs/SPEC_EXTERNAL_HOST_BUILD_2026_10_09.md). srv is reached on the
// page's own origin, through a proxy that adds its key.

import type { HostModule } from "@/app/host/host-module";
import { requireSignIn } from "./not-signed-in";
import { streamedBrowserAvailable } from "./streamed-browser-probe";
import { buildWebApi } from "./web-api";

/** srv's HTTP and WebSocket endpoints for a page served from `origin`. */
export function endpointsFor(origin: string): { web: string; ws: string } {
    return { web: origin, ws: origin.replace(/^http/, "ws") };
}

export const hostModule: HostModule = {
    early() {
        // The browser's own console is the log; nothing to set up.
    },
    firstPaint() {
        // No native window waiting to be shown.
    },
    async setup() {
        const { web, ws } = endpointsFor(window.location.origin);
        window.__WAVE_SERVER_WEB_ENDPOINT__ = web;
        window.__WAVE_SERVER_WS_ENDPOINT__ = ws;
        const api = buildWebApi({ streamedBrowserPane: await streamedBrowserAvailable(window.sessionStorage.getItem("amx-session") ?? undefined) });
        // A browser that was never signed in gets one plain card, not a reconnect loop that ends in "Press Restore".
        // `window.api` is installed only after the check: the app's own start (app-init.ts waits for `window.api`)
        // must not run beside it, or its failure would put the recovery card over this one.
        await requireSignIn(api.getSessionToken?.());
        window.api = api;
    },
};
