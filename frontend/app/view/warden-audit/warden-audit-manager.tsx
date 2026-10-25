// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — Audit section. The jekt-injection audit feed lifted out of the
// original monolithic warden.tsx's Host section into its own section
// (it was never Host-specific — every jekt, from any
// tier, lands here), plus fleet and pane actions (with their `reason`).

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";

import { ageMs, formatAge, pollWhileVisible, WARDEN_REFRESH_MS } from "@/app/view/warden-shared/warden-shared";
import { WardenSecrets } from "@/app/view/vault/vault-warden";
import { fetchWardenAudit, WARDEN_AUDIT_LIMIT, type AuditEntry } from "./warden-audit-shared";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-audit-manager.scss";

/** What each non-delivery event reads as after the agent's name. */
const REGISTRATION_VERBS: Record<string, string> = {
    register: " registered",
    unregister: " unregistered",
    swept: " swept (its pane was gone)",
};

const isRegistration = (entry: AuditEntry) => entry.event_kind != null && entry.event_kind in REGISTRATION_VERBS;

/** Which block it was, and whom a registration displaced: a same-host id collision shows here. */
function registrationNote(entry: AuditEntry): string {
    const parts = [`block ${entry.block_id.slice(0, 8)}`];
    if (entry.evicted_block) parts.push(`replaced block ${entry.evicted_block.slice(0, 8)}`);
    if (entry.evicted_agent) parts.push(`block was ${entry.evicted_agent}`);
    return parts.join(", ");
}

export const WardenAuditManager = (props: { visible?: boolean }): JSX.Element => {
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

    // Polls only while the section is shown (warden-view.tsx passes `visible`).
    pollWhileVisible(() => void refresh(), () => props.visible ?? true);

    return (
        <div class="warden-manager-body">
            <p class="warden-manager-summary">
                Every jekt delivery, fleet action and agent registration, most recent first
            </p>
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
                            const status = () => (entry.success ? "ok" : "err");
                            return (
                                <li
                                    class="warden-audit-row"
                                    data-variant={status()}
                                >
                                    <span class="warden-audit-time">
                                        {formatAge(ageMs(entry.timestamp, now()))} ago
                                    </span>
                                    <span class="warden-audit-flow">
                                        <Show
                                            when={!isRegistration(entry)}
                                            fallback={
                                                <>
                                                    <span class="warden-manager-mono">{entry.target_agent}</span>
                                                    {REGISTRATION_VERBS[entry.event_kind ?? ""]}
                                                </>
                                            }
                                        >
                                            <Show when={entry.source_agent} fallback={<span class="warden-manager-dim">—</span>}>
                                                <span class="warden-manager-mono">{entry.source_agent}</span>
                                            </Show>
                                            {" → "}
                                            <span class="warden-manager-mono">{entry.target_agent}</span>
                                        </Show>
                                    </span>
                                    <span class={`warden-audit-status warden-audit-status--${status()}`}>
                                        {status()}
                                    </span>
                                    <span class="warden-audit-bytes">
                                        <Show when={!isRegistration(entry)}>{entry.message_length}b</Show>
                                    </span>
                                    {/* A caller-supplied `reason` (e.g. ClosePane's) shows on
                                        its own: outcome is unset for fleet actions (#3196). */}
                                    <Show when={entry.reason} fallback={
                                        <Show when={!entry.success && entry.error_message}>
                                            <span class="warden-audit-error">{entry.error_message}</span>
                                        </Show>
                                    }>
                                        <span class="warden-audit-reason">{entry.reason}</span>
                                    </Show>
                                    <Show when={isRegistration(entry)}>
                                        <span class="warden-audit-reason">{registrationNote(entry)}</span>
                                    </Show>
                                </li>
                            );
                        }}
                    </For>
                </ul>
            </Show>
            <WardenSecrets visible={props.visible} />
            <div class="warden-manager-footnote">
                Refreshes every {WARDEN_REFRESH_MS / 1000}s · last {WARDEN_AUDIT_LIMIT} entries.
            </div>
        </div>
    );
};

WardenAuditManager.displayName = "WardenAuditManager";
