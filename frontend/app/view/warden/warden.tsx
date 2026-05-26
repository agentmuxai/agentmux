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

interface AuditEntry {
    timestamp: number;
    source_agent?: string;
    target_agent: string;
    block_id: string;
    message_hash: string;
    message_length: number;
    success: boolean;
    error_message?: string;
    request_id: string;
}

const HOST_REFRESH_MS = 5_000;
const AUDIT_LIMIT = 50;
// Last-seen newer than this counts as "active"; older is "idle".
const ACTIVE_THRESHOLD_MS = 30_000;

function authedHeaders(): Record<string, string> {
    const headers: Record<string, string> = {};
    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) headers["X-AuthKey"] = authKey;
    }
    return headers;
}

async function fetchHostAgents(): Promise<HostAgent[]> {
    const resp = await fetch(
        getWebServerEndpoint() + "/agentmux/reactive/agents",
        { headers: authedHeaders() },
    );
    if (!resp.ok) {
        throw new Error(`warden: GET /agentmux/reactive/agents → ${resp.status}`);
    }
    const data = await resp.json();
    return Array.isArray(data) ? (data as HostAgent[]) : [];
}

async function fetchHostAudit(): Promise<AuditEntry[]> {
    const resp = await fetch(
        getWebServerEndpoint() + `/agentmux/reactive/audit?limit=${AUDIT_LIMIT}`,
        { headers: authedHeaders() },
    );
    if (!resp.ok) {
        throw new Error(`warden: GET /agentmux/reactive/audit → ${resp.status}`);
    }
    const data = await resp.json();
    return Array.isArray(data) ? (data as AuditEntry[]) : [];
}

/**
 * Deregister an agent from the local ReactiveHandler. This is a *soft*
 * enforcement action — it removes the agent's routing entry so future
 * jekts return "agent not found", but does NOT kill the underlying PTY
 * process (that's owned by the pane / block controller). The agent's
 * shell auto-register hook may re-register on its next heartbeat.
 *
 * Hard kill (PTY termination) lives outside Warden — see future PR.
 */
async function deregisterAgent(agentId: string): Promise<void> {
    const resp = await fetch(
        getWebServerEndpoint() + "/agentmux/reactive/unregister",
        {
            method: "POST",
            headers: { ...authedHeaders(), "Content-Type": "application/json" },
            body: JSON.stringify({ agent_id: agentId }),
        },
    );
    if (!resp.ok) {
        throw new Error(`warden: POST /agentmux/reactive/unregister → ${resp.status}`);
    }
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
    const [audit, setAudit] = createSignal<AuditEntry[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(true);
    const [now, setNow] = createSignal(Date.now());

    const refresh = async () => {
        try {
            const [agentList, auditLog] = await Promise.all([
                fetchHostAgents(),
                fetchHostAudit().catch(() => [] as AuditEntry[]),
            ]);
            setAgents(agentList);
            setAudit(auditLog);
            setError(null);
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    const handleDeregister = async (agentId: string) => {
        const confirmed = globalThis.window?.confirm(
            `Deregister agent "${agentId}"?\n\nThis removes its routing entry so future jekts return "agent not found". The underlying process keeps running and may re-register on its next heartbeat.`,
        );
        if (!confirmed) return;
        try {
            await deregisterAgent(agentId);
            void refresh();
        } catch (e) {
            setError(String(e));
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
                            <th></th>
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
                                        <td class="warden-host-actions">
                                            <button
                                                class="warden-host-deregister"
                                                title="Deregister (soft kill — removes from jekt routing, leaves process running)"
                                                onClick={() => void handleDeregister(a.agent_id)}
                                            >
                                                ×
                                            </button>
                                        </td>
                                    </tr>
                                );
                            }}
                        </For>
                    </tbody>
                </table>
            </Show>
            <div class="warden-host-section-divider">Recent jekts (audit)</div>
            <Show
                when={audit().length > 0}
                fallback={
                    <div class="warden-section-stub">No jekt activity yet.</div>
                }
            >
                <ul class="warden-audit-feed">
                    <For each={audit().slice(0, 20)}>
                        {(entry) => (
                            <li
                                class="warden-audit-row"
                                data-success={entry.success ? "true" : "false"}
                            >
                                <span class="warden-audit-time">
                                    {formatAge(ageMs(entry.timestamp, now()))} ago
                                </span>
                                <span class="warden-audit-flow">
                                    <Show when={entry.source_agent} fallback={<span class="warden-host-dim">—</span>}>
                                        <span class="warden-host-mono">{entry.source_agent}</span>
                                    </Show>
                                    {" → "}
                                    <span class="warden-host-mono">{entry.target_agent}</span>
                                </span>
                                <span class={`warden-audit-status warden-audit-status--${entry.success ? "ok" : "err"}`}>
                                    {entry.success ? "ok" : "err"}
                                </span>
                                <span class="warden-audit-bytes">{entry.message_length}b</span>
                                <Show when={!entry.success && entry.error_message}>
                                    <span class="warden-audit-error">{entry.error_message}</span>
                                </Show>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
            <div class="warden-host-footnote">
                Refreshes every {HOST_REFRESH_MS / 1000}s · last {AUDIT_LIMIT} jekts.
                Identity, capabilities, and jekt/min counters are coming in a
                follow-up PR.
            </div>
        </div>
    );
};

// ── LAN section ──────────────────────────────────────────────────────

interface LanPeer {
    instance_id: string;
    hostname: string;
    version: string;
    address: string;
    port: number;
    agents: string[];
    first_seen: number;
    last_seen: number;
}

async function fetchLanPeers(): Promise<LanPeer[]> {
    // /api/lan-instances is a public route (no auth required).
    const resp = await fetch(getWebServerEndpoint() + "/api/lan-instances");
    if (!resp.ok) {
        throw new Error(`warden: GET /api/lan-instances → ${resp.status}`);
    }
    const data = await resp.json();
    return Array.isArray(data) ? (data as LanPeer[]) : [];
}

const LanSection = (): JSX.Element => {
    const [peers, setPeers] = createSignal<LanPeer[]>([]);
    const [loading, setLoading] = createSignal(true);
    const [error, setError] = createSignal<string | null>(null);
    const [now, setNow] = createSignal(Date.now());

    const refresh = async () => {
        try {
            const list = await fetchLanPeers();
            setPeers(list);
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
        // `last_seen` is in unix *seconds* on the LAN endpoint (not ms like
        // ReactiveHandler) — see lan_discovery.rs:135. The clock tick keeps
        // the relative "Xs ago" column fresh between fetches.
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
                when={peers().length > 0}
                fallback={
                    <div class="warden-section-stub">
                        {loading()
                            ? "Loading…"
                            : 'No LAN peers. Enable mDNS via the HostPopover toggle ("LAN discovery") on each instance.'}
                    </div>
                }
            >
                <table class="warden-host-table">
                    <thead>
                        <tr>
                            <th>peer</th>
                            <th>version</th>
                            <th>address</th>
                            <th>agents</th>
                            <th>last seen</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={peers()}>
                            {(p) => {
                                const lastSeenMs = p.last_seen * 1000;
                                return (
                                    <tr>
                                        <td class="warden-host-mono">
                                            {p.hostname || p.instance_id}
                                        </td>
                                        <td class="warden-host-mono warden-host-dim">v{p.version}</td>
                                        <td class="warden-host-mono warden-host-dim">
                                            {p.address}:{p.port}
                                        </td>
                                        <td>{p.agents.length}</td>
                                        <td>{formatAge(ageMs(lastSeenMs, now()))} ago</td>
                                    </tr>
                                );
                            }}
                        </For>
                    </tbody>
                </table>
            </Show>
            <div class="warden-host-footnote">
                Refreshes every {HOST_REFRESH_MS / 1000}s · mDNS via
                <code> _agentmux._tcp.local</code>. Cross-instance jekt and
                quarantine controls land in PR-F (after lan-awareness Phase 3).
            </div>
        </div>
    );
};

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
        status: "live",
        render: () => <LanSection />,
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
