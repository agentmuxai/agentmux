// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// reactive-coverage: one-shot the account list is read once when the panel opens; the guide is a first-run hint

/** Whether the Create step shows the free sign-up guide: only to someone new to the provider. */

import { createResource } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { checkCliAuth } from "../flows/login-and-confirm";
import { checkProviderInstalled } from "../install-check-cache";
import { getProvider } from "../providers";
import type { PanelState } from "./create-panel-state";

export function useFreeGuide(s: PanelState) {
    // The free sign-up guide is for someone new to this provider: not shown once an
    // account for it exists. It waits for the list, and stays away when the list
    // can't be read, so a person with an account never sees it flash in, and a failed
    // fetch doesn't guess. Asked of the RPC directly: `refreshAccountCache` swallows a
    // failure and returns its (possibly empty) cache, which would read as "no account".
    const [accounts] = createResource(async () => {
        try {
            return await RpcApi.ListIdentityAccountsCommand(TabRpcClient, {});
        } catch {
            return null;
        }
    });
    // A provider whose CLI keeps its own login (Antigravity) never gets an account, so
    // the list can't tell a newcomer from someone signed in: ask the CLI's own check.
    // `undefined` while it runs, `null` when it can't be answered (no guess, no guide).
    // Providers with no check command (Grok, OpenCode, Kilo, Cline) can't be told apart: the guide stays.
    const [cliSignedIn] = createResource(s.providerId, async (id): Promise<boolean | null> => {
        const p = getProvider(id);
        if (!p || p.authType !== "cli-managed" || p.authCheckCommand.length === 0) return false;
        try {
            const found = await checkProviderInstalled({
                providerId: p.id,
                cliCommand: p.cliCommand,
                npmPackage: p.npmPackage || undefined,
            });
            if (!found.installed || !found.path) return false;
            return (await checkCliAuth(found.path, p.authCheckCommand, undefined, 15000)).authenticated;
        } catch {
            return null;
        }
    });
    const showFreeGuide = () => {
        const list = accounts();
        if (!list || cliSignedIn() !== false) return false;
        const provider = s.providerId() ?? s.props.template.provider;
        return !list.some((acct) => acct.provider === provider);
    };

    return { showFreeGuide };
}

export type FreeGuide = ReturnType<typeof useFreeGuide>;
