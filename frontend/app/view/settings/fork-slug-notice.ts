// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The one-time notice pointing to Settings › Agents when some agent has an
// id an older build chained (sections/fork-slug-migration.tsx). Asked once
// at startup, and never again once dismissed: closing it, "Open Settings" and
// "Don't show again" all turn `agent:outdatedidsnotice` off. Nothing runs
// from here; the card is where the update is started.

import { setNotificationAction } from "@/app/notification/usenotification";
import { pushNotification, settingsAtom } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

export const NOTICE_ID = "fork-slug-update";

/** Off once the notice was dismissed; unset means it may be shown. */
export const NOTICE_SETTING = "agent:outdatedidsnotice";

let actionSeq = 0;

function action(fn: () => void): string {
    const key = `forkSlugNotice:${++actionSeq}`;
    setNotificationAction(key, fn);
    return key;
}

function dismiss(): void {
    void RpcApi.SetConfigCommand(TabRpcClient, { [NOTICE_SETTING]: false } as SettingsType);
}

/** Settings, at its Agents section. */
export function openAgentsSettings(): void {
    void RpcApi.PaneOpenCommand(TabRpcClient, { view: "settings", meta: { view: "settings", "settings:section": "agents" } }, {});
}

/** At startup: tell the user once when agents have an outdated id. */
export async function offerForkSlugUpdate(now: number = Date.now()): Promise<void> {
    if (settingsAtom()?.[NOTICE_SETTING] === false) return;
    let count = 0;
    try {
        const result = await RpcApi.AgentsMigrateForkSlugsCommand(TabRpcClient, { mode: "plan", quick: true });
        count = result.plan?.candidates.length ?? 0;
    } catch (err) {
        console.warn("[fork-slug-notice] couldn't look for outdated agent ids", err);
        return;
    }
    if (count === 0) return;
    pushNotification({
        id: NOTICE_ID,
        persistent: true,
        type: "info",
        icon: "id-badge",
        title: `${count} agent${count === 1 ? " has" : "s have"} an outdated id`,
        message: "Forks made by older builds have ids that repeat every generation. Settings › Agents shows what would change, and updates them when you ask.",
        timestamp: new Date(now).toISOString(),
        actions: [
            {
                label: "Open Settings",
                actionKey: action(() => {
                    dismiss();
                    openAgentsSettings();
                }),
                color: "green",
            },
            { label: "Don't show again", actionKey: action(dismiss), color: "grey" },
        ],
        onClose: dismiss,
    });
}
