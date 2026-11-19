// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What Connectors' one list shows: a card per connector, from the service catalog, the stored accounts and the AgentMux
// Cloud session (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4.2, §4.3). Pure: the components only draw it.

import { t } from "@/app/i18n/i18n";
import type { StatusTone } from "@/app/element/ui";
import { matchesEveryWord } from "@/app/util/fuzzysearch";
import { accountSearchText, cardStatus, sortAccounts } from "@/app/view/accounts/account-card";
import { SERVICE_CATALOG, type ServiceTile } from "@/app/view/accounts/accounts-catalog";
import { isMuxBusSessionOk, muxbusNeedsSignInAgain } from "@/app/view/accounts/muxbus-session";
import { canReconnect } from "@/app/view/accounts/oauth-account-state";
import { brandForProvider } from "@/app/view/accounts/provider-brand";
import { accountLabel, type Account } from "@/app/view/identity/identity-model";

/** What the AgentMux Cloud session says about itself (`muxbus.status`). */
export interface CloudSession {
    connected: boolean;
    valid: boolean;
    needsReauth?: boolean;
    email?: string;
}

/** The one control at the card's right (§4.2). */
export type ConnectorControl = "connect" | "howto" | "open" | "reconnect";

export interface ConnectorEntry {
    /** The catalog tile's id; the card's key. */
    key: string;
    tile: ServiceTile;
    name: string;
    /** What it is for, one line. */
    line: string;
    /** This connector's stored accounts, in the list's order. Empty for AgentMux Cloud, which is no stored account. */
    accounts: Account[];
    connected: boolean;
    /** The state's tone; `idle` before anything is connected. */
    tone: StatusTone;
    /** The state as words, never colour alone; `null` before anything is connected. */
    state: string | null;
    control: ConnectorControl;
    /** The account the card's Reconnect re-runs, when exactly one needs it. */
    reconnect?: Account;
    /** No way to connect from here: the agent signs in inside its own pane (xAI, Moonshot). */
    textOnly: boolean;
}

/** AgentMux Cloud's card is named for the service, not the catalog's short name. */
function connectorName(tile: ServiceTile): string {
    return tile.id === "agentmux" ? t("connectors.name.agentmux") : tile.displayName;
}

/** One plain line of what the connector is for. */
export function connectorLine(id: string): string {
    switch (id) {
        case "agentmux":
            return t("connectors.line.agentmux");
        case "github":
            return t("connectors.line.github");
        case "google":
            return t("connectors.line.google");
        case "aws":
            return t("connectors.line.aws");
        case "openai":
            return t("connectors.line.openai");
        case "anthropic":
            return t("connectors.line.anthropic");
        case "xai":
            return t("connectors.line.xai");
        case "kimi":
            return t("connectors.line.kimi");
        case "slack":
            return t("connectors.line.slack");
        case "custom":
            return t("connectors.line.custom");
        default:
            return "";
    }
}

const WORST: Record<StatusTone, number> = { ok: 0, idle: 1, busy: 2, warn: 3, error: 4 };

/** One account's state in words, and its tone. */
export function accountState(a: Pick<Account, "id" | "kind" | "status">): { tone: StatusTone; word: string } {
    const s = cardStatus(a);
    switch (s.tone) {
        case "ok":
            return { tone: "ok", word: t("connectors.state.connected") };
        case "warn":
            return { tone: "warn", word: t("connectors.state.needsSignIn") };
        case "error":
            return { tone: "error", word: s.label === "Invalid" ? t("connectors.state.failed") : t("connectors.state.blocked") };
        case "busy":
            return { tone: "busy", word: t("connectors.state.checking") };
        default:
            return { tone: "idle", word: t("connectors.state.notChecked") };
    }
}

/** The state of a connector holding `accounts`: one account says who, several say how many and what is wrong. */
function accountsState(accounts: readonly Account[]): { tone: StatusTone; state: string } {
    const states = accounts.map((a) => accountState(a));
    const tone = states.reduce<StatusTone>((worst, s) => (WORST[s.tone] > WORST[worst] ? s.tone : worst), "ok");
    if (accounts.length === 1) {
        return { tone, state: t("connectors.state.withWho", { state: states[0].word, who: accountLabel(accounts[0], accounts) }) };
    }
    const signIn = states.filter((s) => s.tone === "warn").length;
    const failed = states.filter((s) => s.tone === "error").length;
    const extra = [
        signIn > 0 ? t("connectors.state.alsoSignIn", { count: signIn }) : "",
        failed > 0 ? t("connectors.state.alsoFailed", { count: failed }) : "",
    ].filter(Boolean);
    const many = t("connectors.state.accounts", { count: accounts.length });
    return { tone, state: extra.length > 0 ? `${many} · ${extra.join(" · ")}` : many };
}

function cloudState(s: CloudSession): { tone: StatusTone; state: string; signIn: boolean } {
    if (isMuxBusSessionOk(s)) {
        const who = s.email ? t("connectors.state.withWho", { state: t("connectors.state.connected"), who: s.email }) : t("connectors.state.connected");
        return { tone: "ok", state: who, signIn: false };
    }
    return { tone: "warn", state: t("connectors.state.needsSignIn"), signIn: muxbusNeedsSignInAgain(s) || !s.valid };
}

/**
 * Every connector as a card entry: Connected first (AgentMux Cloud, then the catalog's order), then Available in the
 * catalog's order with the text-only agents last (§3, D6). Not by use, so cards don't reshuffle.
 */
export function connectorEntries(accounts: readonly Account[], cloud: CloudSession | null | undefined): ConnectorEntry[] {
    const sorted = sortAccounts(accounts);
    const entries = SERVICE_CATALOG.map((tile): ConnectorEntry => {
        const mine = tile.id === "agentmux" ? [] : sorted.filter((a) => brandForProvider(a.provider) === tile.id);
        const base = {
            key: tile.id,
            tile,
            name: connectorName(tile),
            line: connectorLine(tile.id),
            accounts: mine,
            textOnly: tile.id !== "agentmux" && tile.authModes.length === 0,
        };
        if (tile.id === "agentmux" && cloud?.connected) {
            const c = cloudState(cloud);
            return { ...base, connected: true, tone: c.tone, state: c.state, control: c.signIn ? "reconnect" : "open" };
        }
        if (mine.length > 0) {
            const s = accountsState(mine);
            const needing = mine.filter((a) => accountState(a).tone !== "ok" && canReconnect(a));
            const reconnect = mine.length === 1 && needing.length === 1 ? needing[0] : undefined;
            return { ...base, connected: true, tone: s.tone, state: s.state, control: reconnect ? "reconnect" : "open", reconnect };
        }
        return { ...base, connected: false, tone: "idle", state: null, control: base.textOnly ? "howto" : "connect" };
    });
    const rank = (e: ConnectorEntry) => (e.connected ? 0 : e.textOnly ? 2 : 1);
    return entries
        .map((e, i) => ({ e, i }))
        .sort((a, b) => rank(a.e) - rank(b.e) || a.i - b.i)
        .map((x) => x.e);
}

/** Whether a card matches every word of the filter: its name, its line, its state, its accounts. */
export function connectorMatches(e: ConnectorEntry, query: string, cloudEmail?: string): boolean {
    const accounts = e.accounts.flatMap((a) => accountSearchText(a, e.accounts));
    return matchesEveryWord(query, e.name, e.line, e.state ?? "", cloudEmail ?? "", ...accounts);
}
