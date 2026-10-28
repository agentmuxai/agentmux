// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane's "this will happen in 30 s: Veto" banner for a sensitive operation the super agent
 * approved (docs/specs/SPEC_SUPER_AGENT_2026_10_25.md §5). No click is needed to let it
 * proceed; Veto stops it and Approve now skips the wait. The words are srv's, shown as text.
 */

import { createSignal, For, onCleanup, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { PaneRow } from "../components/PaneRow";
import { countdownText, dropCountdown, secondsLeft, useSensitiveCountdowns } from "./sensitive-pending";

export const SensitiveBanner = (): JSX.Element => {
    const { open, release } = useSensitiveCountdowns();
    onCleanup(release);
    const [now, setNow] = createSignal(Date.now());
    const [deciding, setDeciding] = createSignal<string | null>(null);
    const timer = setInterval(() => setNow(Date.now()), 250);
    onCleanup(() => clearInterval(timer));

    const decide = (requestId: string, decision: "veto" | "approve_now") => {
        if (deciding()) return;
        setDeciding(requestId);
        RpcApi.SensitiveDecideCommand(TabRpcClient, { request_id: requestId, decision })
            .then((res) => {
                // `too_late`: it has begun or ended; the clear event follows.
                if (res?.outcome === "vetoed" || res?.outcome === "too_late") dropCountdown(requestId);
            })
            .catch((e) => console.error("[sensitive] decision failed:", e))
            .finally(() => setDeciding(null));
    };

    return (
        <For each={open()}>
            {(c) => (
                <PaneRow
                    sigil="⏻"
                    accent="error"
                    title={countdownText(c, secondsLeft(c, now()))}
                    actions={[
                        {
                            label: "Veto",
                            title: "Stop this from happening",
                            primary: true,
                            disabled: deciding() !== null,
                            onClick: () => decide(c.request_id, "veto"),
                        },
                        {
                            label: "Approve now",
                            title: "Let it happen now, without waiting",
                            disabled: deciding() !== null,
                            onClick: () => decide(c.request_id, "approve_now"),
                        },
                    ]}
                />
            )}
        </For>
    );
};

SensitiveBanner.displayName = "SensitiveBanner";
