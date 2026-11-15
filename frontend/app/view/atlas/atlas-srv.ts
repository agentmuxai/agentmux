// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { authHeaders } from "@/app/store/auth-headers";
import { getWebServerEndpoint } from "@/util/endpoints";

/** What srv's `route` answers, or null when srv is older than the route or cannot be reached. */
export async function srvJson<T>(route: string): Promise<T | null> {
    try {
        const res = await fetch(`${getWebServerEndpoint()}${route}`, { headers: authHeaders() });
        return res.ok ? ((await res.json()) as T) : null;
    } catch {
        return null;
    }
}

/** The list srv's `route` answers under `key`, or nothing when srv is older than the route or cannot be reached. */
export async function srvList<T>(route: string, key: string): Promise<T[]> {
    const list = (await srvJson<Record<string, unknown>>(route))?.[key];
    return Array.isArray(list) ? (list as T[]) : [];
}
