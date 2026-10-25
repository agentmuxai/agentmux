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
import type { HistoryCompression } from "@/types/rpc/HistoryCompression";
import type { HistoryReclaim } from "@/types/rpc/HistoryReclaim";
import type { HistoryUsage } from "@/types/rpc/HistoryUsage";
import { formatBytes } from "@/util/format-bytes";

import "@/app/view/warden-shared/warden-manager-chrome.scss";
import "./warden-history.scss";

/** How compressed the shared pane history is (`agent:transcriptframes`). */
const CompressionLine = (props: { c: HistoryCompression }): JSX.Element => {
    const ratio = (): string => (props.c.stored_bytes > 0 ? (props.c.logical_bytes / props.c.stored_bytes).toFixed(1) : "0");
    return (
        <div class="warden-history-compression" data-testid="history-compression">
            <Show
                when={props.c.frames > 0}
                fallback={
                    <span class="warden-manager-dim">
                        {props.c.enabled ? "Pane history compression is on; nothing is compressed yet." : "Pane history compression is turned off (agent:transcriptframes is false)."}
                    </span>
                }
            >
                <span>
                    Pane history compressed: {formatBytes(props.c.logical_bytes)} of transcript in {formatBytes(props.c.stored_bytes)} ({ratio()}×),{" "}
                    {props.c.frames.toLocaleString()} frames.
                </span>
            </Show>
            <Show when={props.c.enabled && props.c.waiting}>
                <span class="warden-manager-dim"> Compression waits: {props.c.waiting}</span>
            </Show>
        </div>
    );
};

/** An srv from before reclaiming answers without these. */
const NO_RECLAIM: HistoryReclaim = { pending: false, last_done_ms: 0, last_before_bytes: 0, last_after_bytes: 0 };
const reclaimOf = (c: HistoryCompression): HistoryReclaim => c.reclaim ?? NO_RECLAIM;

/** Free space worth offering back: 1 GB and a fifth of the file (spec §6). */
export const reclaimOffered = (c: HistoryCompression): boolean =>
    !c.incremental && !reclaimOf(c).pending && c.steady && (c.free_bytes ?? 0) >= 1e9 && c.free_bytes >= (c.file_bytes ?? 0) * 0.2;

/**
 * Giving the shared store's free space back (`history.reclaim`): offered once,
 * it runs at the next start of AgentMux (SPEC_HISTORY_STORE_RECLAIM_SPACE_2026_10_23).
 */
const ReclaimLine = (props: { c: HistoryCompression; onChange: (r: HistoryReclaim) => void }): JSX.Element => {
    const [confirm, setConfirm] = createSignal(false);
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const r = () => reclaimOf(props.c);
    const act = async (action: "request" | "cancel") => {
        setBusy(true);
        try {
            props.onChange(await RpcApi.HistoryReclaimCommand(TabRpcClient, { action }));
            setError(null);
            setConfirm(false);
        } catch (e) {
            setError(String(e));
        } finally {
            setBusy(false);
        }
    };
    return (
        <div class="warden-history-compression" data-testid="history-reclaim">
            <Show when={r().pending}>
                <span>
                    {formatBytes(props.c.free_bytes)} of free space is given back the next time AgentMux starts (after an update, or quit
                    and reopen it).
                </span>
                <Show when={r().waiting_for}>
                    <span class="warden-manager-dim"> Last start: {r().waiting_for}.</span>
                </Show>{" "}
                <Button tone="quiet" density="compact" busy={busy()} onClick={() => void act("cancel")}>
                    Don't reclaim
                </Button>
            </Show>
            <Show when={reclaimOffered(props.c)}>
                <span>{formatBytes(props.c.free_bytes)} of the shared pane history's file is free space AgentMux can give back.</span>{" "}
                <Show
                    when={confirm()}
                    fallback={
                        <Button tone="quiet" density="compact" onClick={() => setConfirm(true)}>
                            {`Reclaim ${formatBytes(props.c.free_bytes)}…`}
                        </Button>
                    }
                >
                    <span class="warden-manager-dim">
                        It runs at the next start, before agents load, and takes about a minute; nothing is deleted.{" "}
                    </span>
                    <Button tone="accent" density="compact" busy={busy()} onClick={() => void act("request")}>
                        Reclaim at next start
                    </Button>
                    <Button tone="quiet" density="compact" onClick={() => setConfirm(false)}>
                        Cancel
                    </Button>
                </Show>
            </Show>
            <Show when={!r().pending && r().last_done_ms > 0}>
                <span class="warden-manager-dim">
                    Gave back {formatBytes(Math.max(0, r().last_before_bytes - r().last_after_bytes))} on{" "}
                    {new Date(r().last_done_ms).toLocaleDateString()}
                    {props.c.incremental ? "; freed space now goes back by itself." : "."}
                </span>
            </Show>
            <Show when={r().last_error}>
                <span class="warden-manager-error"> Reclaiming failed: {r().last_error}</span>
            </Show>
            <Show when={error()}>
                <span class="warden-manager-error"> ⚠ {error()}</span>
            </Show>
        </div>
    );
};

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
                        <Show when={u().compression}>
                            {(c) => (
                                <>
                                    <CompressionLine c={c()} />
                                    <ReclaimLine c={c()} onChange={(reclaim) => setUsage({ ...u(), compression: { ...c(), reclaim } })} />
                                </>
                            )}
                        </Show>
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
