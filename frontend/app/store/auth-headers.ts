// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The one place an HTTP request to srv gets its `X-AuthKey` header.

import { AUTH_KEY_HEADER } from "@/util/sharedconst";
import { getApi } from "./app-api";

/**
 * Headers that authenticate a request to srv, merged over `extra`.
 *
 * `X-AuthKey` is the key the host gave the UI (`AppApi.getAuthKey`). A host
 * may give none, when a proxy in front of srv adds the header to every
 * request itself; then no `X-AuthKey` is sent rather than an empty one, which
 * srv would refuse. Every HTTP route needs the header: srv accepts the key as
 * a `?authkey=` query parameter on the `/ws` upgrade only (crates/srv/src/server/auth.rs).
 */
export function authHeaders(extra: Record<string, string> = {}): Record<string, string> {
    const key = globalThis.window != null ? getApi()?.getAuthKey?.() : undefined;
    return key ? { ...extra, [AUTH_KEY_HEADER]: key } : { ...extra };
}
