// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getEnv } from "./getenv";

const WebServerEndpointVarName = "WAVE_SERVER_WEB_ENDPOINT";
const WSServerEndpointVarName = "WAVE_SERVER_WS_ENDPOINT";

/**
 * An endpoint is either a bare `host:port`, given the scheme here (the desktop
 * host's loopback srv: `http`, `ws`), or a full origin such as
 * `https://app.example.com` / `wss://app.example.com`, used as it is (a UI
 * served through a proxy, on whatever scheme the page has).
 */
function endpointUrl(value: string | null | undefined, scheme: "http" | "ws"): string {
    if (value != null && value.includes("://")) return value.replace(/\/+$/, "");
    return `${scheme}://${value}`;
}

// Not memoized: endpoints are set asynchronously after module load (by the CEF bootstrap),
// so lazy() would cache "http://null" if called too early.
export const getWebServerEndpoint = () => endpointUrl(getEnv(WebServerEndpointVarName), "http");

/** False until the CEF bootstrap has set the backend address — before that,
 *  getWebServerEndpoint() returns "http://null". */
export const isWebServerEndpointSet = () => !!getEnv(WebServerEndpointVarName);

export const getWSServerEndpoint = () => endpointUrl(getEnv(WSServerEndpointVarName), "ws");
