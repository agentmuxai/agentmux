// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Named browser profiles — a shared_dir file like the bookmarks. See
// docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md and
// crates/srv/src/server/app_api/browser_profiles.rs.

import type { BrowserProfilesResult } from "@/types/rpc/BrowserProfilesResult";
import type { CommandBrowserProfilesListData } from "@/types/rpc/CommandBrowserProfilesListData";
import { RpcClient } from "../rpc-client";

export const BrowserProfilesApi = {
    ListBrowserProfilesCommand(
        client: RpcClient,
        data: CommandBrowserProfilesListData = {},
        opts?: RpcOpts
    ): Promise<BrowserProfilesResult> {
        return client.rpcCall("browser_profiles.list", data, opts);
    },
};
