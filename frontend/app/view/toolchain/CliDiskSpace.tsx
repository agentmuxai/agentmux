// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Old CLI versions": installed provider CLIs that nothing has used for 30 days
 * and nothing is running, with what removing them would free
 * (docs/specs/SPEC_RUNTIME_MENU_REMAINING_GAPS_2026_10_01.md §5).
 *
 * Opt-in on purpose: AgentMux never deletes these by itself. "Check" is a dry
 * run; "Remove" is a separate, explicit click on exactly what the check listed.
 */

import { createSignal, For, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import type { ToolchainPruneResult } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/** `1.2 GB`, `340 MB`, `12 KB`. */
export function formatBytes(n: number): string {
    if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
    if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
    if (n >= 1e3) return `${Math.round(n / 1e3)} KB`;
    return `${n} B`;
}

type State =
    | { kind: "idle" }
    | { kind: "working"; what: "check" | "remove" }
    | { kind: "checked"; result: ToolchainPruneResult }
    | { kind: "removed"; result: ToolchainPruneResult }
    | { kind: "error"; message: string };

export const CliDiskSpace = (): JSX.Element => {
    const [state, setState] = createSignal<State>({ kind: "idle" });

    const check = async () => {
        setState({ kind: "working", what: "check" });
        try {
            setState({ kind: "checked", result: await RpcApi.ToolchainPruneCommand(TabRpcClient, { dry_run: true }, { timeout: 60000 }) });
        } catch (err: any) {
            setState({ kind: "error", message: err?.message ?? String(err) });
        }
    };
    // Removes exactly what the last check listed (and only what is still
    // removable): an install that became removable since is not touched.
    const remove = async (dirs: string[]) => {
        setState({ kind: "working", what: "remove" });
        try {
            setState({
                kind: "removed",
                result: await RpcApi.ToolchainPruneCommand(TabRpcClient, { dry_run: false, only: dirs }, { timeout: 120000 }),
            });
        } catch (err: any) {
            setState({ kind: "error", message: err?.message ?? String(err) });
        }
    };

    const list = (items: ToolchainPruneResult["candidates"]) => (
        <ul class="toolchain-prune-list">
            <For each={items}>
                {(c) => (
                    <li title={c.dir}>
                        {c.provider} {c.version}
                        {c.legacy ? " (old layout)" : ""} · {formatBytes(c.bytes)} · unused for {c.idle_days} days
                    </li>
                )}
            </For>
        </ul>
    );

    return (
        <section class="toolchain-section" data-testid="cli-disk-space">
            <h3 class="toolchain-section-title">Old CLI versions</h3>
            <Show when={state().kind === "idle" || state().kind === "error"}>
                <div class="toolchain-env-line">
                    Agent CLIs are kept after an upgrade. This lists the ones nothing has used for 30 days and nothing is
                    running, and frees their space only when you ask.
                </div>
                <button class="toolchain-link-btn" onClick={check}>
                    Check for old versions
                </button>
            </Show>
            <Show when={state().kind === "error"}>
                <div class="toolchain-env-line toolchain-error" role="alert">
                    Couldn't check: {(state() as { message: string }).message}
                </div>
            </Show>
            <Show when={state().kind === "working"}>
                <div class="toolchain-env-line">
                    {(state() as { what: string }).what === "check" ? "Checking…" : "Removing…"}
                </div>
            </Show>
            <Show when={state().kind === "checked"}>
                {(() => {
                    const r = () => (state() as { result: ToolchainPruneResult }).result;
                    return (
                        <>
                            <Show when={!r().scan_ok}>
                                <div class="toolchain-env-line">
                                    Couldn't tell which CLIs are running, so nothing is offered for removal.
                                </div>
                            </Show>
                            <Show when={r().scan_ok && r().candidates.length === 0}>
                                <div class="toolchain-env-line">Nothing to remove — every installed CLI is current or recently used.</div>
                            </Show>
                            <Show when={r().candidates.length > 0}>
                                <div class="toolchain-env-line">
                                    {r().candidates.length} old {r().candidates.length === 1 ? "install" : "installs"} ·{" "}
                                    {formatBytes(r().reclaimable_bytes)} can be freed
                                </div>
                                {list(r().candidates)}
                                <button class="toolchain-link-btn" onClick={() => remove(r().candidates.map((c) => c.dir))}>
                                    Remove {r().candidates.length} · free {formatBytes(r().reclaimable_bytes)}
                                </button>
                            </Show>
                            <button class="toolchain-link-btn" onClick={check}>
                                Check again
                            </button>
                        </>
                    );
                })()}
            </Show>
            <Show when={state().kind === "removed"}>
                {(() => {
                    const r = () => (state() as { result: ToolchainPruneResult }).result;
                    const freed = () => r().removed.reduce((n, c) => n + c.bytes, 0);
                    return (
                        <>
                            <Show when={!r().scan_ok}>
                                <div class="toolchain-env-line">
                                    Couldn't tell which CLIs are running, so nothing was removed.
                                </div>
                            </Show>
                            <Show when={r().scan_ok && r().removed.length > 0}>
                                <div class="toolchain-env-line">
                                    Removed {r().removed.length} · freed {formatBytes(freed())}
                                </div>
                            </Show>
                            {/* Nothing removed with a working scan: every listed install was
                                no longer removable by then (now in use, or used again). */}
                            <Show when={r().scan_ok && r().removed.length === 0}>
                                <div class="toolchain-env-line">
                                    Nothing was removed — the listed installs are in use again or were used since the check.
                                </div>
                            </Show>
                            {list(r().removed)}
                            <Show when={r().skipped.length > 0}>
                                <div class="toolchain-env-line">
                                    Left in place ({r().skipped.length}): something started using {r().skipped.length === 1 ? "it" : "them"}{" "}
                                    or {r().skipped.length === 1 ? "it was" : "they were"} locked.
                                </div>
                            </Show>
                            <button class="toolchain-link-btn" onClick={check}>
                                Check again
                            </button>
                        </>
                    );
                })()}
            </Show>
        </section>
    );
};
