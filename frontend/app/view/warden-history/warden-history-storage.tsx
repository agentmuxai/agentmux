// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden — History section, Storage: where agent history is kept on this
// machine and how much (`history.usage`, the status bar total's numbers), and
// what is kept for how long.

import { createEffect, createSignal, For, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { measuredAgo } from "@/app/statusbar/HistoryUsagePopover";
import type { HistoryUsage } from "@/types/rpc/HistoryUsage";
import { formatBytes } from "@/util/format-bytes";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-history.scss";

export const WardenHistoryStorage = (props: { visible: boolean }): JSX.Element => {
    const [usage, setUsage] = createSignal<HistoryUsage | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [loading, setLoading] = createSignal(false);

    const load = async (refresh: boolean) => {
        setLoading(true);
        try {
            setUsage(await RpcApi.HistoryUsageCommand(TabRpcClient, { refresh }));
            setError(null);
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    createEffect(() => {
        if (props.visible && usage() === null && !loading()) void load(false);
    });

    const share = (bytes: number): number => {
        const total = usage()?.total_bytes ?? 0;
        return total > 0 ? Math.min(100, (bytes / total) * 100) : 0;
    };

    return (
        <div class="warden-history-storage">
            <Show when={error()}>
                <div class="warden-manager-error">⚠ {error()}</div>
            </Show>
            <Show when={usage()} fallback={<div class="warden-section-stub">{loading() ? "Measuring…" : ""}</div>}>
                {(u) => (
                    <>
                        <div class="warden-history-total">
                            <span class="warden-history-total-bytes">{formatBytes(u().total_bytes)}</span>
                            <span class="warden-manager-dim">of agent history on this machine</span>
                        </div>
                        <table class="warden-manager-table warden-history-stores">
                            <thead>
                                <tr>
                                    <th>Kept in</th>
                                    <th class="warden-history-share-col" />
                                    <th class="warden-history-num">Size</th>
                                    <th class="warden-history-num">Files</th>
                                </tr>
                            </thead>
                            <tbody>
                                <For each={u().stores}>
                                    {(s) => (
                                        <tr data-store={s.key}>
                                            <td>{s.label}</td>
                                            <td class="warden-history-share-col">
                                                <span class="warden-history-share" aria-hidden="true">
                                                    <span class="warden-history-share-fill" style={{ width: `${share(s.bytes)}%` }} />
                                                </span>
                                            </td>
                                            <td class="warden-history-num">{formatBytes(s.bytes)}</td>
                                            <td class="warden-history-num">{s.files.toLocaleString()}</td>
                                        </tr>
                                    )}
                                </For>
                            </tbody>
                        </table>
                        <div class="warden-history-bar">
                            <span class="warden-manager-dim">Measured {measuredAgo(u().computed_at_ms, Date.now())}</span>
                            <Button tone="quiet" density="compact" busy={loading()} onClick={() => void load(true)}>
                                {loading() ? "Measuring…" : "Measure again"}
                            </Button>
                        </div>
                    </>
                )}
            </Show>
            <section class="warden-history-retention" aria-label="Retention">
                <h3>Retention</h3>
                <p>Agent history is kept forever. AgentMux deletes none of it on its own:</p>
                <ul>
                    <li>Claude Code keeps its session files: AgentMux sets its cleanup to 36,500 days, unless its config already sets another value.</li>
                    <li>Conversations archived by "New conversation" and inactive panes' archived transcripts are never pruned.</li>
                    <li>The record of messages between agents is written to disk and kept.</li>
                </ul>
            </section>
        </div>
    );
};

WardenHistoryStorage.displayName = "WardenHistoryStorage";
