// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Clips for the docs, recorded by record.mjs — see
// docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §12. Each clip's
// `prep` sets up what it starts from (not recorded) and returns the selector
// to crop to; `run` is what's recorded, moving the drawn cursor and clicking;
// `cleanup` puts things back. Agent clips use the agent suite's scripted
// Claude Code and its setup (§10), so a clip needs no account.

import { SHOT_AGENT, VIEWPORT as AGENT_VIEWPORT, createAgent, deleteAgent, setup as agentSetup, waitFor } from "./agent-shots.mjs";
import { closeTab, openNewTab } from "./widget-shots.mjs";

export const setup = agentSetup;

/** The Stash's tabs, in order (frontend/app/view/agent/components/AgentStashModal.tsx). */
export const STASH_TABS = ["Accounts", "Personal Memory", "MCP Servers", "Skills", "Bundles", "Registration", "Devices"];

/** A clip in a window tab of its own, with a fresh agent in it; the tab is
 *  closed and the agent deleted afterwards. */
function agentClip(def) {
    const tab = { tabId: null };
    return {
        viewport: AGENT_VIEWPORT,
        ...def,
        prep: async (session) => {
            tab.tabId = null;
            await session.pressKey("Escape");
            await openNewTab(session, tab);
            await deleteAgent(session, SHOT_AGENT);
            return createAgent(session, SHOT_AGENT);
        },
        cleanup: async (session) => {
            await session.pressKey("Escape");
            if (tab.tabId) await closeTab(session, tab.tabId);
            tab.tabId = null;
            await session.wait(800);
            await deleteAgent(session, SHOT_AGENT);
        },
    };
}

export const recordings = [
    agentClip({
        id: "stash-tour",
        title: "The Stash",
        description: "Opening an agent's Stash from the backpack in its header, and going through its tabs.",
        run: async (session, cursor, pane) => {
            await cursor.clickSelector(`${pane} [title="Stash"]`);
            await waitFor(session, `!!document.querySelector(".agent-stash-modal-panel")`, "the Stash");
            await session.wait(1400);
            for (const label of STASH_TABS.slice(1)) {
                await cursor.clickSelector(`${pane} .agent-stash-modal-tabs button`, label);
                await session.wait(1300);
            }
            await cursor.clickSelector(`${pane} .agent-stash-modal-tabs button`, STASH_TABS[0]);
            await session.wait(1000);
        },
    }),
];
