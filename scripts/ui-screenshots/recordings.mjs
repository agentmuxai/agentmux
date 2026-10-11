// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Clips for the docs, recorded by record.mjs — see
// docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §12. Each clip's
// `prep` sets up what it starts from (not recorded) and returns the selector
// to crop to; `run` is what's recorded, moving the drawn cursor and clicking;
// `cleanup` puts things back. Agent clips use the agent suite's scripted
// Claude Code and its setup (§10), so a clip needs no account.

import { MESSAGES, SHOT_AGENT, VIEWPORT as AGENT_VIEWPORT, createAgent, deleteAgent, setup as agentSetup, waitFor } from "./agent-shots.mjs";
import { closeTab, openNewTab } from "./widget-shots.mjs";

export const setup = agentSetup;

/** The Stash's tabs, in order (frontend/app/view/agent/components/AgentStashModal.tsx). */
export const STASH_TABS = ["Accounts", "Personal Memory", "MCP Servers", "Skills", "Bundles", "Registration", "Devices"];

/** Types `text` a character at a time, as a person would, into the focused field. */
async function typeSlowly(session, text, msPerChar = 28) {
    for (const ch of text) {
        await session.send("Input.insertText", { text: ch });
        await session.wait(msPerChar);
    }
}

/** A clip in a window tab of its own, with a fresh agent in it; the tab is
 *  closed and the agent deleted afterwards. `maximized: false` leaves the
 *  agent beside the tab's other panes (Swarm, Sysinfo), and the clip is of
 *  the whole window. */
function agentClip(def) {
    const tab = { tabId: null };
    const state = { pane: null };
    return {
        viewport: AGENT_VIEWPORT,
        ...def,
        prep: async (session) => {
            tab.tabId = null;
            await session.pressKey("Escape");
            await openNewTab(session, tab);
            await deleteAgent(session, SHOT_AGENT);
            const pane = await createAgent(session, SHOT_AGENT, { maximized: def.maximized ?? true });
            state.pane = pane;
            return def.maximized === false ? null : pane;
        },
        // run gets the agent's pane whether or not the clip is cropped to it.
        run: (session, cursor) => def.run(session, cursor, state.pane),
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
        id: "swarm-follows-agent",
        title: "Swarm following an agent",
        description: "Sending an agent a message: Swarm shows it working, with each step, while the reply streams in.",
        viewport: { width: 1280, height: 800, scale: 1 },
        maximized: false,
        run: async (session, cursor, pane) => {
            await cursor.clickSelector(`${pane} textarea.agent-input`);
            await typeSlowly(session, MESSAGES.total);
            await session.wait(400);
            await session.pressKey("Enter");
            await waitFor(session, `!!document.querySelector(${JSON.stringify(pane)})?.innerText.includes("To show the total in the header")`, "the reply to stream", 30000);
            await session.wait(2500);
        },
    }),
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
