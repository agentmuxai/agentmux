// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * /clear — frontend-only document reset. Does not touch the backend
 * session or cancel the running CLI process; just empties the visible
 * document so the pane looks fresh.
 */

import type { SlashCommand, SlashResult } from "../types";

export const clearCommand: SlashCommand = {
    name: "clear",
    category: "session",
    description: "Clear the visible document (frontend-only)",
    arg: { kind: "none" },
    handler: async (ctx): Promise<SlashResult> => {
        const [, setDocument] = ctx.documentAtom;
        setDocument([]);
        return { kind: "ok", message: "chat cleared" };
    },
};
