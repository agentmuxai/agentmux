// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The agent suite: the agent pane mid-conversation, its question panel, the
// Stash, signing in, and the picker — Part 3 of
// docs/reports/REPORT_DOCS_SCREENSHOT_COVERAGE_PLAN_2026_10_10.md, spec §10.
// Run with the capture instance's data folder in the environment:
//
//   AGENTMUX_HOME_OVERRIDE=<capture home> node scripts/ui-screenshots/capture.mjs --suite agent --port N
//
// setup() makes the instance's Claude Code the scripted stand-in
// (fake-claude/), so conversations come from a real srv session with no
// account, model or network: the replies are fake-claude/conversation.json, a
// small change to the acme-web demo project. It adds a demo account
// (dev@acme.example) with a placeholder credential, and agents for the
// picker's My Agents.
//
// A conversation shot creates its agent through the picker's Create new agent
// form in a window tab of its own, as a user would, so the agent starts with
// no history. The form leaves the agent unbound, so the shot clicks the pane's
// "Bind: dev@acme.example" for the demo account. Cleanup closes the tab, which
// stops the agent, and deletes it.

import { mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { installFakeClaude } from "./install-fake-claude.mjs";
import { closeTab, openNewTab, visibleCenter } from "./widget-shots.mjs";

/** The agent pane at a size that reads well in a docs column. */
const VIEWPORT = { width: 960, height: 760, scale: 1 };

export const DEMO_ACCOUNT = { id: "6f1d2c3b-0a9e-4c7d-8b5a-acme00000001", email: "dev@acme.example" };
/** Agents kept for the picker's My Agents, each with one short session. */
export const PICKER_AGENTS = ["api-tests", "docs-writer", "checkout-service"];
/** The agent a conversation shot creates, and deletes afterwards. */
export const SHOT_AGENT = "web-frontend";

/** The conversation's messages, matched by fake-claude/conversation.json. */
const MESSAGES = {
    fix: 'The cart badge says "1 items" when there is one item. Can you fix it and add a test?',
    total: "Can you show the cart total in the header too?",
    where: "Where should it go?",
    hello: "Hello!",
};

const PICKER = ".agent-picker";
const COMPOSER = "textarea.agent-input";
const RUNTIME_SELECT = '[data-testid="create-from-template-runtime-select"]';

function captureHome() {
    const home = process.env.AGENTMUX_HOME_OVERRIDE;
    if (!home) throw new Error("set AGENTMUX_HOME_OVERRIDE to the capture instance's data folder");
    const norm = (p) => resolve(p).replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
    if (norm(home) === norm(join(homedir(), ".agentmux"))) throw new Error("AGENTMUX_HOME_OVERRIDE is this machine's own ~/.agentmux");
    return home;
}

const credentialsFile = () => join(captureHome(), "demo-accounts", "claude", ".credentials.json");

async function waitFor(session, expression, what, timeoutMs = 20000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        if (await session.evaluate(expression)) return;
        await session.wait(250);
    }
    throw new Error(`timed out waiting for ${what}`);
}

/** In the page: the user-owned agent definitions named in `names`. */
const agentsNamed = (names) => `(async () => {
    const r = await TabRpcClient.rpcCall("listagents", {});
    return (r.agents ?? r).filter((d) => !d.is_seeded && ${JSON.stringify(names)}.includes(d.name)).map((d) => ({ id: d.id, name: d.name }));
})()`;

/** Deletes the bundles named after agent `name` ("<name> — ABF", made when
 *  an agent is created) that no agent uses: deleting an agent leaves its
 *  bundle behind. */
async function deleteOrphanBundles(session, name) {
    await session.evaluate(`(async () => {
        const r = await TabRpcClient.rpcCall("listagents", {});
        const used = new Set((r.agents ?? r).map((d) => d.memory_id).filter(Boolean));
        const m = await TabRpcClient.rpcCall("listmemories", {});
        const prefix = ${JSON.stringify(`${name} — ABF`)};
        for (const b of m.memories ?? m) {
            if ((b.name === prefix || b.name.startsWith(prefix + " (")) && !used.has(b.id)) {
                await RpcApi.DeleteBundleCommand(TabRpcClient, { id: b.id });
            }
        }
    })()`);
}

/** Deletes the user-owned agent named `name`, if there is one, and its bundle. */
async function deleteAgent(session, name) {
    for (const d of await session.evaluate(agentsNamed([name]))) {
        await session.evaluate(`RpcApi.DeleteAgentDefinitionCommand(TabRpcClient, { id: ${JSON.stringify(d.id)} })`);
    }
    await deleteOrphanBundles(session, name);
}

/** The visible pane holding `selector`, as a selector by block id, or null. */
const visiblePaneWith = (selector) => `(() => {
    const el = [...document.querySelectorAll(${JSON.stringify(`.pane-stack:has(${selector})`)})].find((p) => {
        const r = p.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) return false;
        const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
        return hit && p.contains(hit);
    });
    const id = el?.querySelector('[data-blockid]')?.getAttribute('data-blockid');
    return id ? '.pane-stack:has([data-blockid="' + id + '"])' : null;
})()`;

/** True when a pane on screen (hit test at its centre) shows `text`. Every
 *  window tab's panes stay in the page, so the first match may be hidden. */
const visibleText = (text) => `[...document.querySelectorAll(".pane-stack")].some((p) => {
    const r = p.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    return !!hit && p.contains(hit) && p.innerText.includes(${JSON.stringify(text)});
})`;

const paneText = (pane, text) => `!!document.querySelector(${JSON.stringify(pane)})?.innerText.includes(${JSON.stringify(text)})`;

async function maximize(session, pane) {
    const fills = `(() => { const r = document.querySelector(${JSON.stringify(pane)})?.getBoundingClientRect(); return !!r && r.width > window.innerWidth * 0.95; })()`;
    for (let attempt = 0; attempt < 2; attempt++) {
        if (await session.evaluate(fills)) return;
        await session.clickSelector(`${pane} .block-frame-magnify[title="Maximize"]`);
        for (let i = 0; i < 12; i++) {
            await session.wait(250);
            if (await session.evaluate(fills)) return;
        }
    }
    throw new Error("the pane didn't maximize");
}

/** A turn in progress: the working row (not the "Worked" summary) is shown. */
const working = (pane) =>
    `!!document.querySelector(${JSON.stringify(`${pane} .agent-working-row:not(.agent-working-row--worked)`)})`;

/** In the new tab, maximizes its picker pane and returns it. */
async function pickerPane(session) {
    let pane = null;
    for (let i = 0; i < 40 && !pane; i++) {
        pane = await session.evaluate(visiblePaneWith(PICKER));
        if (!pane) await session.wait(250);
    }
    if (!pane) throw new Error("the new tab has no agent picker on screen");
    await maximize(session, pane);
    return pane;
}

/** Opens the picker's Create new agent form for Claude Code. */
async function openCreateForm(session) {
    const pane = await pickerPane(session);
    // The harness cards reorder as their install checks come back, so a click
    // can land on another card: wait for the grid to hold still, and check
    // which form opened.
    const cardAt = `(() => {
        const el = [...document.querySelectorAll(${JSON.stringify(`${pane} ${PICKER} *`)})].find((e) => e.children.length === 0 && e.offsetParent && e.textContent.trim() === "Claude Code");
        if (!el) return null;
        // Below My Agents, the card grid can be out of view.
        el.scrollIntoView({ block: "center" });
        const r = el.getBoundingClientRect();
        return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) };
    })()`;
    for (let attempt = 0; attempt < 4; attempt++) {
        let last = null;
        for (let i = 0; i < 20; i++) {
            await session.wait(250);
            const now = await session.evaluate(cardAt);
            if (now && last && now.x === last.x && now.y === last.y) break;
            last = now;
        }
        if (!last) throw new Error("no Claude Code card in the picker");
        await session.clickAt(last);
        for (let i = 0; i < 12; i++) {
            await session.wait(250);
            if (await session.evaluate(paneText(pane, "Create new agent from Claude"))) return pane;
            if (await session.evaluate(paneText(pane, "Create new agent from"))) break;
        }
        // Another harness's form, or none: back out and try again.
        const cancel = await session.evaluate(visibleCenter(`${pane} [data-modal-dismiss]`));
        if (cancel) await session.clickAt(cancel);
        else await session.pressKey("Escape");
        await session.wait(400);
    }
    throw new Error("the Create new agent form for Claude Code didn't open");
}

/** Creates agent `name` from the Claude Code template, on this computer, and
 *  waits for it to launch; returns its pane, maximized. `settled` false skips
 *  waiting for a signed-in, idle agent (for an agent that can't start). */
async function createAgent(session, name, { settled = true } = {}) {
    const pane = await openCreateForm(session);
    await session.clickSelector(`${pane} .agent-new-bundle-modal-input`);
    await session.typeText(name);
    // Docker on the capturing machine makes the form default to a container.
    await session.evaluate(`(() => {
        const s = document.querySelector(${JSON.stringify(`${pane} ${RUNTIME_SELECT}`)});
        const sel = s?.tagName === "SELECT" ? s : s?.querySelector("select");
        if (!sel) return false;
        sel.value = "host";
        sel.dispatchEvent(new Event("change", { bubbles: true }));
        return true;
    })()`);
    await session.wait(300);
    const create = await session.evaluate(`(() => {
        const b = [...document.querySelectorAll(${JSON.stringify(`${pane} button`)})].find((e) => e.offsetParent && e.innerText.trim() === "Create" && !e.disabled);
        if (!b) return null;
        const r = b.getBoundingClientRect();
        return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    })()`);
    if (!create) throw new Error("the form's Create button isn't enabled");
    await session.clickAt(create);
    let agentPane = null;
    for (let i = 0; i < 60 && !agentPane; i++) {
        await session.wait(250);
        agentPane = await session.evaluate(visiblePaneWith(COMPOSER));
    }
    if (!agentPane) throw new Error(`${name}'s pane didn't open`);
    await maximize(session, agentPane);
    if (settled) {
        // The form doesn't bind the account itself: the pane offers
        // "Bind: dev@acme.example", as it does for any unbound agent when an
        // account exists.
        await waitFor(
            session,
            `${paneText(agentPane, "Logged in")} || [...document.querySelectorAll(${JSON.stringify(`${agentPane} button`)})].some((b) => b.offsetParent && b.innerText.includes("Bind:"))`,
            `${name} to sign in or offer to bind the account`,
            30000
        );
        // A click while the row is still settling can miss; try a few times.
        for (let attempt = 0; attempt < 4 && !(await session.evaluate(paneText(agentPane, "Logged in"))); attempt++) {
            await session.wait(600);
            const bindAt = await session.evaluate(`(() => {
                const b = [...document.querySelectorAll(${JSON.stringify(`${agentPane} button`)})].find((e) => e.offsetParent && e.innerText.includes("Bind:"));
                if (!b) return null;
                const r = b.getBoundingClientRect();
                return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
            })()`);
            if (bindAt) await session.clickAt(bindAt);
            for (let i = 0; i < 16 && !(await session.evaluate(paneText(agentPane, "Logged in"))); i++) await session.wait(250);
        }
        await waitFor(session, paneText(agentPane, "Logged in"), `${name} to sign in`, 15000);
        // The launch runs AgentMux's own context turn first; wait it out.
        await session.wait(2000);
        await waitFor(session, `!(${working(agentPane)})`, "the launch turn to end", 30000);
        // Clear what the launch left in the pane (the sign-in notice), as
        // /clear does for a user: the view only, not the session.
        await send(session, agentPane, "/clear");
        await session.wait(800);
    }
    return agentPane;
}

async function send(session, pane, text) {
    await session.clickSelector(`${pane} ${COMPOSER}`);
    await session.typeText(text);
    await session.wait(200);
    await session.pressKey("Enter");
}

/** Waits for the turn just sent to start, then to finish. (The launch turn's
 *  "Worked" summary is already shown, so finishing alone isn't enough.) */
async function turnDone(session, pane) {
    await waitFor(session, working(pane), "the turn to start", 15000);
    await waitFor(session, `!!document.querySelector(${JSON.stringify(`${pane} .agent-working-row--worked`)}) && !(${working(pane)})`, "the turn to finish", 45000);
}

/** Installs the stand-in, adds the demo account, makes sure each picker agent
 *  has had a session, and reloads the page so the picker sees Claude Code as
 *  installed. */
export async function setup(session) {
    const home = captureHome();
    installFakeClaude(home);
    const accountDir = join(home, "demo-accounts", "claude");
    mkdirSync(accountDir, { recursive: true });
    // srv checks for a credential file before it asks the CLI; this one only
    // ever reaches the stand-in.
    writeFileSync(
        credentialsFile(),
        JSON.stringify({ claudeAiOauth: { accessToken: "demo-placeholder", refreshToken: "demo-placeholder", expiresAt: 4102444800000, scopes: ["user:inference"] } }) + "\n"
    );
    await session.evaluate(`RpcApi.UpsertIdentityAccountCommand(TabRpcClient, { id: ${JSON.stringify(DEMO_ACCOUNT.id)}, name: ${JSON.stringify(DEMO_ACCOUNT.email)},
        provider: "claude", kind: "oauth", display_name: ${JSON.stringify(DEMO_ACCOUNT.email)},
        secret_ref: { backend: "oauth_config_dir", dir: ${JSON.stringify(accountDir.replace(/\\/g, "/"))} } })`);
    await session.evaluate("location.reload()");
    await session.wait(1500);
    await waitFor(session, `!!(window.RpcApi && document.querySelector(".window-header"))`, "the page to reload");
    await session.wait(1500);

    // A shot's agent left behind by an interrupted run, and bundles left by
    // agents deleted before.
    await deleteAgent(session, SHOT_AGENT);
    for (const name of PICKER_AGENTS) await deleteOrphanBundles(session, name);
    // My Agents lists the agents that have had a session started by the
    // picker; each picker agent that hasn't is created afresh and says hello.
    const listed = await session.evaluate(`(async () => {
        const { rows } = await RpcApi.ListRecentSessionsCommand(TabRpcClient, { limit: 200 });
        return rows.map((r) => r.definition_name);
    })()`);
    for (const name of PICKER_AGENTS.filter((n) => !listed.includes(n))) {
        await deleteAgent(session, name);
        const tab = { tabId: null };
        await session.setViewport(VIEWPORT);
        await openNewTab(session, tab);
        const pane = await createAgent(session, name);
        await send(session, pane, MESSAGES.hello);
        await turnDone(session, pane);
        await session.pressKey("Escape");
        await closeTab(session, tab.tabId);
        await session.clearViewport();
        console.log(`  created ${name}`);
    }
}

/** A shot in a new tab of its own: `inTab(session)` returns the selector to crop
 *  to (or null for the whole window). Cleanup closes the tab, then runs
 *  `after` (deleting the shot's agent, say). */
function agentShot(def) {
    const tab = { tabId: null };
    let crop = null;
    return {
        containsWorkspaceData: "review",
        retries: 1,
        settleMs: 600,
        ...def,
        prep: async (session) => {
            await session.setViewport(VIEWPORT);
            await session.pressKey("Escape");
            tab.tabId = null;
            await openNewTab(session, tab);
            crop = await def.inTab(session);
        },
        selector: () => crop,
        cleanup: async (session) => {
            await session.pressKey("Escape");
            if (tab.tabId) await closeTab(session, tab.tabId);
            tab.tabId = null;
            await session.clearViewport();
            if (def.after) await def.after(session);
        },
    };
}

/** A shot of a new agent's conversation: creates it, runs `inPane`, and
 *  deletes it afterwards. */
function conversationShot(def) {
    return agentShot({
        ...def,
        inTab: async (session) => {
            await deleteAgent(session, SHOT_AGENT);
            const pane = await createAgent(session, SHOT_AGENT, { settled: def.settled ?? true });
            return (await def.inPane(session, pane)) ?? pane;
        },
        after: async (session) => {
            // The agent stops when its tab closes; give srv a moment.
            await session.wait(800);
            await deleteAgent(session, SHOT_AGENT);
        },
    });
}

export const shots = [
    agentShot({
        id: "agent-picker",
        title: "Agent picker with your agents",
        description: "The agent picker: your agents under My Agents, and a card per harness to create a new one.",
        inTab: (session) => pickerPane(session),
        verify: async (session) =>
            (await session.evaluate(`${visibleText(PICKER_AGENTS[0])} && ${visibleText(PICKER_AGENTS[1])}`)) || "My Agents doesn't list the demo agents",
    }),
    agentShot({
        id: "agent-create",
        title: "Create new agent",
        description: "Creating an agent from the Claude Code harness: its name, runtime, model, endpoint and bundles.",
        inTab: (session) => openCreateForm(session),
        verify: async (session) => (await session.evaluate(visibleText("Create new agent"))) || "the form isn't open",
    }),
    conversationShot({
        id: "agent-sign-in",
        title: "A new agent that isn't signed in",
        description: "A new agent with no account bound yet: log in, log in from a terminal, or bind an account you already have.",
        settled: false,
        inPane: async (session, pane) => {
            await waitFor(session, paneText(pane, "Not signed in"), "the Not signed in row", 30000);
        },
        verify: async (session) => (await session.evaluate(visibleText("Bind: dev@acme.example"))) || "no Bind button",
    }),
    conversationShot({
        id: "agent-conversation",
        title: "A finished turn",
        description: "The end of a turn: the test run, the agent's summary, and how long the turn took and what it cost.",
        inPane: async (session, pane) => {
            await send(session, pane, MESSAGES.fix);
            await turnDone(session, pane);
        },
        verify: async (session) => (await session.evaluate(visibleText("Both tests pass"))) || "the summary isn't shown",
    }),
    conversationShot({
        id: "agent-conversation-start",
        title: "A turn's tool calls",
        description: "The start of the same turn: your message, the agent's thinking, a file read, its plan, and an edit with its diff.",
        inPane: async (session, pane) => {
            await send(session, pane, MESSAGES.fix);
            await turnDone(session, pane);
            // A wheel scroll, as a user's: setting scrollTop is undone by the
            // pane keeping itself pinned to the bottom.
            const region = await session.evaluate(visibleCenter(`${pane} .agent-document-scroll-region`));
            if (!region) throw new Error("no scroll region in the pane");
            for (let i = 0; i < 12; i++) {
                await session.send("Input.dispatchMouseEvent", { type: "mouseWheel", x: region.x, y: region.y, deltaX: 0, deltaY: -1500 });
                await session.wait(80);
            }
            await session.wait(800);
        },
        verify: async (session) => (await session.evaluate(visibleText("Let me look at the cart badge"))) || "the start of the turn isn't shown",
    }),
    conversationShot({
        id: "agent-working",
        title: "An agent at work",
        description: "A turn in progress: a subagent's result, a file read, the reply streaming in, and the working timer.",
        inPane: async (session, pane) => {
            await send(session, pane, MESSAGES.total);
            await waitFor(session, paneText(pane, "To show the total in the header"), "the reply to stream", 30000);
        },
        // The working row, labelled with what the agent is doing.
        verify: ".agent-working-row--loading",
    }),
    conversationShot({
        id: "agent-question",
        title: "The agent asks a question",
        description: "A question from the agent, with its options; the recommended one first.",
        inPane: async (session, pane) => {
            await send(session, pane, MESSAGES.where);
            await waitFor(session, `!!document.querySelector(${JSON.stringify(`${pane} .agent-question-panel`)})`, "the question panel", 30000);
        },
        verify: ".agent-question-panel",
    }),
    conversationShot({
        id: "agent-stash",
        title: "The Stash",
        description: "The agent's Stash, from the backpack in its header.",
        inPane: async (session, pane) => {
            const stash = await session.evaluate(visibleCenter(`${pane} [title="Stash"]`));
            if (!stash) throw new Error("no Stash button in the pane's header");
            await session.clickAt(stash);
            await waitFor(session, `!!document.querySelector(".agent-stash-modal-panel")`, "the Stash");
        },
        verify: ".agent-stash-modal-panel",
    }),
];
