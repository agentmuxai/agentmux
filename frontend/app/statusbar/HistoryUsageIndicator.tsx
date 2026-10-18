// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * HistoryUsageIndicator: the running total of agent history on this machine,
 * in the status bar just left of the token count. Clicking opens the
 * HistoryUsagePopover with where it is kept. History is kept forever, so the
 * total only grows; this is how the user sees that.
 */

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { HistoryUsage } from "@/types/rpc/HistoryUsage";
import { formatBytes } from "@/util/format-bytes";
import { HistoryUsagePopover } from "./HistoryUsagePopover";

/** How often the total is asked for again. The server measures at most every
 *  five minutes, so this costs a lookup. */
const REFRESH_MS = 5 * 60 * 1000;

export const HistoryUsageIndicator = (): JSX.Element => {
    const [usage, setUsage] = createSignal<HistoryUsage | null>(null);
    const [open, setOpen] = createSignal(false);
    let ref: HTMLButtonElement | undefined;

    const load = async (refresh = false) => {
        try {
            setUsage(await RpcApi.HistoryUsageCommand(TabRpcClient, { refresh }));
        } catch (e) {
            // Keep the last total; the panel says when it was measured.
            console.warn("history usage:", e);
        }
    };

    onMount(() => {
        void load();
        const timer = setInterval(() => void load(), REFRESH_MS);
        onCleanup(() => clearInterval(timer));
    });

    return (
        <>
            <button
                type="button"
                ref={ref}
                class="history-usage-indicator"
                onClick={() => setOpen(!open())}
                aria-label="History kept on this machine, click for details"
                aria-haspopup="dialog"
                aria-expanded={open()}
                data-tip="Agent history kept on this machine"
            >
                <i class="fa-regular fa-clock-rotate-left" aria-hidden="true" />{" "}
                {usage() ? formatBytes(usage()!.total_bytes) : "…"}
            </button>
            <Show when={open()}>
                <HistoryUsagePopover
                    anchor={ref}
                    usage={usage()}
                    onRefresh={() => load(true)}
                    onClose={() => setOpen(false)}
                />
            </Show>
        </>
    );
};

HistoryUsageIndicator.displayName = "HistoryUsageIndicator";
