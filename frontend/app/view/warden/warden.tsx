// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden widget — three-layer operator surface (Host / LAN / Internet).
//
// Phase 1 shell shipped the section scaffold. This PR (Phase 2 of the
// spec — Host L1 read-only) lights up the Host section: it fetches the
// agent list from /agentmux/reactive/agents on mount and refreshes every
// 5 s. LAN and Internet remain stubs until their substrates land.
//
// Spec: specs/SPEC_WARDEN_WIDGET_2026-05-25.md

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { getApi } from "@/store/global";
import { getWebServerEndpoint } from "@/util/endpoints";

import "./warden.scss";

class WardenViewModel implements ViewModel {
    viewType: string;
    blockId: string;

    constructor(blockId: string) {
        this.viewType = "warden";
        this.blockId = blockId;
    }

    get viewComponent(): ViewComponent {
        return WardenView as unknown as ViewComponent;
    }
}

// ── Host section data ────────────────────────────────────────────────

interface HostAgent {
    agent_id: string;
    block_id: string;
    tab_id?: string;
    registered_at: number;
    last_seen: number;
}

const HOST_REFRESH_MS = 5_000;
// Last-seen newer than this counts as "active"; older is "idle".
const ACTIVE_THRESHOLD_MS = 30_000;

async function fetchHostAgents(): Promise<HostAgent[]> {
    const headers: Record<string, string> = {};
    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) headers["X-AuthKey"] = authKey;
    }
    const resp = await fetch(
        getWebServerEndpoint() + "/agentmux/reactive/agents",
        { headers },
    );
    if (!resp.ok) {
        throw new Error(`warden: GET /agentmux/reactive/agents → ${resp.status}`);
    }
    const data = await resp.json();
    return Array.isArray(data) ? (data as HostAgent[]) : [];
}

function ageMs(ts: number, now: number): number {
    // `last_seen` / `registered_at` are unix millis from the Rust backend.
    return Math.max(0, now - ts);
}

function formatAge(ms: number): string {
    if (ms < 60_000) return `${Math.floor(ms / 1000)}s`;
    if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m`;
    return `${Math.floor(ms / 3_600_000)}h`;
}

function agentState(agent: HostAgent, now: number): "active" | "idle" {
    return ageMs(agent.last_seen, now) < ACTIVE_THRESHOLD_MS ? "active" : "idle";
}

const HostSection = (): JSX.Element => {
    const [agents, setAgents] = createSignal<HostAgent[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(true);
    const [now, setNow] = createSignal(Date.now());

    const refresh = async () => {
        try {
            const list = await fetchHostAgents();
            setAgents(list);
            setError(null);
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    onMount(() => {
        void refresh();
        const dataTimer = window.setInterval(() => void refresh(), HOST_REFRESH_MS);
        // Tick `now` once per second so age columns update without a fresh
        // fetch. Cheap — just re-renders the existing rows.
        const clockTimer = window.setInterval(() => setNow(Date.now()), 1000);
        onCleanup(() => {
            window.clearInterval(dataTimer);
            window.clearInterval(clockTimer);
        });
    });

    return (
        <div class="warden-host">
            <Show when={error()}>
                <div class="warden-host-error">⚠ {error()}</div>
            </Show>
            <Show
                when={agents().length > 0}
                fallback={
                    <div class="warden-section-stub">
                        {loading() ? "Loading…" : "No agents registered on this host."}
                    </div>
                }
            >
                <table class="warden-host-table">
                    <thead>
                        <tr>
                            <th>agent</th>
                            <th>block</th>
                            <th>last seen</th>
                            <th>state</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={agents()}>
                            {(a) => {
                                const state = () => agentState(a, now());
                                return (
                                    <tr data-state={state()}>
                                        <td class="warden-host-mono">{a.agent_id}</td>
                                        <td class="warden-host-mono warden-host-dim">
                                            {a.block_id.slice(0, 8)}
                                        </td>
                                        <td>{formatAge(ageMs(a.last_seen, now()))} ago</td>
                                        <td>
                                            <span class={`warden-host-state warden-host-state--${state()}`}>
                                                {state()}
                                            </span>
                                        </td>
                                    </tr>
                                );
                            }}
                        </For>
                    </tbody>
                </table>
            </Show>
            <div class="warden-host-footnote">
                Refreshes every {HOST_REFRESH_MS / 1000}s. Identity, capabilities, and
                jekt/min are coming in a follow-up PR.
            </div>
        </div>
    );
};

// ── Stub sections (unchanged from Phase 1 shell) ─────────────────────

const LanStub = (): JSX.Element => (
    <div class="warden-section-stub">
        Peer list reads through to <code>lan_discovery</code>. Enrollment + policy
        push wait on lan-awareness Phase 3 (LAN jekt forwarding).
    </div>
);

const InternetStub = (): JSX.Element => (
    <div class="warden-section-stub">
        Closed by default. Cross-network governance ships behind lan-awareness
        Phase 4 (cloud fallback).
    </div>
);

// ── Layer scaffold ────────────────────────────────────────────────────

interface LayerSection {
    key: "host" | "lan" | "internet";
    title: string;
    summary: string;
    status: "live" | "stub" | "disabled";
    render: () => JSX.Element;
}

const SECTIONS: LayerSection[] = [
    {
        key: "host",
        title: "Host",
        summary: "This AgentMux process · jekt tiers 1–2 · < 1 ms",
        status: "live",
        render: () => <HostSection />,
    },
    {
        key: "lan",
        title: "LAN",
        summary: "mDNS-discovered peers · jekt tier 3 · 1–10 ms",
        status: "stub",
        render: () => <LanStub />,
    },
    {
        key: "internet",
        title: "Internet",
        summary: "AgentBus cloud relay · jekt tier 4 · opt-in",
        status: "disabled",
        render: () => <InternetStub />,
    },
];

function WardenView({ model: _model }: { model: WardenViewModel }): JSX.Element {
    return (
        <div class="warden-pane">
            <header class="warden-header">
                <span class="warden-title">Warden</span>
                <span class="warden-subtitle">3-layer operator surface</span>
            </header>
            <div class="warden-sections">
                <For each={SECTIONS}>
                    {(section) => (
                        <section
                            class="warden-section"
                            data-status={section.status}
                            data-layer={section.key}
                        >
                            <header class="warden-section-header">
                                <span class="warden-section-title">{section.title}</span>
                                <span class="warden-section-summary">{section.summary}</span>
                                <span class="warden-section-status">{section.status}</span>
                            </header>
                            <div class="warden-section-body">{section.render()}</div>
                        </section>
                    )}
                </For>
            </div>
        </div>
    );
}

WardenView.displayName = "WardenView";

export { WardenViewModel };
