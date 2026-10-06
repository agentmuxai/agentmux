// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// How a link in the app opens. A click opens the system browser; a
// middle-click opens the page in a new AgentMux browser pane. Neither ever
// navigates the app's own window.

import { createBlock, getApi, openLink } from "@/app/store/global";

function isWebUrl(url: string): boolean {
    return /^https?:\/\//i.test(url.trim());
}

/** Open `url` in a new browser pane. A link that isn't a web page (mailto:,
 *  vscode:) goes to its system handler instead. */
export function openLinkInPane(url: string): void {
    if (!isWebUrl(url)) {
        void openLink(url);
        return;
    }
    void createBlock({ meta: { view: "browser", url } }).catch(() => openLink(url));
}

/** A link's click: the system browser. */
export function onLinkClick(e: MouseEvent, href: string): void {
    e.preventDefault();
    void openLink(href);
}

/** A link's middle-click: a new browser pane. The default (a new tab) would
 *  load the page in the app's own window. Other buttons are left alone. */
export function onLinkAuxClick(e: MouseEvent, href: string): void {
    if (e.button !== 1) return;
    e.preventDefault();
    openLinkInPane(href);
}

/** A middle-clicked link the host caught outside these handlers
 *  (crates/cef `on_open_url_from_tab`). */
export function registerLinkInPaneListener(): void {
    void getApi().listen<{ url: string }>("open-link-in-pane", (payload) => {
        if (payload?.url) openLinkInPane(payload.url);
    });
}
