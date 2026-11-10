// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { getMuxObjectAtom, makeORef } from "@/store/mos";
import { agentNameOf } from "./pane-cover-log";

/** The agent's name for a block, read from its meta when asked (it arrives after the block does); nothing without a block id. */
export function blockAgentName(blockId: string | undefined): () => string | undefined {
    if (!blockId) return () => undefined;
    const block = getMuxObjectAtom<Block>(makeORef("block", blockId));
    return () => agentNameOf(block());
}
