// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which agents may fire this drone at once (real-world jobs spec §6.5): the agent that made it, and the agents the
// person gave it to. An agent that is neither asks the person first, with a countdown. The list is srv's; this edits it.

import { createEffect, createMemo, createSignal, For, on, onCleanup, Show, type JSX } from "solid-js";

import { Button, Select, type SelectOption } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { useAgentList } from "@/app/view/agent/use-agent-list";
import type { DroneAccess } from "@/types/rpc/DroneAccess";

/** The drone's owner and the agents it was given to, read again when drones change. None until it is read. */
function useDroneAccess(droneId: () => string): [() => DroneAccess | null, (agents: string[]) => Promise<void>] {
    const [access, setAccess] = createSignal<DroneAccess | null>(null);
    // An answer counts only while the pane still shows the drone it was asked for.
    const load = async () => {
        const id = droneId();
        if (!id) return setAccess(null);
        try {
            const got = await RpcApi.GetDroneAccessCommand(TabRpcClient, { drone_id: id });
            if (id === droneId()) setAccess(got);
        } catch {
            if (id === droneId()) setAccess(null);
        }
    };
    // Another drone opened in the same row: forget the last one's list before reading this one's.
    createEffect(
        on(droneId, () => {
            setAccess(null);
            void load();
        })
    );
    onCleanup(onMuxEvent(WpsEvent.DronesChanged, () => void load()));
    const save = async (agents: string[]) => {
        const id = droneId();
        const saved = await RpcApi.SetDroneAgentsCommand(TabRpcClient, { drone_id: id, agents });
        if (id === droneId()) setAccess(saved);
    };
    return [access, save];
}

/** A row in the Trigger card: who may fire the drone without asking, and a way to give it to another agent. */
export const DroneFireAgents = (p: { droneId: () => string }): JSX.Element => {
    const [access, save] = useDroneAccess(p.droneId);
    const [agents] = useAgentList();
    const given = () => access()?.agents ?? [];
    const options = createMemo<SelectOption[]>(() =>
        (agents() ?? [])
            .filter((a) => !a.is_seeded && a.id !== access()?.owner && !given().includes(a.id))
            .map((a) => ({ value: a.id, label: a.name }))
    );
    return (
        <Show when={access()}>
            <div class="drone-trigger-agents">
                <span class="drone-trigger-name">{t("drone.trigger.agents")}</span>
                <Show when={access()!.owner}>
                    <span class="drone-trigger-kind">{t("drone.trigger.agentsOwner", { agent: access()!.owner })}</span>
                </Show>
                <For each={given()}>
                    {(id) => (
                        <Button class="drone-btn" onClick={() => save(given().filter((a) => a !== id))}>
                            {t("drone.trigger.agentsRemove", { agent: id })}
                        </Button>
                    )}
                </For>
                <Select
                    value=""
                    onChange={(v) => v && void save([...given(), v])}
                    options={[{ value: "", label: t("drone.trigger.agentsAdd") }, ...options()]}
                />
            </div>
            <Show when={!access()!.owner && given().length === 0}>
                <p class="drone-trigger-note">{t("drone.trigger.agentsNone")}</p>
            </Show>
        </Show>
    );
};
