// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The state behind a ToolBlock row: when it holds itself open, whether its panel is open and its
 * body mounted, the read-back of a result unloaded from memory, and the hover peek. The hooks
 * are called in this order from ToolBlock, which is also the order their effects are created.
 * Props are read as `props.X`, never destructured: pin toggles mutate `documentState` without
 * touching the document array, so a destructured prop would never see them.
 */

import { createEffect, createMemo, createSignal, untrack, type Accessor, type Setter } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatPeekTime } from "@/util/format-time";
import type { AgentDispatch } from "../../swarm/swarm-model";
import type { ToolNode } from "../../../agent-model/types";
import { TOOL_STATUS } from "../../../agent-model/tool-status";
import { useNodePeek, type NodePeek } from "../hooks/useNodePeek";
import { useAgentDormant } from "../agent-dormancy";
import { rowDisclosure } from "../virtualization/disclosure";
import { arrivedLive } from "../virtualization/live-arrival";
import { useToolResultLoader } from "../tool-result-loader";
import { createBodyMount } from "./body-mount";
import { type ToolHeaderParts } from "./tool-header";

export interface ToolBlockProps {
    node: ToolNode;
    /** User has clicked to pin this tool block open. */
    pinned: boolean;
    /**
     * Held expanded after completing live on screen (`documentState.heldOpenNodes`).
     * Set by `onHoldOpen` on completion; cleared by the pane's scroll-off scan
     * once the row leaves the top — the scroll-driven replacement for the old
     * 3 s post-completion timer.
     */
    heldOpen?: boolean;
    /** Toggle the pinned state (called on click of the collapsed row). */
    onTogglePin: () => void;
    /** Mark this tool held-open — called once on its active→inactive transition. */
    onHoldOpen?: () => void;
    /** Content-first tools (agent-model/tool-descriptors.ts) only: the user collapsed
     *  it (`documentState.collapsedNodes`). They're expanded by default. */
    userCollapsed?: boolean;
    /** Content-first tools only: toggle `userCollapsed` (the header click). */
    onToggleCollapse?: () => void;
    /** Ordinal-matched live dispatch for an Agent/Task/Workflow tool call —
     *  see `activity/dispatch-correlation.ts`. Undefined when no confident
     *  match was found, or for any other tool kind. */
    dispatchMatch?: AgentDispatch;
}

/**
 * A tool stays expanded after completing live until its row scrolls off the top — the
 * "post-completion hold" lives in `documentState.heldOpenNodes` (read via props.heldOpen). Here we
 * just detect the active → inactive TRANSITION and mark the tool held-open via onHoldOpen.
 *
 * Gate on a real TRANSITION (not a status-value snapshot): firing on mount for an already-completed
 * tool would auto-expand every row of a loaded transcript (#988). prevNodeId guards <Index>
 * slot-position state leakage — when a streaming-buffer cap-advance swaps the node at this slot,
 * the old prevStatus must not seed the incoming node's transition baseline.
 */
export function useHoldOpenOnFinish(props: ToolBlockProps): void {
    let prevStatus: ToolNode["status"] = props.node.status;
    let prevNodeId: string = props.node.id;
    let arrivalChecked = false;
    const isActive = (s: ToolNode["status"]): boolean => TOOL_STATUS[s]?.active === true;
    // A fast tool (a Read is a few ms) can start and finish inside one stream
    // flush, so its row is first drawn already finished and there is no
    // transition to see: it came out collapsed, sometimes. Live tool calls are
    // stamped when the call arrives and replayed ones have no stamp
    // (stream-parser `isReplay`), so a finished row stamped within the last few
    // seconds is a live completion we missed, and is held like one we saw. A
    // loaded transcript has no stamps, so it still opens collapsed (#988).
    const holdIfFinishedOnArrival = (): void => {
        const n = props.node;
        if (isActive(n.status) || TOOL_STATUS[n.status]?.dismissed) return;
        if (!arrivedLive(n.timestamp)) return;
        props.onHoldOpen?.();
    };
    createEffect(() => {
        const s = props.node.status;
        const id = props.node.id;
        if (id !== prevNodeId) {
            prevNodeId = id;
            prevStatus = s;
            untrack(holdIfFinishedOnArrival);
            return;
        }
        if (!arrivalChecked) {
            arrivalChecked = true;
            untrack(holdIfFinishedOnArrival);
        }
        if (isActive(prevStatus) && !isActive(s)) {
            props.onHoldOpen?.();
        }
        prevStatus = s;
    });
}

export interface ToolDisclosure {
    disclosure: Accessor<ReturnType<typeof rowDisclosure>>;
    expanded: Accessor<boolean>;
    /** The pointer is inside an open, pin-toggled row: hold it open until it leaves. */
    setUserHolding: Setter<boolean>;
    /** The preview at three times its usual height, toggled from its top-right actions. */
    tall: Accessor<boolean>;
    setTall: Setter<boolean>;
    /** The height toggle shows for a preview that scrolls, and for one already tall (it has to shrink back). */
    showHeightToggle: () => boolean;
    /** ToolOverlayLog reports whether the preview scrolls. */
    setScrolls: Setter<boolean>;
    bodyMounted: Accessor<boolean>;
    dormant: Accessor<boolean>;
    /** Whether the body's output can be seen now: the panel is open and the pane isn't dormant. */
    bodyShowing: () => boolean;
    onHeaderClick: () => void;
}

/**
 * Open or closed comes from the one rule every row and the virtualizer share
 * (virtualization/disclosure.ts): pinned; auto while running or awaiting approval, and while held
 * after finishing on screen (not for a dismissed denied/canceled tool); a finished content-first
 * tool (WebSearch) open until the user collapses it. Hover is no trigger for expansion
 * (SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28), with one exception: while the pointer is inside an
 * already-expanded block it stays open, so a scroll-off collapse can't fold it mid-read.
 */
export function useToolDisclosure(props: ToolBlockProps): ToolDisclosure {
    const disclosure = () =>
        rowDisclosure(props.node, { pinned: props.pinned, collapsed: props.userCollapsed, held: props.heldOpen });
    const [userHolding, setUserHolding] = createSignal(false);
    const expanded = () => disclosure().open || userHolding();
    const [tall, setTall] = createSignal(false);
    // The toggle is for seeing more of a preview that is cut off, so a preview that fits has none,
    // unless it is already tall.
    const [scrolls, setScrolls] = createSignal(false);
    const showHeightToggle = () => tall() || scrolls();
    // The panel's body (full tool output: diffs, file previews, Bash output, all highlighted with
    // shiki) mounts while the panel is open and for a short linger after it closes, so the
    // collapse transition runs on the real content; then it goes, and opening again builds it
    // again. A row that is never opened never builds it: restoring history used to highlight every
    // collapsed Edit/Read/Bash row on mount (SPEC_AGENT_OPEN_LATENCY_2026_09_27.md F4: shiki + its
    // wasm ≈ 94 ms of one open). It used to stay forever once opened — and every live tool call
    // opens while it runs — so a session kept every tool's full output
    // (SPEC_TOOL_BODY_UNMOUNT_ON_COLLAPSE_2026_10_12.md).
    const bodyMounted = createBodyMount(expanded);
    // While the pane is dormant (a hidden pane-stack member, or a window tab not displayed) the
    // body keeps showing what it showed instead of building new output (ToolOverlayLog's `open`):
    // with a dozen agents streaming, panes on hidden tabs built more tool output than the visible
    // ones (SPEC_AGENT_PANE_STREAM_FRAME_BUDGET_2026_10_13.md §2.3). The body stays mounted, so
    // the row keeps its height and nothing moves when the pane is shown again; it catches up then.
    const dormant = useAgentDormant();
    const bodyShowing = (): boolean => expanded() && !dormant();
    // The body is gone, and with it the box that reported; the next one reports afresh.
    createEffect(() => {
        if (!bodyMounted()) setScrolls(false);
    });
    // The header click flips whichever set the rule says this row uses:
    // collapsedNodes for an open-by-default (content-first) tool, else the pin.
    const onHeaderClick = () =>
        disclosure().toggle === "collapse" ? props.onToggleCollapse?.() : props.onTogglePin();
    return { disclosure, expanded, setUserHolding, tall, setTall, showHeightToggle, setScrolls, bodyMounted, dormant, bodyShowing, onHeaderClick };
}

export interface UnloadedResult {
    loadState: Accessor<"idle" | "loading" | "failed">;
    /** The result body's height when its text last left memory; 0 if unknown. */
    bodyPx: { value: number };
}

/**
 * A result unloaded from memory is read back from the transcript when its row is opened
 * (SPEC_AGENT_PANE_TOOL_RESULT_UNLOADING_2026_10_01.md §3.4).
 */
export function useUnloadedResult(props: ToolBlockProps, view: ToolDisclosure): UnloadedResult {
    const loadResult = useToolResultLoader();
    const [loadState, setLoadState] = createSignal<"idle" | "loading" | "failed">("idle");
    // Not while the pane is hidden: a hidden pane's held rows stay open but may
    // have their text unloaded (useLiveFeedRollOff), and read it back on show.
    createEffect(() => {
        if (view.dormant() || !view.expanded() || !props.node.resultUnloaded || !loadResult || untrack(loadState) !== "idle") return;
        setLoadState("loading");
        void loadResult(props.node).then((ok) => setLoadState(ok ? "idle" : "failed"));
    });
    // A failed read is retried the next time the row is opened, including one
    // that failed after the row was closed.
    createEffect(() => {
        if (!view.expanded() && loadState() === "failed") setLoadState("idle");
    });
    return { loadState, bodyPx: { value: 0 } };
}

export interface ToolPeek {
    node: NodePeek;
    timeText: Accessor<string | null>;
    estimateText: Accessor<string | null>;
    /** Raw-data existence check for showing the overlay at all, not the ticking memos (see there). */
    hasAnyContent: Accessor<boolean>;
}

/**
 * The peek tooltip's time and estimate lines (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md §2.3).
 * Real API-reported token/cost data doesn't exist per-tool-call today (see
 * SPEC_PER_NODE_TOKEN_ACCOUNTING_2026_08_03.md), so this is a client-side chars÷4 estimate, always
 * labeled "(est.)". The time text short-circuits before reading the shared 1 s ticker, so only
 * rows whose panel is showing subscribe to it (#2392).
 */
export function useToolPeek(props: ToolBlockProps, header: Accessor<ToolHeaderParts>): ToolPeek {
    // Drives the peek tooltip's live "time ago" text. Unconditional: Tooltip doesn't expose
    // whether it is open to its caller.
    const peekTick = useTick(1000);
    const node = useNodePeek();
    const timeText = createMemo(() => {
        if (!node.panelVisible()) return null;
        peekTick(); // re-run every second so "ago" stays live while hovered
        const ts = props.node.timestamp;
        if (ts == null) return null;
        return formatPeekTime(ts);
    });
    const estimateText = createMemo(() => {
        const text = JSON.stringify(props.node.params ?? {}) + JSON.stringify(props.node.result ?? "");
        const count = estimateTokenCount(text) + (props.node.resultUnloaded?.tokens ?? 0);
        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
    });
    // Deliberately NOT reading timeText()/estimateText() themselves: timeText is gated behind
    // panelVisible() (only resolves once the panel is showing), so using it here would make the
    // show-check circularly depend on hover having already begun.
    const hasAnyContent = createMemo(
        () => header().detail !== "" || props.node.timestamp != null || estimateText() != null
    );
    return { node, timeText, estimateText, hasAnyContent };
}
