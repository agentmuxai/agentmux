// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — LAN section. Lifted out of the original monolithic warden.tsx
// (docs/specs/SPEC_WARDEN_WIDGET_2026-05-25.md) into its own rail-switchable
// manager. Behavior unchanged, including the known pre-existing bug noted
// below (out of scope for this restructure).

import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";

import { getWebServerEndpoint } from "@/util/endpoints";
import { authedHeaders, ageMs, formatAge, WARDEN_REFRESH_MS } from "@/app/view/warden-shared/warden-shared";

import "@/app/view/warden-shared/warden-manager-chrome.scss";

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
    // /api/lan-instances sits in `authed_routes` (server/mod.rs) like every
    // other HTTP route — the 2026-05-11 audit removed all unauthenticated
    // localhost routes, which predates this section. The original "public
    // route (no auth required)" comment here was wrong from the start, so
    // this fetch 401'd from the day it shipped (visible as the LAN
    // section's inline "GET /api/lan-instances → 401" error). Pre-existing,
    // out of scope for this rail restructure.
    const resp = await fetch(getWebServerEndpoint() + "/api/lan-instances", {
        headers: authedHeaders(),
    });
    if (!resp.ok) {
        throw new Error(`warden: GET /api/lan-instances → ${resp.status}`);
    }
    const data = await resp.json();
    return Array.isArray(data) ? (data as LanPeer[]) : [];
}

export const WardenLanManager = (): JSX.Element => {
    const [peers, setPeers] = createSignal<LanPeer[]>([]);
    const [loading, setLoading] = createSignal(true);
    const [error, setError] = createSignal<string | null>(null);
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));

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
        const dataTimer = window.setInterval(() => void refresh(), WARDEN_REFRESH_MS);
        onCleanup(() => window.clearInterval(dataTimer));
    });

    return (
        <div class="warden-manager-body">
            <p class="warden-manager-summary">mDNS-discovered peers · jekt tier 3 · 1–10 ms</p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
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
                <table class="warden-manager-table">
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
                                        <td class="warden-manager-mono">
                                            {p.hostname || p.instance_id}
                                        </td>
                                        <td class="warden-manager-mono warden-manager-dim">v{p.version}</td>
                                        <td class="warden-manager-mono warden-manager-dim">
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
            <div class="warden-manager-footnote">
                Refreshes every {WARDEN_REFRESH_MS / 1000}s · mDNS via
                <code> _agentmux._tcp.local</code>. Cross-instance jekt and
                quarantine controls land in PR-F (after lan-awareness Phase 3).
            </div>
        </div>
    );
};

WardenLanManager.displayName = "WardenLanManager";
