// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What each widget permission means, in the words the install prompt uses
// (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.4). One place,
// so the prompt and the Widgets list never say different things.

import type { WidgetSignatureInfo } from "@/app/store/rpc-api/widgets";

export interface PermissionText {
    text: string;
    /** Worth stopping for: shown in bold, with the warning icon. */
    strong?: boolean;
}

export function describePermission(permission: string): PermissionText {
    if (permission.startsWith("net:")) return { text: `Connect to ${permission.slice(4)}` };
    switch (permission) {
        case "storage":
            return { text: "Keep its own data on this computer" };
        case "files":
            return { text: "Open files you pick, and save files where you choose" };
        case "clipboard:write":
            return { text: "Copy text to your clipboard" };
        case "panes":
            return { text: "Open other kinds of panes" };
        case "agents:read":
            return { text: "See your agents' names and whether they're working" };
        case "agents:send":
            return {
                text: "Send messages to your agents. An agent acts on what it's told, so only allow this for a widget you trust.",
                strong: true,
            };
        default:
            return { text: permission };
    }
}

/** What the prompt says about who signed a widget
 *  (SPEC_WIDGET_SHARING_2026_10_10.md §2.3). `warning`: the publisher's
 *  pinned key didn't sign it. */
export function describeSignature(sig: WidgetSignatureInfo | null | undefined): { text: string; warning?: boolean } {
    if (!sig) return { text: "Not signed: AgentMux can't tell who made it." };
    const p = sig.publisher;
    switch (sig.state) {
        case "signed":
            return { text: `Signed by ${p} · ${sig.fingerprint} (the same key as your other ${p} widgets)` };
        case "signed_new":
            return { text: `Signed by ${p} · ${sig.fingerprint}. This is the first ${p} widget here: check the key with its author if it matters.` };
        case "key_changed":
            return {
                text: sig.fingerprint
                    ? `Not signed by the key that signed your other ${p} widgets (${sig.pinned}); this one is signed by ${sig.fingerprint}. It may not be from the same author.`
                    : `Not signed, though your other ${p} widgets are (${sig.pinned}). It may not be from the same author.`,
                warning: true,
            };
        default:
            return { text: "Not signed: AgentMux can't tell who made it." };
    }
}

/** The one line a trusted widget's prompt has instead of a permission list. */
export const TRUSTED_WIDGET_WARNING =
    "This widget runs as part of AgentMux, with full access to everything AgentMux can do. Only install it if you trust its author.";
