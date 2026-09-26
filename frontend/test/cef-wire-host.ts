// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// For tests that assert on the wire: install a host whose browser-pane
// commands, focus reclaim and event listening are the real CEF
// implementations, so a test's `vi.mock("@/app/platform/ipc", ...)` sees the
// same command names and payloads the app sends.
// docs/specs/SPEC_HOST_API_SEAM_2026_09_26.md

import { cefBrowserPanes, cefReclaimWindowFocus } from "@/app/host/cef-host-commands";
import { CEF_HOST_CAPS } from "@/app/host/host-caps";
import { makeTestHostApi } from "@/app/host/test-host";
import { listenEvent } from "@/app/platform/ipc";

export function installCefWireHost(overrides: Partial<AppApi> = {}): AppApi {
    const api = makeTestHostApi(
        {
            browserPanes: cefBrowserPanes,
            reclaimWindowFocus: cefReclaimWindowFocus,
            listen: (event, callback) => listenEvent(event, callback),
            ...overrides,
        },
        CEF_HOST_CAPS,
    );
    window.api = api;
    return api;
}
