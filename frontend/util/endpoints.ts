// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { getEnv } from "./getenv";

export const WebServerEndpointVarName = "WAVE_SERVER_WEB_ENDPOINT";
export const WSServerEndpointVarName = "WAVE_SERVER_WS_ENDPOINT";

// Not memoized: endpoints are set asynchronously after module load (by setupTauriApi),
// so lazy() would cache "http://null" if called too early.
export const getWebServerEndpoint = () => `http://${getEnv(WebServerEndpointVarName)}`;

export const getWSServerEndpoint = () => `ws://${getEnv(WSServerEndpointVarName)}`;
