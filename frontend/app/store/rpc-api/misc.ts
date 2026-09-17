// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Everything that doesn't fit a single domain: activity/telemetry, AI messages,
// authentication, suggestions, CPU/test streams, notifications, terminal
// scrollback, widget HTTP proxy, and MuxBus cloud connectivity. Split from the
// original rpc-api.ts.

import { RpcClient } from "../rpc-client";

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
        data: { provider_id: string },
        opts?: RpcOpts,
    ): Promise<{ models: Array<{ id: string; display_name: string }> }> {
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
        data: { cognitoDomain: string; clientId: string },
        opts?: RpcOpts,
    ): Promise<{ success: boolean; email: string; error?: string }> {
        return client.rpcCall("muxbus.login", data, { timeout: 360000, ...opts });
    },

    // command "muxbus.login.cancel" — abort an in-flight muxbus.login (e.g.
    // user closed the browser without completing sign-in). Resolves the
    // pending MuxBusLoginCommand call with a "sign-in cancelled" error.
    MuxBusLoginCancelCommand(
        client: RpcClient,
        opts?: RpcOpts,
    ): Promise<{ cancelled: boolean }> {
        return client.rpcCall("muxbus.login.cancel", {}, opts);
    },

    // command "muxbus.status" — current credential state
    MuxBusStatusCommand(
        client: RpcClient,
        opts?: RpcOpts,
    ): Promise<{
        connected: boolean;
        email: string;
        cognitoDomain: string;
        expiresAt: number;
        valid: boolean;
    }> {
        return client.rpcCall("muxbus.status", {}, opts);
    },

    // command "muxbus.disconnect" — clear stored credentials
    MuxBusDisconnectCommand(client: RpcClient, opts?: RpcOpts): Promise<Record<string, never>> {
        return client.rpcCall("muxbus.disconnect", {}, opts);
    },
};
