// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export const PlatformMacOS = "darwin";
const PlatformLinux = "linux";
export const PlatformWindows = "win32";
/** @deprecated Use getPlatform(), isMacOS(), isLinux(), isWindows() instead. Direct reads at module scope capture the default "darwin" before setPlatform() runs. */
export let PLATFORM: NodeJS.Platform = PlatformMacOS;

export function setPlatform(platform: NodeJS.Platform) {
    PLATFORM = platform;
}

export function getPlatform(): NodeJS.Platform {
    return PLATFORM;
}

export function isMacOS(): boolean {
    return PLATFORM === PlatformMacOS;
}

export function isLinux(): boolean {
    return PLATFORM === PlatformLinux;
}

export function isWindows(): boolean {
    return PLATFORM === PlatformWindows;
}
