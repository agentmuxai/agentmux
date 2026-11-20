// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How much of an account's usage window is used: one line per window with a thin bar, in the
// accounts view (an account's panel) and at the top of the Swarm (a strip with one block per
// account that reports). Read-only, no modal.

import { For, Show, type JSX } from "solid-js";

import { t } from "@/app/i18n/i18n";
import {
    accountsWithReading,
    ageNote,
    barPercent,
    noReadingText,
    useAccountUsage,
    usageTone,
    windowName,
    windowsOf,
    windowText,
} from "@/app/store/account-usage";
import type { AccountUsage } from "@/types/rpc/AccountUsage";

import "./account-usage.scss";

/** An account's windows, or why there are none. */
export function AccountUsageRows(props: { usage: AccountUsage }): JSX.Element {
    return (
        <Show
            when={windowsOf(props.usage).length > 0}
            fallback={<span class="account-usage__none">{noReadingText(props.usage)}</span>}
        >
            <ul class="account-usage" aria-label={t("accountUsage.rows.aria")}>
                <For each={windowsOf(props.usage)}>
                    {([key, w]) => (
                        <li class="account-usage__row" data-tone={usageTone(w)}>
                            <span class="account-usage__name" title={windowName(key)}>
                                {windowName(key)}
                            </span>
                            <span
                                class="account-usage__bar"
                                role="progressbar"
                                aria-label={t("accountUsage.window.aria", { name: windowName(key) })}
                                aria-valuemin={0}
                                aria-valuemax={100}
                                aria-valuenow={barPercent(w)}
                            >
                                <span class="account-usage__fill" style={{ width: `${barPercent(w)}%` }} />
                            </span>
                            <span class="account-usage__text">
                                {windowText(w)}
                                <Show when={ageNote(w)}>{(n) => <span class="account-usage__age">{n()}</span>}</Show>
                            </span>
                        </li>
                    )}
                </For>
            </ul>
        </Show>
    );
}

/** An account's usage by this machine's account id, in the accounts view's panel. */
export function AccountUsageField(props: { accountId: string }): JSX.Element {
    const accounts = useAccountUsage();
    return (
        <Show when={accounts().find((a) => a.account_id === props.accountId)}>
            {(a) => (
                <div class="identity-detail-field account-usage__field">
                    <span class="identity-detail-label">{t("accountUsage.field")}</span>
                    <AccountUsageRows usage={a().usage} />
                </div>
            )}
        </Show>
    );
}

/** At the top of the Swarm: every account that reports its usage, or nothing. */
export function AccountUsageStrip(): JSX.Element {
    const accounts = useAccountUsage();
    const shown = () => accountsWithReading(accounts());
    return (
        <Show when={shown().length > 0}>
            <div class="account-usage-strip" role="group" aria-label={t("accountUsage.strip.aria")}>
                <For each={shown().map((a) => a.account_id)}>
                    {(id) => (
                        <Show when={shown().find((a) => a.account_id === id)}>
                            {(a) => (
                                <div class="account-usage-strip__account">
                                    <span class="account-usage-strip__label">{a().label}</span>
                                    <AccountUsageRows usage={a().usage} />
                                </div>
                            )}
                        </Show>
                    )}
                </For>
            </div>
        </Show>
    );
}
