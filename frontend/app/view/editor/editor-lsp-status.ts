// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { LspState } from "./lsp/lsp-client";

/** The status chip's text for the language server's state; empty with none or once disposed. */
export function lspStatusChipText(state: LspState | null, language: string): string {
    if (!state) return "";
    switch (state.kind) {
        case "starting":
            return `${language}: starting…`;
        case "initializing":
            return `${language}: initializing…`;
        case "ready":
            return `${language}: ready`;
        case "missing":
            return `${language}: not installed`;
        case "notOnHost":
            return `${language}: off in this folder`;
        case "crashed":
            return `${language}: error`;
        case "disposed":
            return "";
    }
}

/** What the banner says for a file in a folder a container agent can write. */
export const NOT_ON_HOST_TEXT = "This folder belongs to a container agent, so no language server runs on the host for it.";

/** The name the banner of a state is dismissed under for the session: a missing server's language, one
 *  name for every container agent's folder; `null` for a state with no banner. */
export function lspBannerKey(state: LspState | null): string | null {
    if (state?.kind === "missing") return state.language;
    if (state?.kind === "notOnHost") return "(a container agent's folder)";
    return null;
}

/** The banner shows for a missing server and for a container agent's folder, until it is dismissed this session. */
export function lspBannerShows(state: LspState | null, dismissed: ReadonlySet<string>): boolean {
    const key = lspBannerKey(state);
    return key !== null && !dismissed.has(key);
}
