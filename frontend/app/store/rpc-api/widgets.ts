// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Widget packages (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8).
// Their types are generated from crates/srv/src/backend/widget_packages.rs and
// crates/srv/src/server/widget_handlers.rs. Approving a widget isn't here: only
// the host can carry that answer (AppApi `approvals.decideWidget`).

export type { SignatureState } from "@/types/rpc/SignatureState";
export type { StatusAlignment } from "@/types/rpc/StatusAlignment";
export type { WidgetCommandInfo } from "@/types/rpc/WidgetCommandInfo";
export type { WidgetInstallRequest } from "@/types/rpc/WidgetInstallRequest";
export type { WidgetKind } from "@/types/rpc/WidgetKind";
export type { WidgetPackageInfo } from "@/types/rpc/WidgetPackageInfo";
export type { WidgetPaneInfo } from "@/types/rpc/WidgetPaneInfo";
export type { WidgetPublisherPin } from "@/types/rpc/WidgetPublisherPin";
export type { WidgetSignatureInfo } from "@/types/rpc/WidgetSignatureInfo";
export type { WidgetState } from "@/types/rpc/WidgetState";
export type { WidgetStatusItemInfo } from "@/types/rpc/WidgetStatusItemInfo";

export type { WidgetCatalogEntry } from "@/types/rpc/WidgetCatalogEntry";
export type { WidgetCatalogItem } from "@/types/rpc/WidgetCatalogItem";
export type { WidgetCatalogResult } from "@/types/rpc/WidgetCatalogResult";

export const WidgetsApi = {};
