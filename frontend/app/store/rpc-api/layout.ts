// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Layout files — save a window as `*.agentmux-layout.json`. See
// docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md and
// agentmux-srv/src/server/app_api/layout.rs.

import { RpcClient } from "../rpc-client";
import type { CommandLayoutSaveData } from "@/types/rpc/CommandLayoutSaveData";
import type { LayoutSaveResult } from "@/types/rpc/LayoutSaveResult";

export const LayoutApi = {
    /** Export `window_id`'s tabs and write them to `path` (absolute,
     *  `*.agentmux-layout.json`, normally from `showSaveLayoutDialog`). */
    SaveLayoutCommand(client: RpcClient, data: CommandLayoutSaveData, opts?: RpcOpts): Promise<LayoutSaveResult> {
        return client.rpcCall("layout.save", data, opts);
    },
};
