// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export const PlatformMacOS = "darwin";
export const PlatformLinux = "linux";
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

export function makeNativeLabel(isDirectory: boolean) {
    let managerName: string;
    if (!isDirectory) {
        managerName = "Default Application";
    } else if (PLATFORM === PlatformMacOS) {
        managerName = "Finder";
    } else if (PLATFORM === PlatformWindows) {
        managerName = "Explorer";
    } else {
        managerName = "File Manager";
    }

    let fileAction: string;
    if (isDirectory) {
        fileAction = "Reveal";
    } else {
        fileAction = "Open File";
    }
    return `${fileAction} in ${managerName}`;
}
