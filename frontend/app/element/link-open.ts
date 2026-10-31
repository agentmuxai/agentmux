// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// How a link in the app opens. A web link opens the system browser on a click
// and a new AgentMux browser pane on a middle-click. A link to something in
// AgentMux itself opens there: a local file in the Editor, a folder in Files,
// `agentmux://task/<n>` on the Kanban (link-target.ts). A Ctrl+click (Cmd on
// macOS) opens whichever pane the link calls for as a floating window
// (store/float-modifier.ts). None ever navigates the app's own window.

import { takeFloatRequest } from "@/app/store/float-modifier";
import { createBlock, getApi, openLink } from "@/app/store/global";
import { classifyHref } from "./link-target";

/** Open what `href` points at inside AgentMux. False for a link that isn't one of those. */
function openInApp(href: string, floating: boolean): boolean {
    const target = classifyHref(href);
    const failed = (err: unknown) => console.warn("[link] couldn't open", href, err);
    switch (target.kind) {
        case "file":
            void import("./link-destinations").then((d) => d.openPathLink(target.path, { floating })).catch(failed);
            return true;
        case "task":
            void import("./link-destinations").then((d) => d.openTaskLink(target.num, { floating })).catch(failed);
            return true;
        case "blocked":
            // An `agentmux:` URL that isn't a task link: a link only navigates, so the OS never gets it.
            return true;
        default:
            return false;
    }
}

/** Open `url` in a new browser pane (a floating one when asked). A link that isn't a web page (mailto:,
 *  vscode:) goes to its system handler instead. */
function openLinkInPane(url: string, floating = false): void {
    if (openInApp(url, floating)) return;
    if (classifyHref(url).kind !== "web") {
        void openLink(url);
        return;
    }
    if (floating) {
        void import("./link-destinations")
            .then((d) => d.openWebLinkFloating(url))
            .catch(() => openLink(url));
        return;
    }
    void createBlock({ meta: { view: "browser", url } }).catch(() => openLink(url));
}

/** A link's click: the system browser, or AgentMux itself for a file or a task. With the float modifier
 *  held, the pane the link calls for (a browser pane, for a web page) opens floating. */
export function onLinkClick(e: MouseEvent, href: string): void {
    e.preventDefault();
    // The click is this link's: no other pane open that follows it is a float.
    const floating = takeFloatRequest();
    if (floating && classifyHref(href).kind === "web") {
        openLinkInPane(href, true);
        return;
    }
    if (openInApp(href, floating)) return;
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
