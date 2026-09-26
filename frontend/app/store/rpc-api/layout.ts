// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Layout files — save a window as `*.agentmux-layout.json`, and open one
// back as new tabs. See docs/specs/SPEC_LAYOUT_FILES_2026_09_25.md and
// agentmux-srv/src/server/app_api/layout.rs.

import { RpcClient } from "../rpc-client";
import type { CommandLayoutOpenData } from "@/types/rpc/CommandLayoutOpenData";
import type { CommandLayoutPreviewData } from "@/types/rpc/CommandLayoutPreviewData";
import type { CommandLayoutSaveData } from "@/types/rpc/CommandLayoutSaveData";
import type { LayoutOpenResult } from "@/types/rpc/LayoutOpenResult";
import type { LayoutPreviewResult } from "@/types/rpc/LayoutPreviewResult";
import type { LayoutSaveResult } from "@/types/rpc/LayoutSaveResult";

export const LayoutApi = {
    /** Export `window_id`'s tabs and write them to `path` (absolute,
     *  `*.agentmux-layout.json`, normally from `showSaveLayoutDialog`). */
    SaveLayoutCommand(client: RpcClient, data: CommandLayoutSaveData, opts?: RpcOpts): Promise<LayoutSaveResult> {
        return client.rpcCall("layout.save", data, opts);
    },

    /** Describe what opening `path` would do — tabs, panes, commands, what's
     *  missing here — without opening anything. */
    PreviewLayoutCommand(
        client: RpcClient,
        data: CommandLayoutPreviewData,
        opts?: RpcOpts,
    ): Promise<LayoutPreviewResult> {
        return client.rpcCall("layout.preview", data, opts);
    },

    /** Add `path`'s tabs to `window_id`. Nothing already open is replaced. */
    OpenLayoutCommand(client: RpcClient, data: CommandLayoutOpenData, opts?: RpcOpts): Promise<LayoutOpenResult> {
        return client.rpcCall("layout.open", data, opts);
    },
};
