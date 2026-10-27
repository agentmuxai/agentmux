#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Gives a `task dev` build's own (isolated) home an agent that works without an account, a model
// or a network: the scripted Claude stand-in (scripts/ui-screenshots/fake-claude) as its Claude
// Code, a placeholder account, and one or more agents linked to that account. Run it once the dev
// window is up; it is safe to run again.
//
//   node scripts/dev-seed.mjs [--port 9223] [--home <dir>] [--agents 1] [--name fake-agent] [--script <conversation.json>] [--no-open]
//
// --home defaults to AGENTMUX_HOME_OVERRIDE, else the one folder under ./.dev-home. It refuses the
// real ~/.agentmux, and a window that is not a dev build (one served by Vite on localhost).

import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { homeProblem, installFakeClaude } from "./ui-screenshots/install-fake-claude.mjs";
import { CdpSession, findMainTarget } from "./ui-screenshots/lib/session.mjs";
import { HOME_DIR_NAME } from "./dev-home.mjs";

export const DEMO_ACCOUNT = { id: "6f1d2c3b-0a9e-4c7d-8b5a-devseed00001", email: "dev@example.invalid" };
const PLACEHOLDER = { claudeAiOauth: { accessToken: "demo-placeholder", refreshToken: "demo-placeholder", expiresAt: 4102444800000, scopes: ["user:inference"] } };

/** The home to seed: `--home`, else the environment's, else the single folder under `<cwd>/.dev-home`. */
export function pickHome({ home, env, cwd }) {
    if (home) return resolve(home);
    if (env.AGENTMUX_HOME_OVERRIDE) return resolve(env.AGENTMUX_HOME_OVERRIDE);
    const root = join(cwd, HOME_DIR_NAME);
    const found = existsSync(root) ? readdirSync(root).filter((d) => !d.startsWith(".")) : [];
    if (found.length === 1) return join(root, found[0]);
    throw new Error(
        found.length === 0
            ? `no dev home: start \`task dev\` first (it makes one under ${root}), or pass --home`
            : `more than one dev home under ${root} (${found.join(", ")}): pass --home`,
    );
}

/** In the page: the user-owned agent definitions named `name`. */
const agentNamed = (name) => `(async () => {
    const r = await TabRpcClient.rpcCall("listagents", {});
    return (r.agents ?? r).filter((d) => !d.is_seeded && d.name === ${JSON.stringify(name)}).map((d) => d.id);
})()`;

export async function seed({ home, port, count, name, open, script }) {
    const problem = homeProblem(home);
    if (problem) throw new Error(`${home}: ${problem}`);
    const target = await findMainTarget(port);
    // A dev build's page is served by Vite on localhost; a release's is not.
    if (new URL(target.url).hostname !== "localhost") {
        throw new Error(`the window on port ${port} is not a dev build (${target.url}): not seeding it`);
    }
    mkdirSync(join(home, "shared"), { recursive: true });
    installFakeClaude(home, script ? resolve(script) : undefined);
    const accountDir = join(home, "demo-accounts", "claude");
    mkdirSync(accountDir, { recursive: true });
    // srv checks for a credential file before it asks the CLI; this one only ever reaches the stand-in.
    writeFileSync(join(accountDir, ".credentials.json"), `${JSON.stringify(PLACEHOLDER)}\n`);
    const workDir = join(home, "dev-workspace");
    mkdirSync(workDir, { recursive: true });

    const session = await CdpSession.connect(target.webSocketDebuggerUrl);
    try {
        await session.evaluate(`RpcApi.UpsertIdentityAccountCommand(TabRpcClient, { id: ${JSON.stringify(DEMO_ACCOUNT.id)}, name: ${JSON.stringify(DEMO_ACCOUNT.email)},
            provider: "claude", kind: "oauth", display_name: ${JSON.stringify(DEMO_ACCOUNT.email)}, status: "valid",
            secret_ref: { backend: "oauth_config_dir", dir: ${JSON.stringify(accountDir.replace(/\\/g, "/"))} } })`);
        const ids = [];
        for (let i = 1; i <= count; i++) {
            const agentName = count === 1 ? name : `${name}-${i}`;
            let [id] = await session.evaluate(agentNamed(agentName));
            if (!id) {
                const def = await session.evaluate(
                    `TabRpcClient.rpcCall("createagent", ${JSON.stringify({ name: agentName, provider: "claude", working_directory: workDir })})`,
                );
                id = def?.id ?? def?.agent?.id;
                if (!id) throw new Error(`createagent returned ${JSON.stringify(def)}`);
            }
            await session.evaluate(
                `RpcApi.LinkAgentIdentityCommand(TabRpcClient, { agent_id: ${JSON.stringify(id)}, account_id: ${JSON.stringify(DEMO_ACCOUNT.id)}, provider: "claude" })`,
            );
            ids.push({ name: agentName, id });
        }
        if (open && ids.length) {
            const tab = await session.evaluate(`document.querySelector('.tab.active')?.getAttribute('data-tab-id') ?? null`);
            if (tab) await session.evaluate(`TabRpcClient.rpcCall("agent.open", ${JSON.stringify({ agent_id: ids[0].id, tab_id: tab, focus: true })})`);
        }
        return ids;
    } finally {
        session.close();
    }
}

function parse(argv) {
    const get = (flag, fallback) => {
        const i = argv.indexOf(flag);
        return i >= 0 ? argv[i + 1] : fallback;
    };
    return {
        home: get("--home"),
        port: Number(get("--port", process.env.AGENTMUX_CDP_PORT && /^\d+$/.test(process.env.AGENTMUX_CDP_PORT) ? process.env.AGENTMUX_CDP_PORT : 9223)),
        count: Math.max(1, Number(get("--agents", 1))),
        name: get("--name", "fake-agent"),
        script: get("--script"),
        open: !argv.includes("--no-open"),
    };
}

async function main() {
    const opts = parse(process.argv.slice(2));
    try {
        const home = pickHome({ home: opts.home, env: process.env, cwd: process.cwd() });
        const ids = await seed({ ...opts, home });
        console.log(`seeded ${home}: ${ids.map((a) => a.name).join(", ")} (fake Claude Code, account ${DEMO_ACCOUNT.email})`);
    } catch (err) {
        console.error(`❌ ${err.message}`);
        process.exit(1);
    }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) await main();
