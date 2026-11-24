// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The cards for the approvals a run of this drone waits on (real-world jobs spec §6.4): what is proposed, the items it
// was made from with the ones that came from outside text marked, and Approve and Reject. The answer is the person's, so
// it goes out from this window; srv refuses it from anything else.

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { pollWhileVisible } from "@/app/view/warden-shared/warden-shared";
import type { DroneApproval } from "@/types/rpc/DroneApproval";

export const DroneApprovals = (p: { droneId: () => string }): JSX.Element => {
    const [open, setOpen] = createSignal<DroneApproval[]>([]);
    const [error, setError] = createSignal("");
    const refresh = async () => {
        const id = p.droneId();
        if (!id) return setOpen([]);
        try {
            setOpen(await RpcApi.ListDroneApprovalsCommand(TabRpcClient, { drone_id: id }));
        } catch {
            setOpen([]);
        }
    };
    pollWhileVisible(() => void refresh());
    const answer = async (a: DroneApproval, approve: boolean) => {
        setError("");
        try {
            await RpcApi.AnswerDroneApprovalCommand(TabRpcClient, { run_id: a.run_id, block_id: a.block_id, approve });
        } catch (e) {
            setError(String(e));
        }
        await refresh();
    };
    return (
        <For each={open()}>
            {(a) => (
                <section class="drone-approval" role="alert" aria-label={t("drone.approval.title")}>
                    <h3 class="drone-approval-title">{a.title}</h3>
                    <For each={a.items}>
                        {(item) => (
                            <div class="drone-approval-item">
                                <span class="drone-approval-label">{item.label}</span>
                                <Show when={item.tainted}>
                                    <span class="drone-approval-outside">{t("drone.approval.outside")}</span>
                                </Show>
                                <pre class="drone-approval-text">{item.text}</pre>
                            </div>
                        )}
                    </For>
                    <div class="drone-approval-actions">
                        <Button class="drone-btn" onClick={() => void answer(a, true)}>
                            {t("drone.approval.approve")}
                        </Button>
                        <Button class="drone-btn" onClick={() => void answer(a, false)}>
                            {t("drone.approval.reject")}
                        </Button>
                        <span class="drone-approval-until">{t("drone.approval.until", { time: new Date(a.deadline_ms).toLocaleString() })}</span>
                        <Show when={error()}>
                            <span class="drone-approval-error">{error()}</span>
                        </Show>
                    </div>
                </section>
            )}
        </For>
    );
};
