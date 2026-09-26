// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Host capabilities (`HostCaps`, declared in types/custom.d.ts).
// docs/specs/SPEC_HOST_API_SEAM_2026_09_26.md

import { getApi } from "@/app/store/app-api";

/** The CEF desktop host: every capability. */
export const CEF_HOST_CAPS: Readonly<HostCaps> = Object.freeze({
    multiWindow: true,
    tearOff: true,
    nativeBrowserPane: true,
    nativeDialogs: true,
    updater: true,
    autostart: true,
    tray: true,
    localCliInstall: true,
    windowTransparency: true,
    nativeWindowChrome: true,
});

/** A host with none of the desktop-only capabilities. */
export const NO_HOST_CAPS: Readonly<HostCaps> = Object.freeze({
    multiWindow: false,
    tearOff: false,
    nativeBrowserPane: false,
    nativeDialogs: false,
    updater: false,
    autostart: false,
    tray: false,
    localCliInstall: false,
    windowTransparency: false,
    nativeWindowChrome: false,
});

/** Does the current host have `cap`? */
export function hostHas(cap: keyof HostCaps): boolean {
    return getApi().getHostCaps()[cap];
}
