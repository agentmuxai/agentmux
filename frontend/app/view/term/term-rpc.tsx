// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { RpcResponseHelper, RpcClient } from "@/app/store/rpc-client";
import { makeFeBlockRouteId } from "@/app/store/rpc-router";
import { TermViewModel } from "@/app/view/term/term";

export class TermRpcClient extends RpcClient {
    blockId: string;
    model: TermViewModel;

    constructor(blockId: string, model: TermViewModel) {
        super(makeFeBlockRouteId(blockId));
        this.blockId = blockId;
        this.model = model;
    }

    async handle_termgetscrollbacklines(
        rh: RpcResponseHelper,
        data: CommandTermGetScrollbackLinesData
    ): Promise<CommandTermGetScrollbackLinesRtnData> {
        const termWrap = this.model.termRef.current;
        if (!termWrap || !termWrap.terminal) {
            return {
                totallines: 0,
                linestart: data.linestart,
                lines: [],
                lastupdated: 0,
            };
        }

        const buffer = termWrap.terminal.buffer.active;
        const totalLines = buffer.length;
        const lines: string[] = [];

        const startLine = Math.max(0, data.linestart);
        const endLine = Math.min(totalLines, data.lineend);

        for (let i = startLine; i < endLine; i++) {
            const bufferIndex = totalLines - 1 - i;
            const line = buffer.getLine(bufferIndex);
            if (line) {
                lines.push(line.translateToString(true));
            }
        }

        lines.reverse();

        return {
            totallines: totalLines,
            linestart: startLine,
            lines: lines,
            lastupdated: termWrap.lastUpdated,
        };
    }
}
