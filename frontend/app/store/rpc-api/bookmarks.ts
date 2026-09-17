// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Browser pane bookmarks — a global (shared_dir-backed) flat list, not
// per-agent or per-channel. See
// docs/specs/SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md and
// agentmux-srv/src/server/app_api/bookmarks.rs.

import { RpcClient } from "../rpc-client";
import type { BookmarksResult } from "@/types/rpc/BookmarksResult";
import type { CommandBookmarksListData } from "@/types/rpc/CommandBookmarksListData";
import type { CommandBookmarksSetData } from "@/types/rpc/CommandBookmarksSetData";

export const BookmarksApi = {
    ListBookmarksCommand(
        client: RpcClient,
        data: CommandBookmarksListData = {},
        opts?: RpcOpts,
    ): Promise<BookmarksResult> {
        return client.rpcCall("bookmarks.list", data, opts);
    },

    /** Wholesale replace — same shape as `SetConfigCommand`'s
     *  merge-the-whole-value convention, just against the dedicated
     *  bookmarks file instead of settings.json. Callers read-modify-write
     *  the full list (see the nav bar's toggle/add/remove logic). */
    SetBookmarksCommand(
        client: RpcClient,
        data: CommandBookmarksSetData,
        opts?: RpcOpts,
    ): Promise<BookmarksResult> {
        return client.rpcCall("bookmarks.set", data, opts);
    },
};
