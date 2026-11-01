// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The recovery actions for `open_in_other_pane` (this agent runs in another pane of this AgentMux):
 * Go there focuses the pane that holds it, across windows and tabs; Move here asks srv to stop the
 * agent there (`agent.move-here`) and then sends the message this pane refused.
 *
 * Spec: docs/specs/SPEC_AGENT_SAME_PROCESS_DUPLICATE_PANE_RECOVERY_2026_10_01.md §4.3, §4.5.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { revealBlock } from "@/app/util/reveal-block";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import { refusedSendOf } from "./refused-send";

/** srv waits up to 25 s for the other pane's agent to let go. */
const MOVE_HERE_TIMEOUT_MS = 60_000;

export function openInOtherPaneActions(opts: {
    blockId: string;
    log: (tag: string, message: string) => void;
    send: (message: string, attachments?: AttachmentRef[], system?: boolean) => Promise<void>;
}) {
    return {
        onGoThere: (holderBlockId: string) => {
            opts.log("agent", `Go there — focusing the pane that runs this agent (${holderBlockId})`);
            void revealBlock(holderBlockId, { source: "agent-duplicate" });
        },
        onMoveHere: async () => {
            opts.log("agent", "Move here — asking the other pane to let this agent go");
            const r = await RpcApi.AgentMoveHereCommand(TabRpcClient, { block_id: opts.blockId }, { timeout: MOVE_HERE_TIMEOUT_MS });
            opts.log("agent", r.from_block ? `Move here — released by ${r.from_block}` : "Move here — nobody else was running it");
            // The kept message stays until this pane's send is accepted (`sendMessage` forgets it
            // then); a resend that fails leaves it in the row, so Move here can be tried again.
            const refused = refusedSendOf(opts.blockId);
            if (!refused) return;
            await opts.send(refused.text, refused.attachments, refused.system);
            // `send` resolves even when the pane refused the resend: the message is still kept then,
            // and the row stays with the reason instead of reading as moved.
            if (refusedSendOf(opts.blockId)) throw new Error("the message was not sent; try Move here again");
        },
    };
}
