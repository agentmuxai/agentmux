// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { AUTH_KEY_HEADER } from "./sharedconst";

type ComboWebSocket = WebSocket;

/** A browser WebSocket for `url`. It can't send headers, so the auth key goes
 *  in the query string. (A Node `ws` branch for running outside a window was
 *  never reached in the app and is gone.) */
function newWebSocket(url: string, headers: { [key: string]: string }): ComboWebSocket {
    let finalUrl = url;
    if (headers && headers[AUTH_KEY_HEADER]) {
        const separator = url.includes("?") ? "&" : "?";
        finalUrl = `${url}${separator}authkey=${encodeURIComponent(headers[AUTH_KEY_HEADER])}`;
    }
    return new WebSocket(finalUrl);
}

export { newWebSocket };
export type { ComboWebSocket as WebSocket };
