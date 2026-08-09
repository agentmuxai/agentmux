// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, For, Show, type JSX } from "solid-js";
import type { Account, IdentityViewModel } from "./identity-model";
import { agentsAssignedToAccount, KIND_LABELS, PROVIDER_LABELS } from "./identity-model";
import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { ProviderLogo } from "@/element/ProviderLogo";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { brandForProvider, isCliOAuthProvider } from "@/app/view/accounts/provider-brand";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { buildAccountRowMenu } from "./bind-to-agent-menu";
import { Modal, ModalBody, ModalFooter, ModalHeader } from "@/app/element/modal";
import "./identity-view.scss";

const STATUS_DOT: Record<string, string> = {
    valid: "status-dot status-valid",
    expired: "status-dot status-expired",
    invalid: "status-dot status-invalid",
    checking: "status-dot status-checking",
    unknown: "status-dot status-unknown",
};

// ── Accounts tab ─────────────────────────────────────────────────────────────

export function AccountsTab({ model }: { model: IdentityViewModel }): JSX.Element {
    const groups = () => model.accountsByProvider();
    const agents = useAgentDefinitions();

    // Right-click an account row → "Bind to Agent" submenu + Copy account ID
    // (SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md). Menu contents
    // are computed fresh per open (links + open-pane snapshot), so the
    // binding annotations are current without any subscription here. The
    // account list itself re-renders via the shared cache's live sync
    // (identityaccounts:changed → #2474) after a bind.
    const handleRowContextMenu = (account: Account, e: MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        void buildAccountRowMenu(account, agents(), model.accountsAtom()).then((items) => {
            ContextMenuModel.showContextMenu(items, e);
        });
    };

    return (
        <>
            {/* Delete-time disclosure (layer 4 —
                SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §4).
                Transient, dismissable; only fires when the deleted account
                had linked agents. Honest wording: running processes keep
                their tokens until restarted — we disclose, not revoke. */}
            <Show when={model.deleteNoticeAtom()}>
                {(notice) => (
                    <div class="identity-delete-notice" role="status" aria-live="polite">
                        <span class="identity-delete-notice-icon" aria-hidden="true">⚠</span>
                        <span class="identity-delete-notice-text">{notice()}</span>
                        <button
                            type="button"
                            class="identity-delete-notice-dismiss"
                            title="Dismiss"
                            aria-label="Dismiss"
                            onClick={() => model.dismissDeleteNotice()}
                        >
                            ×
                        </button>
                    </div>
                )}
            </Show>
            <div class="identity-accounts-layout">
                <div class="identity-accounts-list">
                    <Show
                        when={model.accountsAtom().length > 0}
                        fallback={
                            <div class="identity-empty">
                                <p>No accounts configured.</p>
                                <button class="identity-empty-add" onClick={() => model.openAddForm()}>
                                    + Add your first account
                                </button>
                            </div>
                        }
                    >
                        <For each={[...groups().entries()]}>
                            {([provider, accounts]) => (
                                <div class="identity-group">
                                    <div class="identity-group-header">{PROVIDER_LABELS[provider]}</div>
                                    <For each={accounts}>
                                        {(account) => (
                                            <AccountRow
                                                account={account}
                                                selected={model.selectedAccountAtom()?.id === account.id}
                                                onClick={() => model.setSelectedAccount(account)}
                                                onContextMenu={(e) => handleRowContextMenu(account, e)}
                                            />
                                        )}
                                    </For>
                                </div>
                            )}
                        </For>
                    </Show>
                </div>
            </div>

            {/* Central detail overlay — keyed so switching accounts remounts the content */}
            <Modal
                open={model.selectedAccountAtom() !== null}
                onClose={() => model.setSelectedAccount(null)}
                scope="window"
                size="md"
                showCloseButton
            >
                <Show when={model.selectedAccountAtom()} keyed>
                    {(account) => <AccountDetail model={model} account={account} />}
                </Show>
            </Modal>
        </>
    );
}

function AccountRow(props: {
    account: Account;
    selected: boolean;
    onClick: () => void;
    onContextMenu?: (e: MouseEvent) => void;
}): JSX.Element {
    const a = props.account;
    return (
        <div
            class={`identity-account-row${props.selected ? " selected" : ""}`}
            onClick={props.onClick}
            onContextMenu={(e) => props.onContextMenu?.(e)}
        >
            <span class={`identity-provider-badge provider-${a.provider}`}>
                <ProviderLogo provider={a.provider} size={16} />
            </span>
            <span class="identity-account-name">{a.name}</span>
            <div class="identity-row-meta">
                <Show when={a.display_name}>
                    <span class="identity-display-name">{a.display_name}</span>
                </Show>
            </div>
            <span class={STATUS_DOT[a.status] ?? STATUS_DOT["unknown"]} title={a.status} />
        </div>
    );
}

// ── Account detail panel (rendered inside <Modal>) ───────────────────────────

function AccountDetail({ model, account }: { model: IdentityViewModel; account: Account }): JSX.Element {
    // Pre-delete disclosure (spec §4): how many agents reference this
    // account. Count is by *link*, not process liveness — see
    // deleteDisclosureNotice's note.
    //
    // Source of truth is db_agent_identity_links (the table the modern
    // launch flow writes exclusively, per
    // SPEC_IDENTITY_DIRECT_LINKS_PHASE3_PRC and what the backend's own
    // affected_agents disclosure reads) — the deprecated agent.accounts
    // reverse index alone misses every modern-linked agent, which made the
    // pre-delete warning never fire for exactly the agents that matter
    // (reagent P1, PR #2161 round 1). The legacy index is kept as a union
    // for agents that predate direct links.
    const agents = useAgentDefinitions();
    const [linkedAgentIds, setLinkedAgentIds] = createSignal<string[]>([]);
    RpcApi.ListAllAgentIdentitiesCommand(TabRpcClient)
        .then((links) => {
            setLinkedAgentIds(
                [...new Set(links.filter((l) => l.account_id === account.id).map((l) => l.agent_id))]
            );
        })
        .catch(() => {
            // Best-effort: the legacy index below still contributes, and a
            // failed lookup must not block the delete flow itself — the
            // post-delete disclosure (backend-sourced) remains accurate.
        });
    const assignedAgents = () => {
        const byId = new Map(agents().map((a) => [a.id, a.name] as const));
        const names = new Set(agentsAssignedToAccount(account.id, agents()));
        for (const id of linkedAgentIds()) {
            names.add(byId.get(id) ?? id);
        }
        return [...names];
    };
    return (
        <>
            <ModalHeader title={account.name} />
            <ModalBody>
                <div class="identity-detail-meta-row">
                    <span class={`identity-provider-badge provider-${account.provider}`}>
                        <ProviderLogo provider={account.provider} size={14} />
                    </span>
                    <span class="identity-detail-meta-label">
                        <Show when={account.display_name}>
                            <span class="identity-detail-subname">{account.display_name}</span>
                        </Show>
                    </span>
                    <span class={`${STATUS_DOT[account.status] ?? STATUS_DOT["unknown"]} detail-status`} title={account.status} />
                    <span class="identity-detail-status-text" data-status={account.status}>{account.status}</span>
                </div>

                {/* Resolve the label via the brand so CLI-OAuth accounts surface "via <CLI>". */}
                <DetailField
                    label="Provider"
                    value={`${PROVIDER_LABELS[brandForProvider(account.provider)] ?? account.provider}${
                        isCliOAuthProvider(account.provider) ? ` (via ${account.provider} CLI)` : ""
                    }`}
                />
                <DetailField label="Kind" value={KIND_LABELS[account.kind]} />

                <div class="identity-detail-section">Secret</div>
                {/* `?.` guards throughout: an account whose secret_ref shape the
                    frontend doesn't know yet must degrade to "unknown", not crash
                    the pane (live repro 2026-07-14: OAuth accounts crashed here
                    before `oauth_config_dir` was mapped). */}
                <DetailField label="Backend" value={account.secret_ref?.backend ?? "unknown"} />
                <Show when={account.secret_ref?.env_var}>
                    <DetailField label="Env var" value={account.secret_ref?.env_var ?? ""} />
                </Show>
                <Show when={account.secret_ref?.sm_path}>
                    <DetailField
                        label="Secrets Manager"
                        value={`${account.secret_ref?.sm_path ?? ""}${account.secret_ref?.sm_json_path ? ` → ${account.secret_ref.sm_json_path}` : ""}`}
                    />
                </Show>
                <Show when={account.secret_ref?.backend === "plaintext_dev"}>
                    <DetailField label="Value" value="••••••••••••" />
                </Show>
                <Show when={account.secret_ref?.backend === "keychain"}>
                    <DetailField label="Stored in" value="OS keychain" />
                    <DetailField label="Key" value={account.context.masked_tail ?? "••••••••"} />
                </Show>
                <Show when={account.secret_ref?.backend === "oauth_config_dir"}>
                    <DetailField label="Stored in" value="Provider CLI config dir (tokens owned by the CLI)" />
                    <Show when={account.secret_ref?.dir}>
                        <DetailField label="Config dir" value={account.secret_ref?.dir ?? ""} />
                    </Show>
                </Show>

                <Show when={account.context.github_username}>
                    <div class="identity-detail-section">GitHub</div>
                    <DetailField label="Username" value={account.context.github_username!} />
                    <Show when={(account.context.github_scopes ?? []).length > 0}>
                        <DetailField label="Scopes" value={account.context.github_scopes!.join(", ")} />
                    </Show>
                </Show>
                <Show when={account.context.aws_profile || account.context.aws_role_arn}>
                    <div class="identity-detail-section">AWS</div>
                    <Show when={account.context.aws_profile}>
                        <DetailField label="Profile" value={account.context.aws_profile!} />
                    </Show>
                    <Show when={account.context.aws_role_arn}>
                        <DetailField label="Role ARN" value={account.context.aws_role_arn!} />
                    </Show>
                    <Show when={account.context.aws_region}>
                        <DetailField label="Region" value={account.context.aws_region!} />
                    </Show>
                </Show>
                <Show when={account.context.anthropic_model}>
                    <div class="identity-detail-section">Anthropic</div>
                    <DetailField label="Model" value={account.context.anthropic_model!} />
                </Show>
                <Show when={account.context.description}>
                    <DetailField label="Notes" value={account.context.description!} />
                </Show>

                <div class="identity-detail-section">Agents</div>
                <span class="identity-detail-empty">Assigned via Identities tab</span>

                <DetailField label="Created" value={new Date(account.created_at).toLocaleString()} />
            </ModalBody>
            <ModalFooter>
                <Show when={account.status === "expired" && account.kind !== "oauth"}>
                    <button class="identity-btn identity-btn-primary" onClick={() => model.openEditForm(account)}>
                        Reauth
                    </button>
                </Show>
                <Show when={account.status === "unknown" && account.secret_ref?.backend === "keychain"}>
                    <button class="identity-btn identity-btn-primary" onClick={() => model.openEditForm(account)}>
                        Validate…
                    </button>
                </Show>
                <button class="identity-btn identity-btn-secondary" onClick={() => model.openEditForm(account)}>
                    Edit
                </button>
                <button
                    class="identity-btn identity-btn-danger"
                    onClick={() => {
                        // Pre-delete affected-agent disclosure (spec §4):
                        // surface the linked-agent count BEFORE the delete.
                        const used = assignedAgents();
                        const usage =
                            used.length > 0
                                ? `\n\n${used.length} agent(s) use this account: ${used.join(", ")}.` +
                                  ` Any that are running keep its tokens until restarted.`
                                : "";
                        if (confirm(`Delete account "${account.name}"?${usage}`)) {
                            model.deleteAccount(account.id);
                        }
                    }}
                >
                    Delete
                </button>
            </ModalFooter>
        </>
    );
}

function DetailField({ label, value }: { label: string; value: string }): JSX.Element {
    return (
        <div class="identity-detail-field">
            <span class="identity-detail-label">{label}</span>
            <span class="identity-detail-value">{value}</span>
        </div>
    );
}
