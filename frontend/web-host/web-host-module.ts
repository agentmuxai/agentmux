// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The browser host's startup hooks: the `@host-module` of the web build
// (scripts/build-web.mjs, through AGENTMUX_HOST_MODULE; the seam is
// docs/specs/SPEC_EXTERNAL_HOST_BUILD_2026_10_09.md). srv is reached on the
// page's own origin, through a proxy that adds its key.

import type { HostModule } from "@/app/host/host-module";
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
        window.api = buildWebApi();
    },
};
