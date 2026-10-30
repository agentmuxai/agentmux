// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Agents on sites: the standing list of what an agent may do on a site
// without asking each time (docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md §3).
// A row names one agent and one site and the powers it carries; Revoke ends it
// at once, and a call in flight stops. Only this window can write one
// (crates/srv/src/server/app_api/browser_grants.rs), and an agent's own
// request for auto-approve can add one when the person answers "Always".

import { Button, Notice, Select, TextInput } from "@/app/element/ui";
import { createSignal, For, onMount, Show, type JSX } from "solid-js";

import { RpcApi, type AgentDefinition } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { BrowserGrantsResult } from "@/types/rpc/BrowserGrantsResult";
import type { SettingsIndexEntry } from "../settings-model";
import { sortByLastUsed } from "@/app/view/agent/agent-recency";
import { ago } from "./pinned-senders";
import { LATE_PROMPT_LIMIT, POWER_LABELS, powerChecklist, sitesLabel } from "./agent-site-grants-model";

export const AGENT_SITE_GRANTS_SETTING = {
    id: "browser.agent_sites",
    label: "Agents on sites",
    description:
        "What an agent may do on a site without asking each time, until you revoke it. Every row lets the agent make committing clicks there without asking (a payment still asks). Add Terminal to let it type and run commands in a web terminal there, such as Cloud Shell: nobody checks each line, so Stop, Pause and Take over in the pane are how you stop it, and Revoke here ends it at once.",
    section: "browser",
    keywords: ["agent", "agents", "site", "sites", "grant", "auto-approve", "terminal", "cloud shell", "revoke", "always", "standing", "permission"],
} satisfies SettingsIndexEntry;

const agentName = (a: AgentDefinition) => a.name || a.slug || a.id;

export function AgentSiteGrants(): JSX.Element {
    const [data, setData] = createSignal<BrowserGrantsResult | null>(null);
    const [agents, setAgents] = createSignal<AgentDefinition[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    const [pick, setPick] = createSignal("");
    const [site, setSite] = createSignal("");
    const [terminal, setTerminal] = createSignal(false);
    const now = () => Math.floor(Date.now() / 1000);

    const load = async () => {
        try {
            const [grants, defs] = await Promise.all([
                RpcApi.BrowserGrantsListCommand(TabRpcClient),
                RpcApi.ListAgentDefinitionsCommand(TabRpcClient, { is_seeded: 0 }),
            ]);
            setData(grants);
            setAgents(defs);
            setError(null);
        } catch (e) {
            setError(`Couldn't load the agents on sites: ${e instanceof Error ? e.message : e}`);
        }
    };
    onMount(() => void load());

    /** One change, then the list as srv now has it. */
    const run = async (what: string, change: () => Promise<BrowserGrantsResult>) => {
        setBusy(true);
        try {
            setData(await change());
            setError(null);
            return true;
        } catch (e) {
            setError(`Couldn't ${what}: ${e instanceof Error ? e.message : e}`);
            return false;
        } finally {
            setBusy(false);
        }
    };

    const nameOfSlug = (slug: string) => {
        const a = agents().find((d) => d.slug.toLowerCase() === slug.toLowerCase());
        return a ? agentName(a) : slug;
    };
    const candidates = () => sortByLastUsed(agents(), agentName);
    const picked = () => candidates().find((a) => a.slug === pick());

    const add = async () => {
        const agent = picked();
        if (!agent || !site().trim()) return;
        const ok = await run(`let ${agentName(agent)} onto ${site().trim()}`, () =>
            RpcApi.BrowserGrantsSetCommand(TabRpcClient, {
                agent: agent.slug,
                site: site().trim(),
                powers: terminal() ? ["terminal"] : [],
            })
        );
        if (ok) setSite("");
    };

    return (
        <div id={`setting-${AGENT_SITE_GRANTS_SETTING.id}`} class="setting-row agent-site-grants">
            <div class="setting-devices-description">{AGENT_SITE_GRANTS_SETTING.description}</div>
            <Notice tone="warning" title={<>One limit of the Terminal power</>}>
                {LATE_PROMPT_LIMIT}
            </Notice>
            <Show when={error()}>
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            </Show>
            <Show
                when={data()}
                fallback={
                    <Show when={!error()}>
                        <div class="setting-devices-empty">Loading…</div>
                    </Show>
                }
            >
                {(d) => (
                    <>
                        <Show
                            when={d().rows.length > 0}
                            fallback={<div class="setting-devices-empty">No agent has a standing grant. Each one asks in the pane, and you answer there.</div>}
                        >
                            <ul class="agent-site-grants-list" aria-label="Agents on sites">
                                <For each={d().rows}>
                                    {(row) => (
                                        <li class="agent-site-grant">
                                            <div class="agent-site-grant-who">
                                                <span class="agent-site-grant-agent">{nameOfSlug(row.agent)}</span>
                                                <span class="agent-site-grant-site">{sitesLabel(row.origins)}</span>
                                                <span class="agent-site-grant-when">granted {ago(Math.floor(row.granted_at_ms / 1000), now())}</span>
                                            </div>
                                            <div class="agent-site-grant-powers" role="group" aria-label={`What ${nameOfSlug(row.agent)} may do on ${sitesLabel(row.origins)}`}>
                                                <For each={powerChecklist(d().powers, row.powers)}>
                                                    {(p) => (
                                                        <label class="agent-site-grant-power">
                                                            <input
                                                                type="checkbox"
                                                                checked={p.on}
                                                                disabled={busy()}
                                                                aria-label={`${POWER_LABELS[p.power] ?? p.power} for ${nameOfSlug(row.agent)} on ${sitesLabel(row.origins)}`}
                                                                onChange={(e) =>
                                                                    void run(`change ${nameOfSlug(row.agent)}'s powers`, () =>
                                                                        RpcApi.BrowserGrantsPowersCommand(TabRpcClient, {
                                                                            id: row.id,
                                                                            powers: powerChecklist(d().powers, row.powers)
                                                                                .map((q) => (q.power === p.power ? { ...q, on: e.currentTarget.checked } : q))
                                                                                .filter((q) => q.on)
                                                                                .map((q) => q.power),
                                                                        })
                                                                    )
                                                                }
                                                            />
                                                            <span>{POWER_LABELS[p.power] ?? p.power}</span>
                                                        </label>
                                                    )}
                                                </For>
                                                <span class="agent-site-grant-power agent-site-grant-power-fixed">Committing clicks</span>
                                            </div>
                                            <Button
                                                disabled={busy()}
                                                aria-label={`Revoke ${nameOfSlug(row.agent)} on ${sitesLabel(row.origins)}`}
                                                onClick={() => void run("revoke it", () => RpcApi.BrowserGrantsRevokeCommand(TabRpcClient, { id: row.id }))}
                                            >
                                                Revoke
                                            </Button>
                                        </li>
                                    )}
                                </For>
                            </ul>
                        </Show>
                        <div class="agent-site-grants-add">
                            <Select
                                ariaLabel="Agent to give a standing grant"
                                value={pick()}
                                onChange={setPick}
                                options={[{ value: "", label: "Choose an agent…" }, ...candidates().map((a) => ({ value: a.slug, label: agentName(a) }))]}
                            />
                            <TextInput
                                detached
                                aria-label="Site"
                                placeholder="shell.example.com"
                                value={site()}
                                onInput={(e) => setSite(e.currentTarget.value)}
                                onKeyDown={(e) => {
                                    if (e.key === "Enter") void add();
                                }}
                            />
                            <label class="agent-site-grant-power">
                                <input type="checkbox" checked={terminal()} onChange={(e) => setTerminal(e.currentTarget.checked)} />
                                <span>{POWER_LABELS.terminal}</span>
                            </label>
                            <Button disabled={!picked() || !site().trim() || busy()} onClick={() => void add()}>
                                Allow
                            </Button>
                        </div>
                        <Show when={d().presets.length > 0}>
                            <div class="agent-site-grants-presets" role="group" aria-label="Consoles">
                                <span class="agent-site-grants-presets-label">Consoles, with the terminal:</span>
                                <For each={d().presets}>
                                    {(preset) => (
                                        <Button
                                            disabled={!picked() || busy()}
                                            title={`${preset.sites.join(", ")}. ${preset.note}`}
                                            onClick={() => {
                                                const agent = picked();
                                                if (agent)
                                                    void run(`turn on ${preset.label}`, () =>
                                                        RpcApi.BrowserGrantsPresetCommand(TabRpcClient, { preset: preset.id, agent: agent.slug })
                                                    );
                                            }}
                                        >
                                            {preset.label}
                                        </Button>
                                    )}
                                </For>
                                <Show when={!picked()}>
                                    <span class="setting-devices-empty">Choose an agent first.</span>
                                </Show>
                            </div>
                        </Show>
                    </>
                )}
            </Show>
        </div>
    );
}
