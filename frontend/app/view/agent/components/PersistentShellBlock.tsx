// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PersistentShellBlock — compact document row for a long-running shell process.
 *
 * Displays as a single colored line in the conversation (status color on the
 * left border, sigil, title, elapsed timer, live-tail of last output).
 * Click toggles an inline log panel showing all streamed output.
 *
 * Spec: docs/specs/SPEC_PERSISTENT_SHELL_NODE_2026_06_11.md
 */

import clsx from "clsx";
import { IconButton } from "@/app/element/ui/Button";
import { Show, createMemo, createSignal, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatElapsedClock, formatExactTime, formatTimeAgo } from "@/util/format-time";
import { useNodePeek } from "../hooks/useNodePeek";
import { createBodyMount } from "./body-mount";
import { ChunkPreview } from "./ChunkPreview";
import { PeekOverlay } from "./PeekOverlay";
import { PeekMetaRow } from "./PeekMetaRow";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { ShellNode } from "../../../agent-model/types";
import { rowDisclosure } from "../virtualization/disclosure";

interface PersistentShellBlockProps {
    node: ShellNode;
    pinned: boolean;
    onTogglePin: () => void;
}


/** The last non-empty stdout/stderr line of the log, trimmed. */
function lastOutputLine(node: ShellNode): string | undefined {
    const chunks = node.log.chunks;
    if (!chunks.length) return undefined;
    // Walk from the end to find the last non-empty stdout/stderr line
    for (let i = chunks.length - 1; i >= 0; i--) {
        const c = chunks[i];
        if ((c.kind === "stdout" || c.kind === "stderr") && c.content.trim()) {
            return c.content.trim();
        }
    }
    return undefined;
}

function sigilFor(status: ShellNode["status"]): string {
    switch (status) {
        case "running": return "⟩";
        case "exited-ok": return "✓";
        case "exited-err": return "✗";
        case "stopped": return "■";
    }
}

/** What the row shows of the process: the clock, the live tail, the sigil, and whether the panel is open. */
function useShellView(props: PersistentShellBlockProps) {
    const tick = useTick(1000);

    const elapsed = createMemo(() => {
        const end = props.node.exitedAt ?? (tick(), Date.now());
        return formatElapsedClock(end - props.node.spawnedAt);
    });

    const lastLine = createMemo(() => lastOutputLine(props.node));

    const statusSigil = createMemo(() => sigilFor(props.node.status));

    // Pin-to-expand only (virtualization/disclosure.ts).
    const expanded = () => rowDisclosure(props.node, { pinned: props.pinned }).open;
    // The log is built only while the panel is open, and for a moment after
    // (SPEC_TOOL_BODY_UNMOUNT_ON_COLLAPSE_2026_10_12.md §3).
    const logMounted = createBodyMount(expanded);
    return { elapsed, lastLine, statusSigil, expanded, logMounted };
}

/** The peek tooltip's state and text (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25). */
function useShellPeek(props: PersistentShellBlockProps) {
    // Suppressed once expanded — the full command is already visible in the
    // panel header (`.agent-shell-header-cmd`), same rule ToolBlock uses.
    const peekTick = useTick(1000);
    const { isPeeking, panelVisible: peekPanelVisible, rowEl: peekRowEl, setRowEl: setPeekRowEl, handlePeekEnter, handlePeekLeave } = useNodePeek();
    const peekTimeText = createMemo(() => {
        if (!peekPanelVisible()) return null;
        peekTick();
        return `${formatExactTime(props.node.spawnedAt)} · ${formatTimeAgo(props.node.spawnedAt)}`;
    });
    const peekEstimateText = createMemo(() => {
        const count = estimateTokenCount(props.node.cmd);
        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
    });
    return { isPeeking, peekRowEl, setPeekRowEl, handlePeekEnter, handlePeekLeave, peekTimeText, peekEstimateText };
}

/** Stop button (Phase 3): tree-kills the running process. Status updates
 *  arrive via the `stopped` exit event, so we don't optimistically mutate. */
function useShellStop(props: PersistentShellBlockProps) {
    const [stopping, setStopping] = createSignal(false);
    const handleStop = async (e: MouseEvent) => {
        e.stopPropagation(); // don't toggle the expand panel
        if (stopping()) return;
        setStopping(true);
        try {
            await RpcApi.ShellStopCommand(TabRpcClient, { shell_id: props.node.id });
        } catch {
            // Best-effort — the exit event reconciles the final status.
        } finally {
            setStopping(false);
        }
    };
    return { stopping, handleStop };
}

type ShellView = ReturnType<typeof useShellView>;
type ShellPeek = ReturnType<typeof useShellPeek>;
type ShellStop = ReturnType<typeof useShellStop>;

/** The summary line: sigil, title, clock, live tail and Stop button; it is also the peek's anchor. */
const ShellSummary = (props: { node: ShellNode; view: ShellView; peek: ShellPeek; stop: ShellStop; onTogglePin: () => void }): JSX.Element => (
    <div
        ref={props.peek.setPeekRowEl}
        class="agent-shell-summary"
        onClick={props.onTogglePin}
        onMouseEnter={props.peek.handlePeekEnter}
        onMouseLeave={props.peek.handlePeekLeave}
    >
        <span class="agent-shell-sigil">{props.view.statusSigil()}</span>
        <span class="agent-shell-title">{props.node.title}</span>
        <span class="agent-shell-elapsed">[{props.view.elapsed()}]</span>
        <Show when={props.view.lastLine()}>
            {/* U+2192 (→), not U+21B3 (↳) — see ActivityRow.tsx's
                identical comment. SPEC_ACTIVITY_DOCK_TITLE_WIDTH_AND_TAIL_GLYPH_2026_09_05.md §3. */}
            <span class="agent-shell-live-tail">→ {props.view.lastLine()}</span>
        </Show>
        <Show when={props.node.status === "running"}>
            <IconButton
                icon="stop"
                label="Stop process"
                tooltip={false}
                tone="danger"
                density="compact"
                class="agent-shell-stop"
                disabled={props.stop.stopping()}
                onClick={props.stop.handleStop}
            />
        </Show>
    </div>
);

/** The pin-to-open panel: the command, the exit code and the log. */
const ShellPanel = (props: { node: ShellNode; view: ShellView }): JSX.Element => (
    <div
        class={clsx("agent-tool-panel", {
            "agent-tool-panel--hidden": !props.view.expanded(),
            "agent-tool-panel--flow": props.view.expanded(),
        })}
        inert={!props.view.expanded()}
        aria-hidden={!props.view.expanded()}
        onClick={(e) => e.stopPropagation()}
    >
        <div class="agent-tool-overlay">
            <div class="agent-tool-overlay-header">
                <span class="agent-shell-header-cmd">{props.node.cmd}</span>
                <Show when={props.node.exitCode !== undefined}>
                    <span class="agent-tool-overlay-status-label">
                        exit {props.node.exitCode}
                    </span>
                </Show>
            </div>
            <div class="agent-tool-overlay-log">
                {/* Joined into lines, collapsed and capped the way every
                    streamed log is (ChunkPreview.tsx). Only while the
                    panel is open (and briefly after), like a tool's body. */}
                <Show when={props.view.logMounted()}>
                    <ChunkPreview chunks={props.node.log.chunks} linkify />
                    <Show when={props.node.log.open}>
                        <div class="agent-shell-streaming-indicator" />
                    </Show>
                </Show>
            </div>
        </div>
    </div>
);

export const PersistentShellBlock = (props: PersistentShellBlockProps): JSX.Element => {
    const view = useShellView(props);
    const peek = useShellPeek(props);
    const stop = useShellStop(props);

    return (
        <div
            class={clsx("agent-shell-block", {
                "running": props.node.status === "running",
                "exited-ok": props.node.status === "exited-ok",
                "exited-err": props.node.status === "exited-err",
                "stopped": props.node.status === "stopped",
                "expanded": view.expanded(),
                "collapsed": !view.expanded(),
            })}
        >
            {/* The peek is the summary line's, not the expanded output's, as for
                a tool call (SPEC_AGENT_PANE_TOOL_PEEK_ON_LINE_AND_MD_OPEN_IN_EDITOR_2026_10_11.md §1). */}
            <ShellSummary node={props.node} view={view} peek={peek} stop={stop} onTogglePin={props.onTogglePin} />
            <ShellPanel node={props.node} view={view} />
            <PeekOverlay show={peek.isPeeking() && !view.expanded()} rowEl={peek.peekRowEl}>
                <PeekMetaRow time={peek.peekTimeText()} tokens={peek.peekEstimateText()} />
                <div class="agent-node-peek-tooltip-body agent-node-peek-tooltip-body--shell">{props.node.cmd}</div>
            </PeekOverlay>
        </div>
    );
};

PersistentShellBlock.displayName = "PersistentShellBlock";
