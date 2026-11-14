// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { authHeaders } from "@/app/store/auth-headers";
import { getWebServerEndpoint } from "@/util/endpoints";

/** The list srv's `route` answers under `key`, or nothing when srv is older than the route or cannot be reached. */
export async function srvList<T>(route: string, key: string): Promise<T[]> {
    try {
        const res = await fetch(`${getWebServerEndpoint()}${route}`, { headers: authHeaders() });
        if (!res.ok) return [];
        const list = ((await res.json()) as Record<string, unknown>)[key];
        return Array.isArray(list) ? (list as T[]) : [];
    } catch {
        return [];
    }
}
