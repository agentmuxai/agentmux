// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Wave file fetching — split out of global.ts (see global.ts's "Wave file
// fetching" section for the original context). Re-exported from global.ts
// for backward-compat (97 files import from that module).

import { getWebServerEndpoint } from "@/util/endpoints";
import { fetch } from "@/util/fetchutil";
import { getApi } from "./app-api";

export async function fetchWaveFile(
    zoneId: string,
    fileName: string,
    offset?: number
): Promise<{ data: Uint8Array; fileInfo: WaveFile }> {
    const usp = new URLSearchParams();
    usp.set("zoneid", zoneId);
    usp.set("name", fileName);
    if (offset != null) usp.set("offset", offset.toString());
    // Use X-AuthKey header instead of `?authkey=` query-string fallback.
    // The fallback was removed in the 2026-05-11 audit (C3) for everything
    // except the /ws upgrade route, where headers aren't possible.
    const headers: Record<string, string> = {};
    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) headers["X-AuthKey"] = authKey;
    }
    const resp = await fetch(getWebServerEndpoint() + "/agentmux/file?" + usp.toString(), { headers });
    if (!resp.ok) {
        if (resp.status === 404) return { data: null, fileInfo: null };
        throw new Error("error getting wave file: " + resp.statusText);
    }
    if (resp.status == 204) return { data: null, fileInfo: null };
    const fileInfo64 = resp.headers.get("X-ZoneFileInfo");
    if (fileInfo64 == null) throw new Error(`missing zone file info for ${zoneId}:${fileName}`);
    const fileInfo = JSON.parse(atob(fileInfo64));
    const data = await resp.arrayBuffer();
    return { data: new Uint8Array(data), fileInfo };
}
