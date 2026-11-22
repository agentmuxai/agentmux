// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The oversight half of the Vault, in the Audit pane
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.6): what
// agents did with the person's secrets, and where they may use them without asking,
// with a way to stop each. The entries themselves are in the Vault pane, which
// this links to, and which links back.

import { agentNameOf } from "./vault-history";
import { createEffect, createSignal, For, on, Show, type JSX } from "solid-js";

import { Button, CloseButton } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultActivity } from "@/types/rpc/VaultActivity";
import { formatWhen } from "./vault-format";
import { openVault } from "./open-vault";
import { createAgentNames } from "./vault-agent-names";
import { grantText, SIGNIN_GRANT } from "./vault-grant-labels";
import { loadActivity, loadGrants, revokeGrant, vaultAvailable, vaultChangedMs, type GrantedEntry } from "./vault-store";

/** How many of the agents' doings to list at first, and how many more each "Show more" adds. */
const PAGE = 50;
/** How many of the Vault's newest activity lines are read first, and the most that "Show more" reads: the
 *  host answers at most 1000 lines to one read (`vault_activity_list(limit.clamp(1, 1000))`), so asking for
 *  more would claim a reach the list does not have. */
const WINDOW = 500;
const WINDOW_MAX = 1000;

/** What an agent asks for rather than does: a request for a new entry, and a request to be given one. */
const ASKS = ["requested an entry", "asked to be given it"];
/** The agent doings an older line (made before the Vault kept who did it) is known by. */
const LEGACY_USES = ["filled a web form", "signed in", "ran a command", "generated"];

/** Everything an agent did with an entry: a fill, a sign-in, a command, a generated value, a capture, a
 *  rotation, an import, a share. The person's own doings (saving, revealing, copying) and the app's own
 *  (a reminder, a check) are the Vault's to show, and so is an agent's mere request for an entry. A line
 *  made before the Vault kept who did it has no kind, so it counts only if it is one of the old uses. */
export const isAgentUse = (a: VaultActivity): boolean => {
    if (ASKS.includes(a.action)) return false;
    if (a.actor_kind) return a.actor_kind === "agent";
    return a.actor !== "you" && LEGACY_USES.includes(a.action);
};

/** No agent doing among the lines read. Only when the read was short is that "none yet": a full read has
 *  older lines it did not look at, so it says how far back it looked and offers to look further. */
function NoUses(props: { loading: boolean; readFull: boolean; lines: number; canReadMore: boolean; onMore: () => void }): JSX.Element {
    return (
        <div class="warden-section-stub">
            <Show when={!props.loading} fallback="Loading…">
                <Show when={props.readFull} fallback="No agent has used a secret yet.">
                    {t("vault.secrets.noneInWindow", { lines: props.lines })}{" "}
                    <Show when={props.canReadMore} fallback={t("vault.secrets.olderNotRead")}>
                        <Button density="compact" onClick={props.onMore}>
                            {t("vault.secrets.more")}
                        </Button>
                    </Show>
                </Show>
            </Show>
        </div>
    );
}

/** What the list reads and shows: the agents' doings from the newest `lines` lines of the Vault's activity,
 *  `shown` of them at a time, and the standing permissions. Neither read opens the Vault key, so showing
 *  this never makes the OS ask for it (the entries' list, which decrypts, is the Vault section's alone). */
function useAgentUses(visible: () => boolean) {
    const [found, setFound] = createSignal<VaultActivity[] | null>(null);
    const [granted, setGranted] = createSignal<GrantedEntry[]>([]);
    const [error, setError] = createSignal<string | null>(null);
    // How many lines of the Vault's activity are read, and how many agent doings of them are shown.
    const [lines, setLines] = createSignal(WINDOW);
    const [shown, setShown] = createSignal(PAGE);
    // The read came back full, so older lines exist that were not read.
    const [readFull, setReadFull] = createSignal(false);
    const rows = () => found()?.slice(0, shown()) ?? null;
    const moreLoaded = () => (found()?.length ?? 0) > shown();
    const moreToRead = () => readFull() && lines() < WINDOW_MAX;
    const load = () =>
        Promise.all([loadActivity(lines()), loadGrants()])
            .then(([activity, grants]) => {
                setFound(activity.filter(isAgentUse));
                setReadFull(activity.length >= lines());
                setGranted(grants);
                setError(null);
            })
            .catch((e) => setError(String(e)));
    const showMore = () => {
        // Every doing already read is on screen: read further back as well.
        const readMore = !moreLoaded() && moreToRead();
        // 50 more than are on screen now, never than were asked for: the count does not jump after a read further back.
        setShown((n) => Math.min(n, found()?.length ?? 0) + PAGE);
        if (readMore) {
            setLines((n) => Math.min(n * 2, WINDOW_MAX));
            void load();
        }
    };
    // When first shown, and again whenever the Vault changes while shown (a use is recorded,
    // a grant is made or revoked).
    createEffect(
        on([visible, vaultChangedMs], ([isShown]) => {
            if (isShown) void load();
        })
    );
    return { found, rows, granted, error, setError, lines, load, showMore, moreLoaded, moreToRead, readFull, canShowMore: () => moreLoaded() || moreToRead() };
}

/** Where each entry may be used without asking, with a button to stop each. */
function StandingGrants(props: { granted: GrantedEntry[]; revoke: (entryId: string, grant: string) => void | Promise<void> }): JSX.Element {
    // A shared login's sign-in grant holds the agent's id; any entry's share list names every agent.
    const names = createAgentNames(() => props.granted.find((e) => e.grants.some((g) => g.startsWith(SIGNIN_GRANT)))?.id ?? false);
    return (
        <Show when={props.granted.length > 0}>
            <table class="warden-manager-table" aria-label="Standing permissions">
                <thead>
                    <tr>
                        <th>Entry</th>
                        <th>May be used without asking</th>
                    </tr>
                </thead>
                <tbody>
                    <For each={props.granted}>
                        {(e) => (
                            <tr data-entry={e.id}>
                                <td>{e.label}</td>
                                <td>
                                    <For each={e.grants}>
                                        {(g) => (
                                            <span class="vault-grant" data-origin={g}>
                                                {grantText(g, names)}
                                                <CloseButton size="sm" label={`Stop ${grantText(g, names)} for ${e.label}`} onClick={() => void props.revoke(e.id, g)} />
                                            </span>
                                        )}
                                    </For>
                                </td>
                            </tr>
                        )}
                    </For>
                </tbody>
            </table>
        </Show>
    );
}

/** `visible`: this is the Audit section's shown. Opening the Vault's list opens its key and can ask the
 *  person (a keychain, a locked keyring), so nothing is loaded until the section is looked at. */
export function VaultSecrets(props: { visible?: boolean }): JSX.Element {
    if (!vaultAvailable()) return <></>;
    const { found, rows, granted, error, setError, lines, load, showMore, moreLoaded, moreToRead, readFull, canShowMore } = useAgentUses(() => props.visible ?? true);

    const revoke = async (entryId: string, origin: string) => {
        try {
            await revokeGrant(entryId, origin);
            await load();
        } catch (e) {
            setError(String(e));
        }
    };
    return (
        <section class="vault-secrets" aria-label="Secrets">
            <h3 class="warden-manager-summary">Secrets: what agents did with them</h3>
            <p class="warden-manager-dim">
                Every use of a Vault entry by an agent, and where each may use one without asking. Never a value.{" "}
                <Button density="compact" onClick={() => void openVault("activity")}>
                    Open the Vault
                </Button>
            </p>
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <StandingGrants granted={granted()} revoke={revoke} />
            <Show
                when={(rows() ?? []).length > 0}
                fallback={<NoUses loading={rows() === null} readFull={readFull()} lines={lines()} canReadMore={moreToRead()} onMore={showMore} />}
            >
                <table class="warden-manager-table vault-uses" aria-label="Agents' use of secrets">
                    <thead>
                        <tr>
                            <th>When</th>
                            <th>Agent</th>
                            <th>What</th>
                            <th>Entry</th>
                            <th>Where</th>
                            <th>Outcome</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={rows()}>
                            {(a) => (
                                <tr data-outcome={a.outcome}>
                                    <td class="warden-manager-dim vault-uses__when">{formatWhen(a.at)}</td>
                                    <td class="warden-manager-mono vault-uses__agent">{agentNameOf(a.actor)}</td>
                                    <td class="vault-uses__what">{a.action}</td>
                                    <td class="vault-uses__entry">{a.entry_label}</td>
                                    <td class="warden-manager-dim vault-uses__where" data-label={t("vault.activity.where")}>
                                        {a.target}
                                    </td>
                                    <td class="vault-uses__outcome" data-label={t("vault.secrets.outcome")}>
                                        {a.outcome}
                                    </td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
                <p class="warden-manager-dim vault-secrets__coverage">
                    {/* A read that came back short has read the Vault's whole activity: no window to name. */}
                    {readFull()
                        ? t("vault.secrets.coverage", { shown: rows()?.length ?? 0, found: found()?.length ?? 0, lines: lines() })
                        : t("vault.secrets.coverageAll", { shown: rows()?.length ?? 0, found: found()?.length ?? 0 })}{" "}
                    <Show when={readFull() && !moreToRead() && !moreLoaded()}>{t("vault.secrets.olderNotRead")}</Show>
                    <Show when={canShowMore()}>
                        <Button density="compact" onClick={showMore}>
                            {t("vault.secrets.more")}
                        </Button>
                    </Show>
                </p>
            </Show>
        </section>
    );
}
