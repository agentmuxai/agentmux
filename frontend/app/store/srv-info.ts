// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What srv says about itself on every WebSocket connect (the `srvinfo` event,
// crates/srv/src/srv_info.rs): its version, and the machine it runs on. That
// machine is the one whose names the UI shows and whose paths it builds; it is
// only the same as the UI's own when the UI is the desktop app.
//
// The UI and srv ship together, so a different version means one of them was
// updated under the other, and the status bar offers a reload.

import { createSignal } from "solid-js";

export interface SrvInfo {
    version: string;
    /** srv's OS, as `process.platform` names it ("darwin", "win32", "linux"). */
    platform: string;
    userName: string;
    hostName: string;
    /** The account-wide AgentMux root on srv's machine (`~/.agentmux`, or its override). */
    homeDir: string | null;
}

const [srvInfo, setSrvInfo] = createSignal<SrvInfo | null>(null);
const [srvSilent, setSrvSilent] = createSignal(false);

/** What `versionSkew()` returns for a srv that never sent `srvinfo`. */
export const OLDER_SRV = "an older version";

/**
 * Called once startup has had a WebSocket RPC reply. srv sends `srvinfo`
 * before it answers any RPC, so none by then means a srv from before
 * `srvinfo` existed: a version mismatch the version can't name.
 */
export function noteSrvInfoDue(): void {
    if (srvInfo() == null) setSrvSilent(true);
}

/** What srv last reported, or null before the first connect. */
export { srvInfo };

/** This UI's own version, stamped at build time. */
export const UI_VERSION: string = __AGENTMUX_VERSION__;

/**
 * Called with every message the WebSocket delivers (`initWshrpc`), before it is
 * routed: records a `srvinfo` event. srv sends it as soon as the socket opens,
 * which can be before anything subscribes to events, so it is caught here
 * rather than through a subscription that could miss it.
 */
export function noteSrvInfoMessage(msg: { command?: string; data?: { event?: string; data?: unknown } } | null): void {
    if (msg?.command === "eventrecv" && msg.data?.event === "srvinfo") {
        onSrvInfo(msg.data.data as Partial<SrvInfo> | null);
    }
}

/** Records a `srvinfo` payload. Ignores one without a version. */
export function onSrvInfo(data: Partial<SrvInfo> | null | undefined): void {
    if (typeof data?.version !== "string" || data.version === "") return;
    setSrvSilent(false);
    setSrvInfo({
        version: data.version,
        platform: data.platform ?? "",
        userName: data.userName ?? "",
        hostName: data.hostName ?? "",
        homeDir: data.homeDir ?? null,
    });
}

/** srv's version when it differs from this UI's (`OLDER_SRV` for one too old
 *  to say), otherwise null. */
export function versionSkew(): string | null {
    const v = srvInfo()?.version;
    if (v == null) return srvSilent() ? OLDER_SRV : null;
    return v !== UI_VERSION ? v : null;
}
