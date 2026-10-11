// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Widget packages (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8).
// Their types are generated from crates/srv/src/backend/widget_packages.rs and
// crates/srv/src/server/widget_handlers.rs. Approving a widget isn't here: only
// the host can carry that answer (AppApi `approvals.decideWidget`).

import { RpcClient } from "../rpc-client";
import type { WidgetCatalogResult } from "@/types/rpc/WidgetCatalogResult";
import type { WidgetInstallResult } from "@/types/rpc/WidgetInstallResult";
import type { WidgetCallResult } from "@/types/rpc/WidgetCallResult";
import type { WidgetPackagesResult } from "@/types/rpc/WidgetPackagesResult";
import type { WidgetReadFileResult } from "@/types/rpc/WidgetReadFileResult";
import type { WidgetRequestsResult } from "@/types/rpc/WidgetRequestsResult";
import type { WidgetSessionResult } from "@/types/rpc/WidgetSessionResult";

export type { WidgetInstallRequest } from "@/types/rpc/WidgetInstallRequest";
export type { StatusAlignment } from "@/types/rpc/StatusAlignment";
export type { WidgetCommandInfo } from "@/types/rpc/WidgetCommandInfo";
export type { WidgetKind } from "@/types/rpc/WidgetKind";
export type { WidgetPackageInfo } from "@/types/rpc/WidgetPackageInfo";
export type { WidgetPaneInfo } from "@/types/rpc/WidgetPaneInfo";
export type { WidgetState } from "@/types/rpc/WidgetState";
export type { SignatureState } from "@/types/rpc/SignatureState";
export type { WidgetPublisherPin } from "@/types/rpc/WidgetPublisherPin";
export type { WidgetSignatureInfo } from "@/types/rpc/WidgetSignatureInfo";
export type { WidgetStatusItemInfo } from "@/types/rpc/WidgetStatusItemInfo";

export type { WidgetCatalogEntry } from "@/types/rpc/WidgetCatalogEntry";
export type { WidgetCatalogItem } from "@/types/rpc/WidgetCatalogItem";
export type { WidgetCatalogResult } from "@/types/rpc/WidgetCatalogResult";

export const WidgetsApi = {
    /** The widget catalog, checked, with what is installed here
     *  (SPEC_WIDGET_SHARING_2026_10_10.md §4.5). */
    WidgetsCatalogCommand(client: RpcClient, opts?: RpcOpts): Promise<WidgetCatalogResult> {
        return client.rpcCall("widgets.catalog", {}, opts);
    },
    /** Download, check and copy in a catalog widget; it then waits for approval. */
    WidgetsCatalogInstallCommand(client: RpcClient, data: { id: string }, opts?: RpcOpts): Promise<WidgetInstallResult> {
        return client.rpcCall("widgets.catalog.install", data, opts);
    },
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

    /** A session for one pane of a sandboxed widget, for its calls through
     *  srv (storage, net, agents): srv checks each against the package. */
    WidgetsSessionCommand(client: RpcClient, data: { id: string; hash: string; blockid: string }, opts?: RpcOpts): Promise<WidgetSessionResult> {
        return client.rpcCall("widgets.session", data, opts);
    },

    WidgetsEndSessionCommand(client: RpcClient, data: { token: string }, opts?: RpcOpts): Promise<unknown> {
        return client.rpcCall("widgets.endsession", data, opts);
    },

    /** Agents' requests to install a widget, waiting for the user. */
    WidgetsRequestsCommand(client: RpcClient, opts?: RpcOpts): Promise<WidgetRequestsResult> {
        return client.rpcCall("widgets.requests", {}, opts);
    },

    /** One bridge method srv answers. A refusal's message is `widget-error:`
     *  and the bridge error as JSON. */
    WidgetsCallCommand(client: RpcClient, data: { token: string; method: string; params: unknown }, opts?: RpcOpts): Promise<WidgetCallResult> {
        return client.rpcCall("widgets.call", data, opts);
    },
};
