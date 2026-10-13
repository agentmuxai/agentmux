// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Browser pane bookmarks — a global (shared_dir-backed) flat list, not
// per-agent or per-channel. See
// docs/specs/SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md and
// crates/srv/src/server/app_api/bookmarks.rs.

import type { BookmarksResult } from "@/types/rpc/BookmarksResult";
import type { CommandBookmarksListData } from "@/types/rpc/CommandBookmarksListData";
import { RpcClient } from "../rpc-client";

export const BookmarksApi = {
    ListBookmarksCommand(
        client: RpcClient,
        data: CommandBookmarksListData = {},
        opts?: RpcOpts
    ): Promise<BookmarksResult> {
        return client.rpcCall("bookmarks.list", data, opts);
    },
};
