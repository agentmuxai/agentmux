// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * UsePathToggle: "Use the CLI on my PATH" for one agent CLI. AgentMux runs its
 * own installed, pinned copy of every CLI; this lists the provider in
 * `cli:usepath` so a launch runs the person's own copy instead, which AgentMux
 * only reads and never changes
 * (docs/specs/SPEC_ISOLATED_INSTALL_FOR_NON_NPM_CLIS_2026_11_05.md §3).
 */

import type { JSX } from "solid-js";

import { Switch } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { settingsAtom } from "@/app/store/global";

const KEY = "cli:usepath";

/** `providerId` is in the setting now. */
function usesPathCli(providerId: string): boolean {
    const ids = (settingsAtom() as Record<string, unknown> | undefined)?.[KEY];
    return Array.isArray(ids) && ids.includes(providerId);
}

interface UsePathToggleProps {
    providerId: string;
    /** The setting was saved: the row looks for the CLI again. */
    onChanged: () => void;
}

export const UsePathToggle = (props: UsePathToggleProps): JSX.Element => {
    const change = async (on: boolean) => {
        const current = (settingsAtom() as Record<string, unknown> | undefined)?.[KEY];
        const ids = (Array.isArray(current) ? current : []).filter((id): id is string => typeof id === "string" && id !== props.providerId);
        if (on) ids.push(props.providerId);
        await RpcApi.SetConfigCommand(TabRpcClient, { [KEY]: ids.length ? ids : null } as any);
        props.onChanged();
    };
    return (
        <div class="toolchain-detail-row">
            <Switch
                checked={usesPathCli(props.providerId)}
                onChange={(on) => void change(on)}
                ariaLabel="Use the CLI on my PATH"
            />
            <span class="toolchain-detail-label" title="Off: AgentMux runs its own copy. On: it runs yours, which it never changes.">
                Use the CLI on my PATH
            </span>
        </div>
    );
};
