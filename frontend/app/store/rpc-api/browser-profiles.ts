// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Named browser profiles — a shared_dir file like the bookmarks. See
// docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md and
// crates/srv/src/server/app_api/browser_profiles.rs.

import { RpcClient } from "../rpc-client";
import type { BrowserProfilesResult } from "@/types/rpc/BrowserProfilesResult";
import type { CommandBrowserProfileCreateData } from "@/types/rpc/CommandBrowserProfileCreateData";
import type { CommandBrowserProfileDeleteData } from "@/types/rpc/CommandBrowserProfileDeleteData";
import type { CommandBrowserProfilesListData } from "@/types/rpc/CommandBrowserProfilesListData";
import type { CommandBrowserProfileUpdateData } from "@/types/rpc/CommandBrowserProfileUpdateData";

export const BrowserProfilesApi = {
    ListBrowserProfilesCommand(
        client: RpcClient,
        data: CommandBrowserProfilesListData = {},
        opts?: RpcOpts,
    ): Promise<BrowserProfilesResult> {
        return client.rpcCall("browser_profiles.list", data, opts);
    },

    CreateBrowserProfileCommand(
        client: RpcClient,
        data: CommandBrowserProfileCreateData,
        opts?: RpcOpts,
    ): Promise<BrowserProfilesResult> {
        return client.rpcCall("browser_profiles.create", data, opts);
    },

    UpdateBrowserProfileCommand(
        client: RpcClient,
        data: CommandBrowserProfileUpdateData,
        opts?: RpcOpts,
    ): Promise<BrowserProfilesResult> {
        return client.rpcCall("browser_profiles.update", data, opts);
    },

    /** Closes the profile's open tabs and deletes its saved data. */
    DeleteBrowserProfileCommand(
        client: RpcClient,
        data: CommandBrowserProfileDeleteData,
        opts?: RpcOpts,
    ): Promise<BrowserProfilesResult> {
        return client.rpcCall("browser_profiles.delete", data, opts);
    },
};
