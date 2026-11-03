// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Reads this window's version, channel, window and tab for a toast's Copy button (toast-debug.ts).

import { atoms, getApi } from "@/app/store/global";
import { windowId } from "@/app/store/window-identity";
import type { ToastDebugEnv } from "./toast-debug";

/** The real version, channel, window and tab, read when the button is pressed. */
export function liveToastDebugEnv(): ToastDebugEnv {
    const about = getApi().getAboutModalDetails?.();
    let tabId: string | undefined;
    try {
        tabId = atoms.activeTabId() || undefined;
    } catch {
        tabId = undefined;
    }
    return {
        version: about?.version,
        build: about?.buildLabel,
        channel: about?.channel,
        platform: [about?.platform, about?.arch].filter(Boolean).join(" ") || undefined,
        windowId: windowId() || undefined,
        tabId,
    };
}
