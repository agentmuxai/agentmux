// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentShellSubblock — Phase 0 spike for
 * docs/specs/SPEC_AGENT_SHELL_XTERM_TERMINAL_2026_07_03.md.
 *
 * Mounts a real xterm.js + PTY terminal (Model A: a headless `term`
 * sub-block parented to the agent block, resolved in that spec's §4)
 * inside the composer details drawer. The sub-block id is persisted on
 * the agent block's meta (`term:shellsubblockid`) so it's created once
 * per pane and reused across drawer open/close — only the xterm
 * renderer is mounted/disposed here (drawer close); the PTY itself is
 * only killed when the pane closes (see agent-view.tsx's pane-level
 * onCleanup, which calls DeleteSubBlockCommand).
 */

import { onCleanup, onMount, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { sendWSCommand } from "@/app/store/ws";
import { stringToBase64 } from "@/util/util";
import { TermWrap } from "@/app/view/term/termwrap";

interface AgentShellSubblockProps {
    parentBlockId: string;
    cwd: string;
    existingSubBlockId: string | undefined;
    onSubBlockCreated: (subBlockId: string) => void;
}

export const AgentShellSubblock = (props: AgentShellSubblockProps): JSX.Element => {
    let containerRef: HTMLDivElement | undefined;
    let termWrap: TermWrap | undefined;
    let disposed = false;

    onMount(() => {
        void (async () => {
            let subBlockId = props.existingSubBlockId;
            if (!subBlockId) {
                const oref = await RpcApi.CreateSubBlockCommand(TabRpcClient, {
                    parentblockid: props.parentBlockId,
                    blockdef: {
                        meta: {
                            view: "term",
                            controller: "shell",
                            "cmd:cwd": props.cwd,
                        },
                    },
                });
                // ORef wire format is always "<otype>:<oid>" (wos.ts makeORef) —
                // oid is a UUID, never contains a colon, so a single split is safe.
                subBlockId = oref.slice(oref.indexOf(":") + 1);
                props.onSubBlockCreated(subBlockId);
            }
            if (disposed || !containerRef) return;
            const wrap = new TermWrap(
                subBlockId,
                containerRef,
                {
                    fontSize: 13,
                    fontFamily: "Hack",
                    allowTransparency: false,
                    scrollback: 2000,
                    allowProposedApi: true,
                },
                {
                    useWebGl: true,
                    // Bare sendDataHandler mirroring TermViewModel's fast path
                    // (termViewModel.ts:370-379) — blockinput, not the
                    // controllerinput RPC, so consecutive keystrokes stay in
                    // TCP order. No chunked-paste handling for this spike.
                    sendDataHandler: (data: string) => {
                        sendWSCommand({
                            wscommand: "blockinput",
                            blockid: subBlockId,
                            inputdata64: stringToBase64(data),
                        } as BlockInputWSCommand);
                    },
                }
            );
            termWrap = wrap;
            await wrap.init();
        })();
    });

    onCleanup(() => {
        disposed = true;
        termWrap?.dispose();
    });

    return <div class="agent-shell-subblock" ref={containerRef} />;
};
