// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Browser pane start page — write-only. The read side is FullConfigType's
// `browserstartpage` field (see config-signals.ts's `browserStartPageAtom`),
// not a matching Get command here — see
// docs/specs/SPEC_BROWSER_PANE_START_PAGE_2026_09_16.md and
// agentmux-srv/src/server/app_api/browser_start_page.rs.

import { RpcClient } from "../rpc-client";

export const BrowserStartPageApi = {
    SetBrowserStartPageCommand(
        client: RpcClient,
        data: { url: string },
        opts?: RpcOpts,
    ): Promise<{ url: string }> {
        return client.rpcCall("browser_start_page.set", data, opts);
    },
};
