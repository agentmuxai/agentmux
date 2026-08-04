// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Misc utilities — split out of global.ts (see global.ts's "Misc utilities"
// section for the original context). Re-exported from global.ts for
// backward-compat (97 files import from that module).

import { getApi } from "./app-api";

let cachedIsDev: boolean = null;
export function isDev() {
    if (cachedIsDev == null) cachedIsDev = getApi().getIsDev();
    return cachedIsDev;
}

let cachedUserName: string = null;
export function getUserName(): string {
    if (cachedUserName == null) cachedUserName = getApi().getUserName();
    return cachedUserName;
}

let cachedHostName: string = null;
export function getHostName(): string {
    if (cachedHostName == null) cachedHostName = getApi().getHostName();
    return cachedHostName;
}

export async function openLink(uri: string) {
    getApi().openExternal(uri);
}
