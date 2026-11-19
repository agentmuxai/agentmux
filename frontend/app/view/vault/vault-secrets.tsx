// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The oversight half of the Vault, in the Audit pane
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.6): what
// agents did with the person's secrets, and where they may use them without asking,
// with a way to stop each. The entries themselves are in the Vault pane, which
// this links to, and which links back.

import { createEffect, createSignal, For, on, Show, type JSX } from "solid-js";

import { Button, CloseButton } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { VaultActivity } from "@/types/rpc/VaultActivity";
import { formatWhen } from "./vault-format";
import { openVault } from "./open-vault";
import { grantText } from "./vault-grant-labels";
import { loadActivity, loadGrants, revokeGrant, vaultAvailable, vaultChangedMs, type GrantedEntry } from "./vault-store";

/** How many of the agents' doings to list at first, and how many more each "Show more" adds. */
const PAGE = 50;
/** How many of the Vault's newest activity lines are read first, and the most that "Show more" reads. */
const WINDOW = 500;
const WINDOW_MAX = 8000;

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
        setShown((n) => n + PAGE);
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
            <Show when={granted().length > 0}>
                <table class="warden-manager-table" aria-label="Standing permissions">
                    <thead>
                        <tr>
                            <th>Entry</th>
                            <th>May be used without asking</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each={granted()}>
                            {(e) => (
                                <tr data-entry={e.id}>
                                    <td>{e.label}</td>
                                    <td>
                                        <For each={e.grants}>
                                            {(g) => (
                                                <span class="vault-grant" data-origin={g}>
                                                    {grantText(g)}
                                                    <CloseButton size="sm" label={`Stop ${grantText(g)} for ${e.label}`} onClick={() => void revoke(e.id, g)} />
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
            <Show when={(rows() ?? []).length > 0} fallback={<div class="warden-section-stub">{rows() === null ? "Loading…" : "No agent has used a secret yet."}</div>}>
                <table class="warden-manager-table" aria-label="Agents' use of secrets">
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
                                    <td class="warden-manager-dim">{formatWhen(a.at)}</td>
                                    <td class="warden-manager-mono">{a.actor}</td>
                                    <td>{a.action}</td>
                                    <td>{a.entry_label}</td>
                                    <td class="warden-manager-dim">{a.target}</td>
                                    <td>{a.outcome}</td>
                                </tr>
                            )}
                        </For>
                    </tbody>
                </table>
                <p class="warden-manager-dim vault-secrets__coverage">
                    {t("vault.secrets.coverage", { shown: rows()?.length ?? 0, found: found()?.length ?? 0, lines: lines() })}{" "}
                    <Show when={readFull() && !moreToRead() && !moreLoaded()}>{t("vault.secrets.olderInVault")}</Show>
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
