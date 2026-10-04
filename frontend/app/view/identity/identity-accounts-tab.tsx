// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, For, on, Show, type Accessor, type JSX } from "solid-js";
import type { Account, IdentityViewModel } from "./identity-model";
import { accountLabel, agentsAssignedToAccount, KIND_LABELS, PROVIDER_LABELS } from "./identity-model";
import { useAgentDefinitions } from "@/app/view/agent/components/AgentPicker";
import { ProviderLogo } from "@/element/ProviderLogo";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { brandForProvider, isCliOAuthProvider } from "@/app/view/accounts/provider-brand";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { buildAccountRowMenu } from "./bind-to-agent-menu";
import { ConfirmModal } from "@/app/element/confirm-modal";
import "./identity-view.scss";

// CLI providers whose config dir holds conversation history that a delete
// keeps: those with a `history_native_subdir` in
// crates/srv/src/backend/providers.rs (Copilot has none).
const PROVIDERS_WITH_KEPT_HISTORY = new Set(["claude", "codex", "gemini"]);

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
    const [agents] = useAgentDefinitions();

    // Right-click an account row → "Bind to Agent" submenu, Copy account ID
    // (SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md) and Delete
    // account… (SPEC_ARMORY_ACCOUNTS_DELETE_AND_INLINE_DETAIL_2026_10_04.md
    // §3.1). Menu contents are computed fresh per open (links + open-pane
    // snapshot), so the binding annotations are current without any
    // subscription here. The account list itself re-renders via the shared
    // cache's live sync (identityaccounts:changed → #2474) after a bind.
    // Latest-right-click-wins: buildAccountRowMenu awaits an RPC, so a
    // second right-click on a different row before the first resolves could
    // otherwise let the earlier request's menu win the race and open for
    // the wrong account (reagentx P2 on #2485).
    let menuSeq = 0;
    const handleRowContextMenu = (account: Account, e: MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        const seq = ++menuSeq;
        void buildAccountRowMenu(
            account,
            agents(),
            model.accountsAtom(),
            (msg) => model.showNotice(msg),
            undefined,
            () => model.requestDelete(account),
        ).then((items) => {
            if (seq !== menuSeq) return; // superseded by a later right-click
            ContextMenuModel.showContextMenu(items, e);
        });
    };

    // A click on the open row closes its panel; on another row, moves it.
    const toggle = (account: Account) =>
        model.setSelectedAccount(model.selectedAccountAtom()?.id === account.id ? null : account);

    return (
        <>
            {/* Delete-time notice (layer 4 —
                SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §4, plus the
                cleanup outcome and delete errors). Transient, dismissable.
                Honest wording: running processes keep their tokens until
                restarted — we disclose, not revoke. */}
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
                                        {(account) => {
                                            const expanded = () => model.selectedAccountAtom()?.id === account.id;
                                            return (
                                                <>
                                                    <AccountRow
                                                        account={account}
                                                        expanded={expanded()}
                                                        onClick={() => toggle(account)}
                                                        onContextMenu={(e) => handleRowContextMenu(account, e)}
                                                    />
                                                    {/* The details slide open under their row
                                                        instead of in a modal (spec §3.2). Reads
                                                        the selected account so an update from
                                                        elsewhere shows in place. */}
                                                    <Show when={expanded() && model.selectedAccountAtom()} keyed>
                                                        {(selected) => (
                                                            <AccountDetailPanel model={model} account={selected} />
                                                        )}
                                                    </Show>
                                                </>
                                            );
                                        }}
                                    </For>
                                </div>
                            )}
                        </For>
                    </Show>
                </div>
            </div>

            <Show when={model.pendingDeleteAtom()} keyed>
                {(account) => <DeleteAccountConfirm model={model} account={account} />}
            </Show>
        </>
    );
}

function AccountRow(props: {
    account: Account;
    expanded: boolean;
    onClick: () => void;
    onContextMenu?: (e: MouseEvent) => void;
}): JSX.Element {
    const a = props.account;
    return (
        <div
            id={`identity-account-row-${a.id}`}
            class={`identity-account-row${props.expanded ? " selected" : ""}`}
            role="button"
            tabIndex={0}
            aria-expanded={props.expanded}
            aria-controls={`identity-account-panel-${a.id}`}
            onClick={props.onClick}
            onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    props.onClick();
                }
            }}
            onContextMenu={(e) => props.onContextMenu?.(e)}
        >
            <span class="identity-row-chevron" aria-hidden="true">
                {props.expanded ? "▾" : "▸"}
            </span>
            <span class={`identity-provider-badge provider-${a.provider}`}>
                <ProviderLogo provider={a.provider} size={16} />
            </span>
            {/* The login email is the row's label (spec §3): it is what tells
                two accounts on one provider apart. The generic name
                (`claude-oauth`) stays in the tooltip, and a user-set display
                name still shows beside it. */}
            <span class="identity-account-name" title={a.name}>
                {accountLabel(a)}
            </span>
            <div class="identity-row-meta">
                <Show when={a.display_name}>
                    <span class="identity-display-name">{a.display_name}</span>
                </Show>
            </div>
            <span class={STATUS_DOT[a.status] ?? STATUS_DOT["unknown"]} title={a.status} />
        </div>
    );
}

/**
 * Names of the agents that use `account`: its link rows
 * (db_agent_identity_links, what the launch flow writes and what the
 * backend's affected-agents disclosure reads) plus the legacy
 * `agent.accounts` index for agents that predate direct links (reagent P1,
 * PR #2161 round 1). Best-effort: a failed link lookup leaves the legacy
 * part, and the post-delete disclosure (backend-sourced) stays accurate.
 */
function useAccountAgentNames(account: Account): Accessor<string[]> {
    const [agents] = useAgentDefinitions();
    const [linkedAgentIds, setLinkedAgentIds] = createSignal<string[]>([]);
    RpcApi.ListAllAgentIdentitiesCommand(TabRpcClient)
        .then((links) => {
            setLinkedAgentIds([...new Set(links.filter((l) => l.account_id === account.id).map((l) => l.agent_id))]);
        })
        .catch(() => {});
    return () => {
        const byId = new Map(agents().map((a) => [a.id, a.name] as const));
        const names = new Set(agentsAssignedToAccount(account.id, agents()));
        for (const id of linkedAgentIds()) {
            names.add(byId.get(id) ?? id);
        }
        return [...names];
    };
}

// ── Account detail panel (opens under its row) ───────────────────────────────

function AccountDetailPanel({ model, account }: { model: IdentityViewModel; account: Account }): JSX.Element {
    const usedBy = useAccountAgentNames(account);
    let panelRef: HTMLDivElement | undefined;

    // Bring the whole panel into view without jumping its row to the top.
    // Not scrollIntoView: it also scrolls every scrollable ancestor (see
    // REPORT_LAN_UNDISCOVERABLE_AND_PAGE_SHIFT_2026_10_04.md §3).
    createEffect(
        on(
            () => account.id,
            () =>
                requestAnimationFrame(() => {
                    const scroller = findScrollParent(panelRef);
                    const row = document.getElementById(`identity-account-row-${account.id}`);
                    if (!panelRef || !scroller || !row) return;
                    const p = panelRef.getBoundingClientRect();
                    const s = scroller.getBoundingClientRect();
                    if (p.bottom <= s.bottom) return;
                    // Scroll so the panel's bottom shows, but never past the
                    // row's top. Rects are zoomed viewport px; scrollTop is
                    // the scroller's own px.
                    const zoom = scroller.offsetHeight > 0 ? s.height / scroller.offsetHeight : 1;
                    const delta = Math.min(p.bottom - s.bottom, row.getBoundingClientRect().top - s.top);
                    if (delta > 0) scroller.scrollTop += delta / zoom;
                }),
        ),
    );

    return (
        <div
            ref={panelRef}
            id={`identity-account-panel-${account.id}`}
            class="identity-account-panel"
            role="region"
            aria-labelledby={`identity-account-row-${account.id}`}
            onKeyDown={(e) => {
                if (e.key === "Escape") {
                    e.stopPropagation();
                    model.setSelectedAccount(null);
                    document.getElementById(`identity-account-row-${account.id}`)?.focus();
                }
            }}
        >
            <div class="identity-detail-meta-row">
                <span class="identity-detail-meta-label">
                    <Show when={account.display_name}>
                        <span class="identity-detail-subname">{account.display_name}</span>
                    </Show>
                </span>
                <span class={`${STATUS_DOT[account.status] ?? STATUS_DOT["unknown"]} detail-status`} title={account.status} />
                <span class="identity-detail-status-text" data-status={account.status}>{account.status}</span>
            </div>

            {/* Resolve the label via the brand so CLI-OAuth accounts surface "via <CLI>" —
                except Claude, whose underlying CLI tool ("Claude Code") is an
                implementation detail this surface deliberately doesn't expose;
                "Anthropic" (the actual account/brand) is shown alone instead. */}
            <DetailField
                label="Provider"
                value={`${PROVIDER_LABELS[brandForProvider(account.provider)] ?? account.provider}${
                    isCliOAuthProvider(account.provider) && brandForProvider(account.provider) !== "anthropic"
                        ? ` (via ${account.provider} CLI)`
                        : ""
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

            <div class="identity-detail-section">Used by</div>
            <Show
                when={usedBy().length > 0}
                fallback={<span class="identity-detail-empty">No agents</span>}
            >
                <div class="identity-agent-chips">
                    <For each={usedBy()}>{(name) => <span class="identity-agent-chip">{name}</span>}</For>
                </div>
            </Show>

            <DetailField label="Created" value={new Date(account.created_at).toLocaleString()} />

            <div class="identity-account-panel-actions">
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
                <button class="identity-btn identity-btn-danger" onClick={() => model.requestDelete(account)}>
                    Delete account…
                </button>
            </div>
        </div>
    );
}

function findScrollParent(el: HTMLElement | undefined): HTMLElement | null {
    let cur = el?.parentElement ?? null;
    while (cur) {
        const oy = getComputedStyle(cur).overflowY;
        if ((oy === "auto" || oy === "scroll") && cur.scrollHeight > cur.clientHeight) return cur;
        cur = cur.parentElement;
    }
    return null;
}

// ── Delete confirmation (spec §3.3) ──────────────────────────────────────────

function DeleteAccountConfirm({ model, account }: { model: IdentityViewModel; account: Account }): JSX.Element {
    const usedBy = useAccountAgentNames(account);
    const backend = account.secret_ref?.backend;
    const keepsHistory = backend === "oauth_config_dir" && PROVIDERS_WITH_KEPT_HISTORY.has(account.provider);
    const isKey = backend === "keychain" || backend === "env" || backend === "secrets_manager";
    const providerName = PROVIDER_LABELS[brandForProvider(account.provider)] ?? account.provider;
    const editing = () => model.formOpenAtom() && model.editingAccountAtom()?.id === account.id;
    return (
        <ConfirmModal
            open={true}
            scope="tab"
            title={`Delete ${accountLabel(account)}?`}
            confirmLabel="Delete account"
            destructive
            onConfirm={() => model.confirmDelete()}
            onCancel={() => model.cancelDelete()}
        >
            <div class="identity-delete-confirm">
                <p>AgentMux forgets this account and removes its saved login from this computer.</p>
                <Show when={usedBy().length > 0}>
                    <p>
                        Used by {usedBy().length === 1 ? "1 agent" : `${usedBy().length} agents`}: {usedBy().join(", ")}.
                        They won't start until you bind another account. Any that are running keep working until they
                        restart.
                    </p>
                </Show>
                <Show when={keepsHistory}>
                    <p>Its conversation history is not deleted.</p>
                </Show>
                <Show when={isKey}>
                    <p>The key itself still works. To revoke it, do that at {providerName}.</p>
                </Show>
                <Show when={editing()}>
                    <p>Unsaved changes to this account will be lost.</p>
                </Show>
            </div>
        </ConfirmModal>
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
