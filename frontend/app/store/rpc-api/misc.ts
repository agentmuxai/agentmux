// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Everything that doesn't fit a single domain: activity/telemetry, AI messages,
// authentication, suggestions, CPU/test streams, notifications, terminal
// scrollback, widget HTTP proxy, and MuxBus cloud connectivity. Split from the
// original rpc-api.ts.

import { RpcClient } from "../rpc-client";

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
export type { MuxBusLoginReq } from "@/types/rpc/MuxBusLoginReq";
export type { MuxBusLoginResp } from "@/types/rpc/MuxBusLoginResp";
export type { MuxBusLoginCancelResp } from "@/types/rpc/MuxBusLoginCancelResp";
export type { MuxBusStatusResp } from "@/types/rpc/MuxBusStatusResp";
export type { MuxBusDisconnectResp } from "@/types/rpc/MuxBusDisconnectResp";
export type { ProvidersModelsParams } from "@/types/rpc/ProvidersModelsParams";
export type { ProvidersModelsResult } from "@/types/rpc/ProvidersModelsResult";
export type { CatalogModel } from "@/types/rpc/CatalogModel";

import type { MuxBusLoginReq } from "@/types/rpc/MuxBusLoginReq";
import type { MuxBusLoginResp } from "@/types/rpc/MuxBusLoginResp";
import type { MuxBusLoginCancelReq } from "@/types/rpc/MuxBusLoginCancelReq";
import type { MuxBusLoginCancelResp } from "@/types/rpc/MuxBusLoginCancelResp";
import type { MuxBusStatusReq } from "@/types/rpc/MuxBusStatusReq";
import type { MuxBusStatusResp } from "@/types/rpc/MuxBusStatusResp";
import type { MuxBusDisconnectReq } from "@/types/rpc/MuxBusDisconnectReq";
import type { MuxBusDisconnectResp } from "@/types/rpc/MuxBusDisconnectResp";
import type { ProvidersModelsParams } from "@/types/rpc/ProvidersModelsParams";
import type { ProvidersModelsResult } from "@/types/rpc/ProvidersModelsResult";

export const MiscApi = {
    ActivityCommand(client: RpcClient, data: ActivityUpdate, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("activity", data, opts);
    },

    // command "providers.models" [call] — authoritative model catalog for a
    // provider, fetched server-side from the Anthropic Models API with the
    // account OAuth token. Returns [] (never throws for the model list) when
    // the token is absent/expired; the frontend then keeps its static catalog.
    ProvidersModelsCommand(
        client: RpcClient,
        data: ProvidersModelsParams,
        opts?: RpcOpts,
    ): Promise<ProvidersModelsResult> {
        return client.rpcCall("providers.models", data, opts);
    },

    RecordTEventCommand(client: RpcClient, data: TEvent, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("recordtevent", data, opts);
    },

    MuxInfoCommand(client: RpcClient, opts?: RpcOpts): Promise<MuxInfoData> {
        return client.rpcCall("waveinfo", null, opts);
    },

    // command "widget.health" [call] — HTTP liveness probe for an external widget
    // server on localhost. Returns { healthy, status_code } — never throws on
    // connection failure so the UI can show a "not running" pill gracefully.
    // health_check_body_contains: optional substring the response body must contain;
    // used to distinguish services that share a default port (e.g. Flowise/Grafana
    // both default to 3000).
    WidgetHealthCommand(
        client: RpcClient,
        data: { port: number; health_check_path: string; health_check_body_contains?: string },
        opts?: RpcOpts,
    ): Promise<{ healthy: boolean; status_code: number | null }> {
        return client.rpcCall("widget.health", data, opts);
    },

    // command "widget.api" [call] — HTTP proxy to a widget's local server.
    // Bypasses browser CORS restrictions so agents (and the frontend) can call
    // ComfyUI /prompt, Grafana /api/query, etc. without a CORS header.
    // body must be a pre-serialised JSON string when calling JSON APIs.
    // Never throws. ok:true means the HTTP exchange completed — check status_code
    // for HTTP-level success/failure (4xx/5xx still return ok:true).
    // ok:false means transport failure (connection refused, timeout) or invalid
    // port/path — status_code is null and error is set.
    WidgetApiCommand(
        client: RpcClient,
        data: {
            port: number;
            path: string;
            method?: string;
            headers?: Record<string, string>;
            body?: string;
        },
        opts?: RpcOpts,
    ): Promise<{ ok: boolean; status_code: number | null; body: string | null; error?: string }> {
        return client.rpcCall("widget.api", data, opts);
    },

    // ── MuxBus cloud connectivity ─────────────────────────────────────────────

    // command "muxbus.login" — PKCE browser flow; blocks until login completes (up to 5 min)
    MuxBusLoginCommand(
        client: RpcClient,
        data: MuxBusLoginReq,
        opts?: RpcOpts,
    ): Promise<MuxBusLoginResp> {
        return client.rpcCall("muxbus.login", data, { timeout: 360000, ...opts });
    },

    // command "muxbus.login.cancel" — abort an in-flight muxbus.login (e.g.
    // user closed the browser without completing sign-in). Resolves the
    // pending MuxBusLoginCommand call with a "sign-in cancelled" error.
    MuxBusLoginCancelCommand(
        client: RpcClient,
        opts?: RpcOpts,
    ): Promise<MuxBusLoginCancelResp> {
        const data: MuxBusLoginCancelReq = {};
        return client.rpcCall("muxbus.login.cancel", data, opts);
    },

    // command "muxbus.status" — current credential state
    MuxBusStatusCommand(client: RpcClient, opts?: RpcOpts): Promise<MuxBusStatusResp> {
        const data: MuxBusStatusReq = {};
        return client.rpcCall("muxbus.status", data, opts);
    },

    // command "muxbus.disconnect" — clear stored credentials
    MuxBusDisconnectCommand(client: RpcClient, opts?: RpcOpts): Promise<MuxBusDisconnectResp> {
        const data: MuxBusDisconnectReq = {};
        return client.rpcCall("muxbus.disconnect", data, opts);
    },
};
