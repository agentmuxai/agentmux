// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The memory suite: Memory (Global, Personal, Skills, Bundles), Connectors
// (Accounts, MCP servers) and a widget waiting for approval — Part 4 of
// docs/reports/REPORT_DOCS_SCREENSHOT_COVERAGE_PLAN_2026_10_10.md, spec §11.
//
//   node scripts/ui-screenshots/capture.mjs --suite memory --port N
//
// setup() adds demo data through the page's own RPCs, each only if it isn't
// there yet: Global Memory entries, two bundles (one with per-provider
// instructions), made-up accounts whose secrets are only environment variable
// names, Personal memory files for an agent, and the docs' sample "notes"
// widget, installed but not approved. Skills and MCP servers are shown as the
// app ships them. Remotes isn't captured: it lists the hosts in the OS user's
// ~/.ssh/config (spec §8.3).
//
// Each shot opens the pane it needs in a window tab of its own, maximized,
// and closes the tab afterwards.

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { closeTab, openNewTab } from "./widget-shots.mjs";

const VIEWPORT = { width: 1100, height: 760, scale: 1 };

const SAMPLE_WIDGET = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "docs", "examples", "widgets", "notes");
export const SAMPLE_WIDGET_ID = "agentmux.notes";

export const GLOBAL_ENTRIES = [
    {
        name: "Coding standards",
        instructions:
            "# Coding standards\n\n- TypeScript strict mode; no `any` without a comment saying why.\n- Components in `src/components/`, one per file.\n- Every bug fix comes with a test.\n",
    },
    {
        name: "Release checklist",
        instructions: "# Release checklist\n\n1. Update CHANGELOG.md.\n2. `npm test` and `npm run build` pass.\n3. Tag the release `vX.Y.Z`.\n",
    },
    {
        name: "Acme glossary",
        instructions: "# Acme glossary\n\n- **Storefront**: the customer-facing web app (acme-web).\n- **Cart badge**: the item count in the header.\n",
    },
];

export const BUNDLES = [
    {
        name: "acme-frontend",
        description: "Working on the acme-web storefront.",
        provider: "claude",
        instructions: "You work on acme-web, a SolidJS storefront. Follow the Coding standards in Global Memory and keep components small.",
        instructions_by_provider: {
            codex: "You work on acme-web, a SolidJS storefront. Run `npm test` before you report a change as done.",
        },
    },
    {
        name: "release-helper",
        description: "Prepares a release: changelog, version and tag.",
        provider: "",
        instructions: "When asked to prepare a release, follow the Release checklist in Global Memory step by step and report each step.",
        instructions_by_provider: {},
    },
];

/** Made-up accounts: their secrets are environment variable names, never values. */
export const ACCOUNTS = [
    { name: "acme-ci", provider: "github", kind: "pat", display_name: "acme-ci-bot", env_var: "GITHUB_TOKEN" },
    { name: "acme-openai", provider: "openai", kind: "api_key", display_name: "acme (OpenAI)", env_var: "OPENAI_API_KEY" },
];

/** Personal memory files, written for this agent (from the agent suite's picker agents). */
export const PERSONAL_AGENT = "api-tests";
export const PERSONAL_FILES = {
    "project_acme_web.md": "---\nname: project-acme-web\ndescription: what acme-web is and where its tests live\n---\n\nacme-web is the Acme storefront (SolidJS). Tests are in `tests/`, run with `npm test`.\n",
    "feedback_test_style.md": "---\nname: feedback-test-style\ndescription: how the team wants tests written\n---\n\nOne behaviour per test, named for what it checks (\"says item for one\").\n",
};

const MEMORY = ".armory-view";
const tab = (label) => `button.ui-tab[role=tab][aria-label=${JSON.stringify(label)}]`;

async function waitFor(session, expression, what, timeoutMs = 15000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        if (await session.evaluate(expression)) return;
        await session.wait(250);
    }
    throw new Error(`timed out waiting for ${what}`);
}

/** Adds the demo data that isn't there yet. */
export async function setup(session) {
    const done = await session.evaluate(`(async () => {
        const out = [];
        const entries = ${JSON.stringify(GLOBAL_ENTRIES)};
        const bundles = ${JSON.stringify(BUNDLES)};
        const accounts = ${JSON.stringify(ACCOUNTS)};
        const m = await TabRpcClient.rpcCall("listmemories", {});
        const memories = m.memories ?? m;
        for (const e of entries) {
            if (memories.some((x) => x.is_global && x.name === e.name)) continue;
            await RpcApi.UpsertBundleCommand(TabRpcClient, { id: "", name: e.name, is_global: true, instructions: e.instructions });
            out.push("global: " + e.name);
        }
        for (const b of bundles) {
            if (memories.some((x) => !x.is_global && x.name === b.name)) continue;
            await RpcApi.UpsertBundleCommand(TabRpcClient, { id: "", name: b.name, description: b.description, is_global: false,
                provider: b.provider, model: "", instructions: b.instructions, instructions_by_provider: JSON.stringify(b.instructions_by_provider),
                context_files: "[]", mcp_servers: "[]", skills: "[]" });
            out.push("bundle: " + b.name);
        }
        const existing = await TabRpcClient.rpcCall("listidentityaccounts", {});
        for (const a of accounts) {
            if ((existing.accounts ?? existing).some((x) => x.name === a.name)) continue;
            await RpcApi.UpsertIdentityAccountCommand(TabRpcClient, { id: "", name: a.name, provider: a.provider, kind: a.kind,
                display_name: a.display_name, secret_ref: { backend: "env", env_var: a.env_var }, status: "valid" });
            out.push("account: " + a.name);
        }
        const r = await TabRpcClient.rpcCall("listagents", {});
        const agent = (r.agents ?? r).find((d) => !d.is_seeded && d.name === ${JSON.stringify(PERSONAL_AGENT)});
        if (agent) {
            for (const [filename, content] of Object.entries(${JSON.stringify(PERSONAL_FILES)})) {
                await TabRpcClient.rpcCall("agent:memory:write_file", { agent_id: agent.id, filename, content });
            }
            out.push("personal memory: " + agent.name);
        } else {
            out.push("no " + ${JSON.stringify(PERSONAL_AGENT)} + " agent: run the agent suite first for Personal memory");
        }
        return out.join(", ");
    })()`);
    console.log(`  ${done || "demo data already there"}`);
    // A copy approved before keeps its approval through a reinstall, and then
    // has no Approve button: uninstall it (which drops the approval) first.
    // An install that fails fails setup (Codex on #4669).
    const state = await session.evaluate(`(async () => {
        const id = ${JSON.stringify(SAMPLE_WIDGET_ID)};
        const before = await RpcApi.WidgetsListCommand(TabRpcClient);
        if (before.packages.some((p) => p.id === id)) await RpcApi.WidgetsUninstallCommand(TabRpcClient, { id });
        await RpcApi.WidgetsInstallCommand(TabRpcClient, { path: ${JSON.stringify(SAMPLE_WIDGET)}, replace: true });
        const after = await RpcApi.WidgetsListCommand(TabRpcClient);
        return after.packages.find((p) => p.id === id)?.state ?? "missing";
    })()`);
    if (state !== "needs_approval") throw new Error(`the sample widget is "${state}" after installing, not waiting for approval`);
    console.log("  widget: installed, waiting for approval");
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

/** Opens the hamburger menu's `item` (Memory, Connectors or Settings) in the
 *  current tab, maximized, and returns its pane. */
async function openFromMenu(session, item, marker) {
    await session.clickSelector(".hamburger-btn");
    await session.wait(300);
    await session.clickText(".menu", item);
    let pane = null;
    for (let i = 0; i < 40 && !pane; i++) {
        await session.wait(250);
        pane = await session.evaluate(visiblePaneWith(marker));
    }
    if (!pane) throw new Error(`${item} didn't open`);
    await maximize(session, pane);
    return pane;
}

async function selectTab(session, pane, label) {
    await session.clickSelector(`${pane} ${tab(label)}`);
    await waitFor(session, `document.querySelector(${JSON.stringify(`${pane} ${tab(label)}`)})?.getAttribute("aria-selected") === "true"`, `the ${label} tab`);
    await session.wait(500);
}

/** Clicks the first visible element under `pane` matching `selector` whose text
 *  includes `text` (any, when text is omitted). */
async function clickIn(session, pane, selector, text) {
    const pt = await session.evaluate(`(() => {
        const el = [...document.querySelectorAll(${JSON.stringify(`${pane} ${selector}`)})].find((e) => e.offsetParent && (${JSON.stringify(text ?? "")} === "" || e.innerText.includes(${JSON.stringify(text ?? "")})));
        if (!el) return null;
        el.scrollIntoView({ block: "center" });
        const r = el.getBoundingClientRect();
        return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    })()`);
    if (!pt) throw new Error(`nothing matches ${selector}${text ? ` with "${text}"` : ""}`);
    await session.clickAt(pt);
    await session.wait(500);
}

/** True when a visible pane shows `text`. */
const visibleText = (text) => `[...document.querySelectorAll(".pane-stack")].some((p) => {
    const r = p.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    return !!hit && p.contains(hit) && p.innerText.includes(${JSON.stringify(text)});
})`;
const showsText = (text, why) => async (session) => (await session.evaluate(visibleText(text))) || why;

/** A shot of a pane opened from the hamburger menu in a tab of its own;
 *  `inPane(session, pane)` takes it to the state shown. */
function paneShot({ item, marker, inPane, ...def }) {
    const t = { tabId: null };
    let crop = null;
    return {
        containsWorkspaceData: "review",
        retries: 1,
        settleMs: 600,
        ...def,
        prep: async (session) => {
            await session.setViewport(VIEWPORT);
            await session.pressKey("Escape");
            t.tabId = null;
            await openNewTab(session, t);
            const pane = await openFromMenu(session, item, marker);
            if (inPane) await inPane(session, pane);
            crop = pane;
        },
        selector: () => crop,
        cleanup: async (session) => {
            await session.pressKey("Escape");
            if (t.tabId) await closeTab(session, t.tabId);
            t.tabId = null;
            await session.clearViewport();
        },
    };
}

const memoryShot = (def) => paneShot({ item: "Memory", marker: MEMORY, ...def });
const connectorsShot = (def) => paneShot({ item: "Connectors", marker: MEMORY, ...def });

export const shots = [
    memoryShot({
        id: "memory-global",
        title: "Memory — Global",
        description: "Global Memory: the notes every agent gets at launch, AgentMux's own entries, the provider's CLAUDE.md, and the combined preview.",
        inPane: (s, p) => selectTab(s, p, "Global"),
        verify: showsText("Coding standards", "the demo entries aren't shown"),
    }),
    memoryShot({
        id: "memory-global-entry",
        title: "A Global Memory entry",
        description: "One Global Memory entry, opened.",
        inPane: async (s, p) => {
            await selectTab(s, p, "Global");
            await clickIn(s, p, '[data-testid="global-memory-tile"][data-kind="entry"]', "Coding standards");
        },
        verify: ".global-bundle-detail",
    }),
    memoryShot({
        id: "memory-global-new",
        title: "Add a Global Memory entry",
        description: "A new Global Memory entry: its name and text.",
        inPane: async (s, p) => {
            await selectTab(s, p, "Global");
            await clickIn(s, p, '[data-testid="global-memory-tile"][data-kind="add"]');
        },
        verify: showsText("New memory", "the New memory form isn't open"),
    }),
    memoryShot({
        id: "memory-personal",
        title: "Memory — Personal",
        description: "Personal memory: each agent's own notes, by agent.",
        inPane: (s, p) => selectTab(s, p, "Personal"),
        verify: ".memory-agent-card",
    }),
    memoryShot({
        id: "memory-personal-files",
        title: "An agent's Personal memory",
        description: "One agent's Personal memory files.",
        inPane: async (s, p) => {
            await selectTab(s, p, "Personal");
            await clickIn(s, p, ".memory-agent-card", PERSONAL_AGENT);
        },
        verify: ".memory-file-card",
    }),
    memoryShot({
        id: "memory-skills",
        title: "Memory — Skills",
        description: "Skills: the catalog of skills agents can be given.",
        inPane: (s, p) => selectTab(s, p, "Skills"),
        verify: ".agent-primitive-modal-list-item",
    }),
    memoryShot({
        id: "memory-skill-new",
        title: "A new skill",
        description: "Adding a skill to the catalog.",
        inPane: async (s, p) => {
            await selectTab(s, p, "Skills");
            await clickIn(s, p, ".agent-primitive-modal-new-btn", "New skill");
        },
        verify: ".agent-primitive-modal-form",
    }),
    memoryShot({
        id: "memory-bundles",
        title: "Memory — Bundles",
        description: "Bundles: reusable sets of instructions, skills and MCP servers for agents.",
        inPane: (s, p) => selectTab(s, p, "Bundles"),
        verify: showsText("acme-frontend", "the demo bundles aren't shown"),
    }),
    memoryShot({
        id: "memory-bundle-detail",
        title: "A bundle",
        description: "One bundle, opened: its description, scope, instructions and skills.",
        inPane: async (s, p) => {
            await selectTab(s, p, "Bundles");
            await clickIn(s, p, "li.bundle-view-list-item", "acme-frontend");
        },
        verify: ".bundle-view-readonly",
    }),
    memoryShot({
        id: "memory-bundle-edit",
        title: "Editing a bundle",
        description: "Editing a bundle, with instructions for one provider that replace the bundle's own.",
        inPane: async (s, p) => {
            await selectTab(s, p, "Bundles");
            await clickIn(s, p, "li.bundle-view-list-item", "acme-frontend");
            await clickIn(s, p, ".bundle-view-edit-btn");
        },
        verify: "form.bundle-view-form",
    }),
    connectorsShot({
        id: "connectors-accounts",
        title: "Connectors — Accounts",
        description: "Accounts: the provider and service accounts your agents use.",
        inPane: (s, p) => selectTab(s, p, "Accounts"),
        verify: showsText("acme-ci-bot", "the demo accounts aren't shown"),
    }),
    connectorsShot({
        id: "connectors-account-add",
        title: "Add an account",
        description: "Adding an account: its provider, kind and where its secret comes from.",
        inPane: async (s, p) => {
            await selectTab(s, p, "Accounts");
            await clickIn(s, p, ".identity-add-btn");
        },
        verify: ".identity-form",
    }),
    connectorsShot({
        id: "connectors-mcp",
        title: "Connectors — MCP servers",
        description: "MCP servers: the catalog of tool servers agents can be given.",
        inPane: (s, p) => selectTab(s, p, "MCP servers"),
        verify: ".agent-primitive-modal-list-item",
    }),
    connectorsShot({
        id: "connectors-mcp-new",
        title: "A new MCP server",
        description: "Adding an MCP server to the catalog.",
        inPane: async (s, p) => {
            await selectTab(s, p, "MCP servers");
            await clickIn(s, p, ".agent-primitive-modal-new-btn", "New MCP server");
        },
        verify: ".agent-primitive-modal-form",
    }),
    paneShot({
        id: "settings-widget-approval",
        title: "Approving a widget",
        description: "A widget waiting for approval in Settings → Widgets, with what it asks for.",
        item: "Settings",
        marker: ".settings-view-container",
        inPane: async (s, p) => {
            await selectTab(s, p, "Widgets");
            await waitFor(s, `!!document.querySelector(${JSON.stringify(`${p} .widget-row[data-widget-id="${SAMPLE_WIDGET_ID}"]`)})`, "the sample widget's row");
            await clickIn(s, p, `.widget-row[data-widget-id="${SAMPLE_WIDGET_ID}"] .widget-row-actions button`, "Approve");
        },
        verify: ".widget-approval",
    }),
];
