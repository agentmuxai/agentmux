// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * When a page fails to load, the host shows its own error page as a `data:`
 * URI (crates/cef `client/navigation.rs` `show_load_error_page`). That URI
 * stands for the address that failed, which is what a pane should show,
 * save and reopen, as Chrome does. The host reports the failed address
 * itself (`error_page_failed_url`). This reads the same thing on the frontend,
 * for a pane saved with the error page's URI before that fix: restoring one
 * used to turn the URI into a Google search.
 */

const PREFIX = "data:text/html;base64,";
const MARKER = "#amux-failed-url=";

/** The address an AgentMux load-error page stands for, or undefined. */
export function errorPageFailedUrl(url: string): string | undefined {
    if (!url.startsWith(PREFIX)) return undefined;
    try {
        const at = url.indexOf(MARKER);
        if (at >= 0) {
            const hex = url.slice(at + MARKER.length);
            if (hex.length % 2 !== 0 || !/^[0-9a-f]*$/i.test(hex)) return undefined;
            const bytes = new Uint8Array(hex.length / 2);
            for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
            return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
        }
        // Saved before the marker existed: the page names the address itself.
        const binary = atob(url.slice(PREFIX.length));
        const html = new TextDecoder().decode(Uint8Array.from(binary, (c) => c.charCodeAt(0)));
        const named = html.match(/Could not load <code>([^<]*)<\/code>/);
        return named ? unescapeHtml(named[1]) : undefined;
    } catch {
        return undefined;
    }
}

/** The inverse of the host's `html_escape`. */
function unescapeHtml(s: string): string {
    return s
        .replace(/&#39;/g, "'")
        .replace(/&quot;/g, '"')
        .replace(/&gt;/g, ">")
        .replace(/&lt;/g, "<")
        .replace(/&amp;/g, "&");
}
