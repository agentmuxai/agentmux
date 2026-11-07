// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Agents on sites, the views: the list of grants, the add form and the console presets, over a state from
// agent-site-grants-state.ts.

import { Button, Select, TextInput } from "@/app/element/ui";
import { For, Show, type Accessor, type JSX } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { BrowserGrantsResult } from "@/types/rpc/BrowserGrantsResult";
import { ago } from "./pinned-senders";
import { POWER_LABELS, powerChecklist, sitesLabel } from "./agent-site-grants-model";
import type { AgentSiteGrantsState, TerminalChoice } from "./agent-site-grants-state";

type Grants = Accessor<BrowserGrantsResult>;

/** The add form's terminal options: off, run freely, or show the person each line first (which needs the terminal on). */
function TerminalChoices(props: { value: TerminalChoice; onChange: (v: TerminalChoice) => void }) {
    return (
        <>
            <label class="agent-site-grant-power">
                <input type="checkbox" checked={props.value !== "none"} onChange={(e) => props.onChange(e.currentTarget.checked ? "terminal" : "none")} />
                <span>{POWER_LABELS.terminal}</span>
            </label>
            <label class="agent-site-grant-power">
                <input
                    type="checkbox"
                    checked={props.value === "terminal_ask"}
                    disabled={props.value === "none"}
                    onChange={(e) => props.onChange(e.currentTarget.checked ? "terminal_ask" : "terminal")}
                />
                <span>and show me each line first</span>
            </label>
        </>
    );
}

function GrantRow(props: { s: AgentSiteGrantsState; d: Grants; row: BrowserGrantsResult["rows"][number] }): JSX.Element {
    const { s, d, row } = props;
    return (
        <li class="agent-site-grant">
            <div class="agent-site-grant-who">
                <span class="agent-site-grant-agent">{s.nameOfSlug(row.agent)}</span>
                <span class="agent-site-grant-site">{sitesLabel(row.origins)}</span>
                <span class="agent-site-grant-when">granted {ago(Math.floor(row.granted_at_ms / 1000), s.now())}</span>
            </div>
            <div class="agent-site-grant-powers" role="group" aria-label={`What ${s.nameOfSlug(row.agent)} may do on ${sitesLabel(row.origins)}`}>
                <For each={powerChecklist(d().powers, row.powers)}>
                    {(p) => (
                        <label class="agent-site-grant-power">
                            <input
                                type="checkbox"
                                checked={p.on}
                                disabled={s.busy()}
                                aria-label={`${POWER_LABELS[p.power] ?? p.power} for ${s.nameOfSlug(row.agent)} on ${sitesLabel(row.origins)}`}
                                onChange={(e) =>
                                    void s.run(`change ${s.nameOfSlug(row.agent)}'s powers`, () =>
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
                disabled={s.busy()}
                aria-label={`Revoke ${s.nameOfSlug(row.agent)} on ${sitesLabel(row.origins)}`}
                onClick={() => void s.run("revoke it", () => RpcApi.BrowserGrantsRevokeCommand(TabRpcClient, { id: row.id }))}
            >
                Revoke
            </Button>
        </li>
    );
}

function GrantList(props: { s: AgentSiteGrantsState; d: Grants }): JSX.Element {
    return (
        <Show
            when={props.d().rows.length > 0}
            fallback={<div class="setting-devices-empty">No agent has a standing grant. Each one asks in the pane, and you answer there.</div>}
        >
            <ul class="agent-site-grants-list" aria-label="Agents on sites">
                <For each={props.d().rows}>{(row) => <GrantRow s={props.s} d={props.d} row={row} />}</For>
            </ul>
        </Show>
    );
}

function AddForm(props: { s: AgentSiteGrantsState }): JSX.Element {
    const s = props.s;
    return (
        <div class="agent-site-grants-add">
            <Select
                ariaLabel="Agent to give a standing grant"
                value={s.pick()}
                onChange={s.setPick}
                options={[{ value: "", label: "Choose an agent…" }, ...s.candidates().map((a) => ({ value: a.slug, label: s.agentName(a) }))]}
            />
            <TextInput
                detached
                aria-label="Site"
                placeholder="shell.example.com"
                value={s.site()}
                onInput={(e) => s.setSite(e.currentTarget.value)}
                onKeyDown={(e) => {
                    if (e.key === "Enter") void s.add();
                }}
            />
            <TerminalChoices value={s.terminal()} onChange={s.setTerminal} />
            <Button disabled={!s.picked() || !s.site().trim() || s.busy()} onClick={() => void s.add()}>
                Allow
            </Button>
        </div>
    );
}

function Presets(props: { s: AgentSiteGrantsState; d: Grants }): JSX.Element {
    const s = props.s;
    return (
        <Show when={props.d().presets.length > 0}>
            <div class="agent-site-grants-presets" role="group" aria-label="Consoles">
                <span class="agent-site-grants-presets-label">Consoles, with the terminal:</span>
                <For each={props.d().presets}>
                    {(preset) => (
                        <Button
                            disabled={!s.picked() || s.busy()}
                            title={`${preset.sites.join(", ")}. ${preset.note}`}
                            onClick={() => {
                                const agent = s.picked();
                                if (agent)
                                    void s.run(`turn on ${preset.label}`, () =>
                                        RpcApi.BrowserGrantsPresetCommand(TabRpcClient, { preset: preset.id, agent: agent.slug })
                                    );
                            }}
                        >
                            {preset.label}
                        </Button>
                    )}
                </For>
                <Show when={!s.picked()}>
                    <span class="setting-devices-empty">Choose an agent first.</span>
                </Show>
            </div>
        </Show>
    );
}

/** What shows once the grants have loaded: the list, the add form and the presets. */
export function GrantsBody(props: { s: AgentSiteGrantsState; d: Grants }): JSX.Element {
    return (
        <>
            <GrantList s={props.s} d={props.d} />
            <AddForm s={props.s} />
            <Presets s={props.s} d={props.d} />
        </>
    );
}
