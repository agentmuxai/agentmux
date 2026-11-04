// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What an agent's Swarm row does with a press, a click and a key, and how its summary flashes on a change. */

import { createEffect, createSignal, onCleanup, untrack, type Accessor } from "solid-js";
import { openAgentInActiveTab } from "@/app/store/agent-runtime-live";
import { revealBlock } from "@/app/util/reveal-block";
import { scaledMs } from "@/app/motion/motion";
import { DURATION_EMPHASIS_MS } from "@/app/motion/tokens";
import { agentUidOfBlockMeta, isParkedMeta } from "./swarm-open-parked";
import type { AgentTreeNode } from "./swarm-model";

/** How long a changed summary keeps its flash class: the flash runs for
 *  --duration-emphasis, scaled, and the class stays 100 ms longer. */
const summaryFlashMs = () => scaledMs(DURATION_EMPHASIS_MS) + 100;

/** Whether the row's summary is flashing: set when it changes to a new one, not for the one the row mounts with. */
export function useSummaryFlash(node: Accessor<AgentTreeNode>): Accessor<boolean> {
    const [summaryFlash, setSummaryFlash] = createSignal(false);
    let flashTimer: ReturnType<typeof setTimeout> | undefined;
    let prevSummary: string | null = untrack(() => node().activitySummary); // don't flash existing summary on mount
    createEffect(() => {
        const summary = node().activitySummary;
        if (summary && summary !== prevSummary) {
            clearTimeout(flashTimer);
            setSummaryFlash(true);
            flashTimer = setTimeout(() => setSummaryFlash(false), summaryFlashMs());
        }
        prevSummary = summary;
    });
    onCleanup(() => clearTimeout(flashTimer));
    return summaryFlash;
}

/** Only controls INSIDE this card count: closest() also walks ancestors, and
 *  the Swarm root above every card has a tabindex, so an unbounded match would
 *  always hit it and skip the guard entirely. */
const pressedOnControl = (e: { target: EventTarget | null; currentTarget: EventTarget | null }) => {
    const control = (e.target as Element | null)?.closest("input, button, select, textarea, a[href], [tabindex]");
    const card = e.currentTarget as Element | null;
    return !!control && control !== card && !!card?.contains(control);
};

export type AgentRowActivation = {
    isParked: () => boolean;
    /** Why srv refused to open a parked agent's pane, or "". */
    openRefused: Accessor<string>;
    onClick: (e: MouseEvent) => void;
    onPointerDown: (e: PointerEvent) => void;
    onKeyDown: (e: KeyboardEvent) => void;
    onMouseDown: (e: MouseEvent) => void;
};

/**
 * The card's handlers. A click is lost when the pressed element is replaced before the button comes
 * up, so a mouse press selects the agent at once and the click that follows does nothing more.
 * Touch and pen keep to click: a touch can turn into a scroll. Keyboard: Enter or Space on the focused card.
 */
export function useAgentRowActivation(node: Accessor<AgentTreeNode>, blockMeta: Accessor<Block["meta"] | undefined>): AgentRowActivation {
    let pressSelected = false;
    const selectAgent = () => {
        const blockId = untrack(() => node().blockId);
        if (blockId) void revealBlock(blockId, { source: "swarm" });
    };
    // An agent that runs with no pane has nothing to focus: the click opens its pane in the active tab,
    // attached to the running session. One open at a time (a double click is one open), and the parked
    // mark is read when the click lands, so a pane that appeared meanwhile (another window) is focused.
    const isParked = () => isParkedMeta(blockMeta());
    const [opening, setOpening] = createSignal(false);
    const [openRefused, setOpenRefused] = createSignal("");
    const openParked = async () => {
        if (opening()) return;
        const agentId = agentUidOfBlockMeta(untrack(blockMeta));
        if (!agentId) {
            selectAgent();
            return;
        }
        setOpenRefused("");
        setOpening(true);
        try {
            const why = await openAgentInActiveTab({ agent_id: agentId, name: untrack(() => node().agentName) });
            if (why) setOpenRefused(why);
        } finally {
            setOpening(false);
        }
    };
    const activateAgent = () => {
        if (untrack(isParked)) void openParked();
        else selectAgent();
    };
    return {
        isParked,
        openRefused,
        // Selecting an agent focuses ITS pane (switching tabs if it's a background tab in a multi-tab
        // pane), not the Swarm pane itself. stopPropagation is required: without it this click bubbles
        // to the Swarm block's own generic click-to-focus and immediately re-steals focus back to Swarm.
        // Clicking the header/empty area (not a row) still reaches that handler unchanged.
        onClick: (e) => {
            e.stopPropagation();
            // A mouse press already selected the agent (onPointerDown below).
            if (pressSelected) {
                pressSelected = false;
                return;
            }
            // A control inside the card (a chip, a link) does its own thing; it does not open the pane.
            if (untrack(isParked) && pressedOnControl(e)) return;
            activateAgent();
        },
        // ...and the Swarm pane must not flash selected first. The Swarm root is focusable
        // (tabIndex={-1}), so a press anywhere inside it moves DOM focus there on MOUSEDOWN, before the
        // click runs; that focusin bubbles to PaneChrome / BlockFrame, which select the Swarm pane, and
        // the click then hands focus to the agent's pane: a highlight-unhighlight blink. Cancelling
        // mousedown's default keeps focus where it is until the click picks the agent's pane. Only the
        // agent card does this: the header and empty space still focus (select) the Swarm pane.
        // Focusable controls inside the card (the fleet checkbox) are left alone: they need real focus
        // (Space toggles a focused checkbox), and pressing one is a Swarm action, not "select this agent".
        onPointerDown: (e) => {
            pressSelected = false;
            if (e.pointerType !== "mouse" || e.button !== 0 || pressedOnControl(e)) return;
            if ((e.target as Element | null)?.closest(".swarm-agent-expand-icon")) return;
            pressSelected = true;
            activateAgent();
        },
        onKeyDown: (e) => {
            if (e.target !== e.currentTarget || (e.key !== "Enter" && e.key !== " ")) return;
            e.preventDefault();
            e.stopPropagation();
            activateAgent();
        },
        onMouseDown: (e) => {
            if (pressedOnControl(e)) return;
            e.preventDefault();
        },
    };
}
