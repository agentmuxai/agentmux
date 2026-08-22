// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Browser pane bookmarks — a global (shared_dir-backed) flat list, not
// per-agent or per-channel. See
// docs/specs/SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md and
// agentmux-srv/src/server/app_api/bookmarks.rs.

import { RpcClient } from "../rpc-client";

export const BookmarksApi = {
    ListBookmarksCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<{ bookmarks: BrowserBookmark[] }> {
        return client.rpcCall("bookmarks.list", data, opts);
    },

    /** Wholesale replace — same shape as `SetConfigCommand`'s
     *  merge-the-whole-value convention, just against the dedicated
     *  bookmarks file instead of settings.json. Callers read-modify-write
     *  the full list (see the nav bar's toggle/add/remove logic). */
    SetBookmarksCommand(
        client: RpcClient,
        data: { bookmarks: BrowserBookmark[] },
        opts?: RpcOpts,
    ): Promise<{ bookmarks: BrowserBookmark[] }> {
        return client.rpcCall("bookmarks.set", data, opts);
    },
};
