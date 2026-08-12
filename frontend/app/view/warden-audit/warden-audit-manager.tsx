// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — Audit section. The jekt-injection audit feed lifted out of the
// original monolithic warden.tsx's Host section into its own rail-
// switchable manager (it was never Host-specific — every jekt, from any
// tier, lands here). Also renders Supervisor's continue/decline decisions
// once those exist (entries with `outcome` set) — see
// docs/analysis/ANALYSIS_WARDEN_AUTO_CONTROLLER_CONTINUATION_WATCHER_2026_08_12.md.

import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";

import { ageMs, formatAge, WARDEN_REFRESH_MS } from "@/app/view/warden-shared/warden-shared";
import { fetchWardenAudit, WARDEN_AUDIT_LIMIT, type AuditEntry } from "./warden-audit-shared";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-audit-manager.scss";

export const WardenAuditManager = (): JSX.Element => {
    const [audit, setAudit] = createSignal<AuditEntry[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(true);
    const tick = useTick(1000);
    const now = createMemo(() => (tick(), Date.now()));

    const refresh = async () => {
        try {
            const auditLog = await fetchWardenAudit();
            setAudit(auditLog);
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
            <p class="warden-manager-summary">Every jekt delivery + Supervisor decision, most recent first</p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show
                when={audit().length > 0}
                fallback={
                    <div class="warden-section-stub">
                        {loading() ? "Loading…" : "No jekt activity yet."}
                    </div>
                }
            >
                <ul class="warden-audit-feed">
                    <For each={audit()}>
                        {(entry) => {
                            // Supervisor rows (outcome set) get neutral/informational
                            // styling regardless of `success` — a "declined" entry has
                            // success=true (nothing failed, it just chose not to act)
                            // and shouldn't read as an error.
                            const statusVariant = () => {
                                if (entry.outcome === "nudge_sent") return "nudge";
                                if (entry.outcome === "nudge_declined") return "declined";
                                return entry.success ? "ok" : "err";
                            };
                            const statusLabel = () => {
                                if (entry.outcome === "nudge_sent") return "nudged";
                                if (entry.outcome === "nudge_declined") return "declined";
                                return entry.success ? "ok" : "err";
                            };
                            return (
                                <li
                                    class="warden-audit-row"
                                    data-variant={statusVariant()}
                                >
                                    <span class="warden-audit-time">
                                        {formatAge(ageMs(entry.timestamp, now()))} ago
                                    </span>
                                    <span class="warden-audit-flow">
                                        <Show when={entry.source_agent} fallback={<span class="warden-manager-dim">—</span>}>
                                            <span class="warden-manager-mono">{entry.source_agent}</span>
                                        </Show>
                                        {" → "}
                                        <span class="warden-manager-mono">{entry.target_agent}</span>
                                    </span>
                                    <span class={`warden-audit-status warden-audit-status--${statusVariant()}`}>
                                        {statusLabel()}
                                    </span>
                                    <Show
                                        when={entry.outcome != null}
                                        fallback={<span class="warden-audit-bytes">{entry.message_length}b</span>}
                                    >
                                        <span class="warden-audit-bytes" />
                                    </Show>
                                    <Show when={entry.outcome != null && entry.reason} fallback={
                                        <Show when={!entry.success && entry.error_message}>
                                            <span class="warden-audit-error">{entry.error_message}</span>
                                        </Show>
                                    }>
                                        <span class="warden-audit-reason">{entry.reason}</span>
                                    </Show>
                                </li>
                            );
                        }}
                    </For>
                </ul>
            </Show>
            <div class="warden-manager-footnote">
                Refreshes every {WARDEN_REFRESH_MS / 1000}s · last {WARDEN_AUDIT_LIMIT} entries.
            </div>
        </div>
    );
};

WardenAuditManager.displayName = "WardenAuditManager";
