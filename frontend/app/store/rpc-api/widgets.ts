// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Widget packages (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8).
// Their types are generated from crates/srv/src/backend/widget_packages.rs and
// crates/srv/src/server/widget_handlers.rs. Approving a widget isn't here: only
// the host can carry that answer (AppApi `approvals.decideWidget`).

import { RpcClient } from "../rpc-client";
import type { WidgetInstallResult } from "@/types/rpc/WidgetInstallResult";
import type { WidgetPackagesResult } from "@/types/rpc/WidgetPackagesResult";
import type { WidgetReadFileResult } from "@/types/rpc/WidgetReadFileResult";

export type { WidgetKind } from "@/types/rpc/WidgetKind";
export type { WidgetPackageInfo } from "@/types/rpc/WidgetPackageInfo";
export type { WidgetPaneInfo } from "@/types/rpc/WidgetPaneInfo";
export type { WidgetState } from "@/types/rpc/WidgetState";

export const WidgetsApi = {
    /** Every installed widget package and its state. */
    WidgetsListCommand(client: RpcClient, opts?: RpcOpts): Promise<WidgetPackagesResult> {
        return client.rpcCall("widgets.list", {}, opts);
    },

    /** Scan the widgets folder again (the folder watcher normally does). */
    WidgetsRescanCommand(client: RpcClient, opts?: RpcOpts): Promise<WidgetPackagesResult> {
        return client.rpcCall("widgets.rescan", {}, opts);
    },

    /** Turn an approved widget off or on. */
    WidgetsSetEnabledCommand(client: RpcClient, data: { id: string; enabled: boolean }, opts?: RpcOpts): Promise<WidgetPackagesResult> {
        return client.rpcCall("widgets.setenabled", data, opts);
    },

    /** Copy a package (a folder, its widget.json, or a .zip) into the widgets
     *  folder. It doesn't run until the user approves it. */
    WidgetsInstallCommand(client: RpcClient, data: { path: string; replace?: boolean }, opts?: RpcOpts): Promise<WidgetInstallResult> {
        return client.rpcCall("widgets.install", data, opts);
    },

    /** Delete a package and its approval. */
    WidgetsUninstallCommand(client: RpcClient, data: { id: string }, opts?: RpcOpts): Promise<WidgetPackagesResult> {
        return client.rpcCall("widgets.uninstall", data, opts);
    },

    /** A trusted widget's module, as approved. */
    WidgetsReadFileCommand(client: RpcClient, data: { id: string; hash: string; path: string }, opts?: RpcOpts): Promise<WidgetReadFileResult> {
        return client.rpcCall("widgets.readfile", data, opts);
    },
};
