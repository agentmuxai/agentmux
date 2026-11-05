// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { onCleanup, onMount } from "solid-js";
import { getApi } from "@/util/app-api";
import { openIncognitoTab } from "./browser-profile-menu";
import type { AddressBarState, BrowserNavBarProps } from "./browser-nav-address";

/**
 * The host's pane shortcuts for this bar: focus the address, go Home, open an Incognito tab.
 * Ctrl+L (Cmd+L on macOS) is dead by default in a browser pane — CEF
 * intercepts it at the pre-key stage (crates/cef/src/client/handlers.rs)
 * since a pane's keystrokes go to the CEF child browser, not this webview,
 * and emits `browser-pane-shortcut` instead of forwarding to the (possibly
 * untrusted) page. See issue #1190.
 */
export function useBrowserPaneShortcuts(
    props: BrowserNavBarProps,
    address: AddressBarState,
    home: () => string,
    goHome: () => void,
): void {
    const model = props.model;
    const diag = props.diag;
    const windowLabel = props.windowLabel;
    onMount(() => {
        let unsub: (() => void) | undefined;
        void getApi().listen<{ block_id: string; action: string }>(
            "browser-pane-shortcut",
            (payload) => {
                if (payload.block_id !== model.blockId) return;
                if (payload.action === "home") {
                    goHome();
                    return;
                }
                if (payload.action === "new-incognito") {
                    openIncognitoTab(model, home());
                    return;
                }
                if (payload.action !== "focus-address") return;
                diag(`shortcut-focus-address`);
                // Same OS-focus handoff the address bar's own onMouseDown
                // needs (see its comment): the pane HWND currently
                // holds OS keyboard focus, so a bare DOM .focus() call
                // wouldn't actually move keystrokes to this input without
                // first reclaiming OS focus for this window.
                getApi().reclaimWindowFocus(windowLabel).catch(() => {});
                address.input()?.focus();
                address.input()?.select();
            }
        ).then((fn) => {
            unsub = fn;
        });
        onCleanup(() => unsub?.());
    });
}
