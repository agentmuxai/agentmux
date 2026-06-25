// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createBlockSplitVertically } from "@/app/store/global";
import type { SlashCommand, SlashResult } from "../types";

export const terminalCommand: SlashCommand = {
    name: "terminal",
    aliases: ["term"],
    category: "session",
    description: "Open the agent's working directory in a new terminal pane below",
    arg: { kind: "none" },
    handler: async (ctx): Promise<SlashResult> => {
        const cwd = ctx.block()?.meta?.["cmd:cwd"] as string | undefined;
        const blockDef = {
            meta: {
                view: "term",
                ...(cwd ? { "cmd:cwd": cwd } : {}),
            },
        };
        await createBlockSplitVertically(blockDef, ctx.blockId, "after");
        return { kind: "ok", message: cwd ? `Terminal opened at ${cwd}` : "Terminal opened" };
    },
};
