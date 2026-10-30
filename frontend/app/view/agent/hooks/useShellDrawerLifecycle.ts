// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The shell drawer's sub-block, from the pane's side: it must die with the pane, and the drawer
 * collapses around it when the shell exits.
 *
 * The drawer's PTY is kept alive across open and close (AgentShellSubblock) but a lingering shell
 * after the pane is gone is the leak class #1936 tracks. When an outer owner disposes the pane
 * silently, `block()` is already null by the time cleanup runs, so the sub-block's id is mirrored
 * into a plain variable on every change (keeping the last one) rather than read at cleanup.
 */

import { createEffect, onCleanup, type Accessor } from "solid-js";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { MOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { collapseDrawerOnShellExit } from "../shell-exit-collapse";

export function useShellDrawerLifecycle(opts: {
    blockId: string;
    /** The drawer's sub-block id from block meta; undefined before one exists or once the block is gone. */
    shellSubBlockId: Accessor<string | undefined>;
    paneModel: AgentPaneModel;
    clearTermWrite: () => void;
}): { handleShellExited: () => void } {
    let subBlockId: string | undefined;
    createEffect(() => {
        const id = opts.shellSubBlockId();
        if (id) subBlockId = id;
    });

    onCleanup(() => {
        if (subBlockId) void RpcApi.DeleteSubBlockCommand(TabRpcClient, { blockid: subBlockId });
    });

    /**
     * The shell exited cleanly: the human typed `exit`. The body is in `shell-exit-collapse.ts`
     * (testable; its three effects are each load-bearing). Here the local id is read and cleared, so
     * the cleanup above can't delete a sub-block this already removed.
     */
    const handleShellExited = () => {
        const exitedId = subBlockId;
        subBlockId = undefined;
        void collapseDrawerOnShellExit({
            parentBlockId: opts.blockId,
            exitedSubBlockId: exitedId,
            clearTermWrite: opts.clearTermWrite,
            collapseDrawer: () => opts.paneModel.dispatchPane({ type: "DetailsCollapse" }, "system"),
            setMeta: (args) => RpcApi.SetMetaCommand(TabRpcClient, { oref: args.oref, meta: args.meta as any }),
            deleteSubBlock: (args) => RpcApi.DeleteSubBlockCommand(TabRpcClient, args),
            makeORef: MOS.makeORef,
        }).catch((err) => {
            // The drawer already collapsed (synchronously, above): a failed RPC costs a leaked
            // sub-block, not a stuck UI.
            console.warn("[agent-view] shell-exit teardown failed:", err);
        });
    };

    return { handleShellExited };
}
