// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How much of an account's usage window is used: one line per window with a thin bar, then where the
// numbers come from and how old they are, in the accounts view (an account's panel). Read-only, no modal.

import { For, Show, type JSX } from "solid-js";

import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";
import {
    ageText,
    barPercent,
    freshestAgeMs,
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
            <div class="account-usage__wrap">
                <ul class="account-usage" aria-label={t("accountUsage.rows.aria")}>
                    <For each={windowsOf(props.usage)}>
                        {([key, w]) => (
                            <li class="account-usage__row" data-tone={usageTone(w)}>
                                <span class="account-usage__name" {...tip(windowName(key))}>
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
                                <span class="account-usage__text">{windowText(w)}</span>
                            </li>
                        )}
                    </For>
                </ul>
                <span class="account-usage__source">{t("accountUsage.source", { age: ageText(freshestAgeMs(props.usage)) })}</span>
            </div>
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
