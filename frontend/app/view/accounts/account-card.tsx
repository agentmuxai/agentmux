// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The cards Connectors' Accounts section shows for what is connected: one per
// stored account, and AgentMux Cloud's session
// (docs/specs/SPEC_CONNECTORS_CARDS_2026_10_13.md §3.5).

import { createSignal, onCleanup, type Accessor, type JSX } from "solid-js";

import { Card, type StatusTone } from "@/app/element/ui";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    accountLabel,
    parseAgentAccounts,
    PROVIDER_LABELS,
    type Account,
    type AccountKind,
    type AccountStatus,
} from "@/app/view/identity/identity-model";
import { ProviderLogo } from "@/element/ProviderLogo";
import { SERVICE_CATALOG } from "./accounts-catalog";
import type { MuxBusController } from "./AgentMuxConnectPanel";
import { isMuxBusSessionOk, muxbusNeedsSignInAgain } from "./muxbus-session";
import { brandForProvider } from "./provider-brand";

const STATUS: Record<AccountStatus, { tone: StatusTone; label: string }> = {
    valid: { tone: "ok", label: "Valid" },
    expired: { tone: "warn", label: "Expired" },
    invalid: { tone: "error", label: "Invalid" },
    checking: { tone: "busy", label: "Checking…" },
    unknown: { tone: "idle", label: "Not checked" },
};

/** Short forms of KIND_LABELS, for a card's footer. */
const KIND_SHORT: Record<AccountKind, string> = {
    oauth: "OAuth",
    api_key: "API key",
    pat: "Token",
    role: "IAM role",
    env_ref: "Env var",
};

export function accountStatus(status: AccountStatus): { tone: StatusTone; label: string } {
    return STATUS[status] ?? STATUS.unknown;
}

/** In the gallery's provider order, then by label. */
export function sortAccounts(accounts: readonly Account[]): Account[] {
    const rank = new Map(SERVICE_CATALOG.map((t, i) => [t.id as string, i]));
    const rankOf = (a: Account) => rank.get(brandForProvider(a.provider)) ?? rank.size;
    return [...accounts].sort(
        (a, b) => rankOf(a) - rankOf(b) || accountLabel(a).localeCompare(accountLabel(b)),
    );
}

/** What a card's title and subtitle say, for the filter. */
export function accountSearchText(a: Account): string[] {
    return [accountLabel(a), a.display_name ?? "", PROVIDER_LABELS[brandForProvider(a.provider)] ?? a.provider];
}

/**
 * How many agents use each account: the link rows (what launches use) plus
 * the legacy `agent.accounts` index, fetched once for every card and
 * refreshed when accounts or their bindings change.
 */
export function useAccountUseCounts(agents: Accessor<AgentDefinition[]>): Accessor<Map<string, number>> {
    const [links, setLinks] = createSignal<{ agent_id: string; account_id: string }[]>([]);
    let seq = 0;
    const load = () => {
        const mine = ++seq;
        RpcApi.ListAllAgentIdentitiesCommand(TabRpcClient)
            .then((rows) => {
                if (mine === seq) setLinks(rows.map((l) => ({ agent_id: l.agent_id, account_id: l.account_id })));
            })
            .catch(() => {});
    };
    load();
    onCleanup(onMuxEvent(WpsEvent.IdentityAccountsChanged, () => load()));
    return () => {
        const byAccount = new Map<string, Set<string>>();
        const add = (account: string, agent: string) => {
            const set = byAccount.get(account) ?? new Set<string>();
            set.add(agent);
            byAccount.set(account, set);
        };
        for (const l of links()) add(l.account_id, l.agent_id);
        for (const agent of agents()) {
            for (const id of Object.values(parseAgentAccounts(agent))) {
                if (id) add(id, agent.id);
            }
        }
        return new Map([...byAccount].map(([k, v]) => [k, v.size]));
    };
}

export function AccountCard(props: {
    account: Account;
    usedBy: number;
    selected: boolean;
    onSelect: () => void;
    onContextMenu: (e: MouseEvent) => void;
}): JSX.Element {
    const a = () => props.account;
    const brand = () => PROVIDER_LABELS[brandForProvider(a().provider)] ?? a().provider;
    const status = () => accountStatus(a().status);
    const meta = () => {
        const out: JSX.Element[] = [<span>{KIND_SHORT[a().kind] ?? a().kind}</span>];
        if (props.usedBy > 0) out.push(<span>{`Used by ${props.usedBy} ${props.usedBy === 1 ? "agent" : "agents"}`}</span>);
        return out;
    };
    return (
        <Card
            id={`identity-account-row-${a().id}`}
            class="account-card"
            aria-label={`${accountLabel(a())}, ${brand()}, ${status().label}`}
            aria-controls={`identity-account-panel-${a().id}`}
            title={accountLabel(a())}
            subtitle={a().display_name || brand()}
            icon={<ProviderLogo provider={a().provider} size={18} />}
            status={status()}
            attention={a().status === "invalid"}
            meta={meta()}
            selected={props.selected}
            onSelect={props.onSelect}
            onContextMenu={(e) => props.onContextMenu(e)}
            data-account-id={a().id}
        />
    );
}

export function AgentMuxCard(props: { status: ReturnType<MuxBusController["status"]>; onSelect: () => void }): JSX.Element {
    const state = (): { tone: StatusTone; label: string } => {
        const s = props.status;
        if (isMuxBusSessionOk(s)) return { tone: "ok", label: "Connected" };
        if (muxbusNeedsSignInAgain(s)) return { tone: "warn", label: "Sign in again" };
        return { tone: "warn", label: "Token expired" };
    };
    return (
        <Card
            class="account-card account-card--agentmux"
            aria-label={`AgentMux Cloud, ${state().label}`}
            title={props.status?.email || "AgentMux Cloud"}
            subtitle="AgentMux Cloud"
            icon={<ProviderLogo provider="agentmux" size={18} />}
            status={state()}
            meta={[<span>Cloud session</span>]}
            onSelect={props.onSelect}
        />
    );
}
