// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Everything that doesn't fit a single domain: activity/telemetry, AI messages,
// authentication, suggestions, CPU/test streams, notifications, terminal
// scrollback, widget HTTP proxy, and MuxBus cloud connectivity. Split from the
// original rpc-api file.

import { RpcClient } from "../rpc-client";

// The widget proxy RESULT shapes are GENERATED from Rust by ts-rs. Their
// REQUESTS deliberately stay hand-written: both handlers read their payload
// field-by-field off a `serde_json::Value` and answer a bad port or path with
// an `ok: false` RESULT rather than an RPC error, so there is no Rust request
// struct to generate from -- see `WidgetHealthResult`'s doc comment.
export type { WidgetApiResult } from "@/types/rpc/WidgetApiResult";
export type { WidgetHealthResult } from "@/types/rpc/WidgetHealthResult";

export type { AiRateLimitResult } from "@/types/rpc/AiRateLimitResult";
export type { AppInfoResult } from "@/types/rpc/AppInfoResult";

// The muxbus and providers shapes are GENERATED from their Rust definitions by
// ts-rs. They were private structs in the handler files, so the inline types
// here were hand-maintained against nothing.
//
// The rest of this file stays hand-written on purpose:
//   * `activity` and `recordtevent` have NO backend handler at all (they are
//     in the contract test's KNOWN_LIVE_UNREGISTERED list -- the frontend calls
//     them and they fail at run time). There is nothing to generate from, and
//     wiring or removing them is product work, not a refactor.
//   * `widget.health` / `widget.api` read their fields straight off a
//     `serde_json::Value` with lenient defaults (a missing port becomes 0,
//     which the handler answers with `{healthy:false}` rather than an error).
//     A typed Req would turn those into deserialization failures -- the same
//     reasoning that kept `bundle.validate` untyped.
export type { CatalogModel } from "@/types/rpc/CatalogModel";
export type { MuxBusCloudConfigResp } from "@/types/rpc/MuxBusCloudConfigResp";
export type { MuxBusDeliveryState } from "@/types/rpc/MuxBusDeliveryState";
export type { MuxBusDeliveryStatus } from "@/types/rpc/MuxBusDeliveryStatus";
export type { MuxBusDisconnectResp } from "@/types/rpc/MuxBusDisconnectResp";
export type { MuxBusLoginCancelResp } from "@/types/rpc/MuxBusLoginCancelResp";
export type { MuxBusLoginReq } from "@/types/rpc/MuxBusLoginReq";
export type { MuxBusLoginResp } from "@/types/rpc/MuxBusLoginResp";
export type { MuxBusStatusResp } from "@/types/rpc/MuxBusStatusResp";
export type { ProvidersModelsParams } from "@/types/rpc/ProvidersModelsParams";
export type { ProvidersModelsResult } from "@/types/rpc/ProvidersModelsResult";

export const MiscApi = {
    ActivityCommand(client: RpcClient, data: ActivityUpdate, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("activity", data, opts);
    },

    RecordTEventCommand(client: RpcClient, data: TEvent, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("recordtevent", data, opts);
    },
};
