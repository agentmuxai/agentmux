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
        case "crashed":
            return `${language}: error`;
        case "disposed":
            return "";
    }
}

/** The install banner shows for a missing server, unless its language was dismissed this session. */
export function lspBannerShows(state: LspState | null, dismissed: ReadonlySet<string>): boolean {
    if (!state || state.kind !== "missing") return false;
    return !dismissed.has(state.language);
}
