// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Misc utilities — split out of global.ts (see global.ts's "Misc utilities"
// section for the original context). Re-exported from global.ts for
// backward-compat (97 files import from that module).

import { getApi } from "./app-api";
import { srvInfo } from "./srv-info";

let cachedIsDev: boolean = null;
export function isDev() {
    if (cachedIsDev == null) cachedIsDev = getApi().getIsDev();
    return cachedIsDev;
}

/** The user srv runs as (`srvinfo`); "" until srv has said. Reactive. */
export function getUserName(): string {
    return srvInfo()?.userName ?? "";
}

/** The machine srv runs on (`srvinfo`); "" until srv has said. Reactive. */
export function getHostName(): string {
    return srvInfo()?.hostName ?? "";
}

export async function openLink(uri: string) {
    getApi().openExternal(uri);
}
