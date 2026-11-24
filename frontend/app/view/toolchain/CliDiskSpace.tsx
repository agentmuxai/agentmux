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
import { Button, Notice, SectionHeader } from "@/app/element/ui";
import { formatBytes } from "@/util/format-bytes";
import { tip } from "@/app/element/tip";
import { t } from "@/app/i18n/i18n";

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
                    <li {...tip(c.dir)}>
                        {c.provider} {c.version}
                        {c.legacy ? t("toolchain.prune.oldLayout") : ""} · {formatBytes(c.bytes)} · {t("toolchain.prune.unused", { count: c.idle_days })}
                    </li>
                )}
            </For>
        </ul>
    );

    return (
        <section class="toolchain-section" data-testid="cli-disk-space">
            <SectionHeader variant="group" level={4} title={t("toolchain.prune.title")} />
            <Show when={state().kind === "idle" || state().kind === "error"}>
                <div class="toolchain-env-line">
                    {t("toolchain.prune.about")}
                </div>
                <Button tone="quiet" onClick={check}>
                    {t("toolchain.prune.check")}
                </Button>
            </Show>
            <Show when={state().kind === "error"}>
                <Notice tone="danger">{t("toolchain.prune.checkFailed", { message: (state() as { message: string }).message })}</Notice>
            </Show>
            <Show when={state().kind === "working"}>
                <div class="toolchain-env-line">
                    {(state() as { what: string }).what === "check" ? t("toolchain.prune.checking") : t("toolchain.prune.removing")}
                </div>
            </Show>
            <Show when={state().kind === "checked"}>
                {(() => {
                    const r = () => (state() as { result: ToolchainPruneResult }).result;
                    return (
                        <>
                            <Show when={!r().scan_ok}>
                                <div class="toolchain-env-line">
                                    {t("toolchain.prune.scanFailedCheck")}
                                </div>
                            </Show>
                            <Show when={r().scan_ok && r().candidates.length === 0}>
                                <div class="toolchain-env-line">{t("toolchain.prune.nothing")}</div>
                            </Show>
                            <Show when={r().candidates.length > 0}>
                                <div class="toolchain-env-line">
                                    {t("toolchain.prune.found", { count: r().candidates.length, size: formatBytes(r().reclaimable_bytes) })}
                                </div>
                                {list(r().candidates)}
                                <Button tone="danger" icon="trash" onClick={() => remove(r().candidates.map((c) => c.dir))}>
                                    {t("toolchain.prune.remove", { n: r().candidates.length, size: formatBytes(r().reclaimable_bytes) })}
                                </Button>
                            </Show>
                            <Button tone="quiet" onClick={check}>
                                {t("toolchain.prune.checkAgain")}
                            </Button>
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
                                    {t("toolchain.prune.scanFailedRemove")}
                                </div>
                            </Show>
                            <Show when={r().scan_ok && r().removed.length > 0}>
                                <div class="toolchain-env-line">
                                    {t("toolchain.prune.removed", { n: r().removed.length, size: formatBytes(freed()) })}
                                </div>
                            </Show>
                            {/* Nothing removed with a working scan: every listed install was
                                no longer removable by then (now in use, or used again). */}
                            <Show when={r().scan_ok && r().removed.length === 0}>
                                <div class="toolchain-env-line">
                                    {t("toolchain.prune.noneRemoved")}
                                </div>
                            </Show>
                            {list(r().removed)}
                            <Show when={r().skipped.length > 0}>
                                <div class="toolchain-env-line">
                                    {t("toolchain.prune.skipped", { count: r().skipped.length })}
                                </div>
                            </Show>
                            <Button tone="quiet" onClick={check}>
                                {t("toolchain.prune.checkAgain")}
                            </Button>
                        </>
                    );
                })()}
            </Show>
        </section>
    );
};
