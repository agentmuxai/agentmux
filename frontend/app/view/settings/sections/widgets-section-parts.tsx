// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The parts of Settings → Widgets (widgets-section.tsx lays them out): the toolbar, the replace prompt, the package
// list with the install prompt of each, and the publishers' keys.

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { widgetLoadErrors } from "@/app/block/widget-loader";
import { getApi } from "@/util/app-api";
import { RpcApi } from "@/app/store/rpc-api";
import type { WidgetPackageInfo, WidgetState } from "@/app/store/rpc-api/widgets";
import { TabRpcClient } from "@/app/store/rpc-util";
import { refreshWidgetPackages, widgetPackagesLoaded } from "@/app/store/widget-packages-store";
import { installLabel, WidgetApprovalDetails } from "./widget-approval-details";
import { CATALOG_LINE, inCatalog } from "./widget-catalog";
import { describePermission } from "./widget-permissions";
import { errorText, type WidgetsSectionState } from "./widgets-section-state";

const stateText = (state: WidgetState): string => t(`settings.widgets.state.${state === "needs_approval" ? "needsApproval" : state}`);

/** The install prompt for one package (spec §8.3). */
function ApprovalPrompt(props: { pkg: WidgetPackageInfo; onDone: () => void }): JSX.Element {
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const decide = async (approve: boolean) => {
        setBusy(true);
        try {
            // With the key the prompt showed: srv refuses if another signs it now.
            await getApi().approvals.decideWidget(props.pkg.id, props.pkg.hash, approve, props.pkg.signature?.fingerprint ?? "");
            props.onDone();
        } catch (e) {
            setError(errorText(e));
        } finally {
            setBusy(false);
        }
    };
    const updated = () => props.pkg.state === "changed";
    return (
        <div class="widget-approval" role="dialog" aria-label={t("settings.widgets.installAria", { name: props.pkg.name })}>
            <div class="widget-approval-title">
                <i class={`fa-solid fa-${props.pkg.icon}`} /> {updated() ? t("settings.widgets.changedTitle", { name: props.pkg.name }) : t("settings.widgets.installTitle", { name: props.pkg.name })}
            </div>
            <WidgetApprovalDetails pkg={props.pkg} />
            <Show when={inCatalog(props.pkg)}>
                <div class="widget-approval-signature">
                    <i class="fa-solid fa-store" aria-hidden="true" /> {CATALOG_LINE}
                </div>
            </Show>
            <Show when={error()}>
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            </Show>
            <div class="widget-approval-actions">
                <Button
                    tone={props.pkg.signature?.state === "key_changed" ? "danger" : "accent"}
                    disabled={busy()}
                    onClick={() => decide(true)}
                >
                    {installLabel(props.pkg, updated() ? t("settings.widgets.approve") : t("settings.widgets.install"))}
                </Button>
                <Button disabled={busy()} onClick={() => decide(false).then(props.onDone)}>
                    {t("settings.widgets.cancel")}
                </Button>
            </div>
        </div>
    );
}
/** The list row's one line on who signed it. */
function signatureShort(pkg: WidgetPackageInfo): string {
    const s = pkg.signature;
    switch (s?.state) {
        case "signed":
        case "signed_new":
            return t("settings.widgets.signedBy", { publisher: s.publisher, fingerprint: s.fingerprint });
        case "key_changed":
            return s.fingerprint
                ? t("settings.widgets.signedOtherKey", { publisher: s.publisher, fingerprint: s.fingerprint })
                : t("settings.widgets.unsignedUnlike", { publisher: s.publisher });
        default:
            return t("settings.widgets.notSigned");
    }
}

export function WidgetsToolbar(part: { s: WidgetsSectionState }): JSX.Element {
    const { busy, install, run, reveal, widgetsDir } = part.s;
    return (
        <div class="widgets-toolbar">
            <Button icon="plus" disabled={busy() === "install"} onClick={() => install()}>
                {t("settings.widgets.installEllipsis")}
            </Button>
            <Button
                icon="rotate"
                disabled={busy() === "rescan"}
                onClick={() => run("rescan", async () => (await RpcApi.WidgetsRescanCommand(TabRpcClient)).packages)}
            >
                {t("settings.widgets.rescan")}
            </Button>
            <Show when={widgetsDir()}>
                {(dir) => (
                    <Button icon="folder-open" onClick={() => void reveal(dir())}>
                        {t("settings.widgets.openFolder")}
                    </Button>
                )}
            </Show>
        </div>
    );
}

/** Asks whether to replace a widget that is already installed. */
export function ReplacePrompt(part: { s: WidgetsSectionState }): JSX.Element {
    const { replacePath, setReplacePath, installFrom } = part.s;
    return (
        <Show when={replacePath()}>
            {(path) => (
                <div class="widget-approval">
                    {t("settings.widgets.replaceQuestion")}
                    <div class="widget-approval-actions">
                        <Button tone="accent" onClick={() => installFrom(path(), true)}>
                            {t("settings.widgets.replace")}
                        </Button>
                        <Button onClick={() => setReplacePath(null)}>{t("settings.widgets.cancel")}</Button>
                    </div>
                </div>
            )}
        </Show>
    );
}

function RowActions(part: { pkg: WidgetPackageInfo; s: WidgetsSectionState }): JSX.Element {
    const { pkg } = part;
    const { busy, run, reveal, setAsking, confirmRemove, setConfirmRemove } = part.s;
    return (
        <div class="widget-row-actions">
            <Show when={pkg.state === "needs_approval" || pkg.state === "changed"}>
                <Button tone="accent" onClick={() => setAsking(pkg.id)}>
                    {pkg.state === "changed" ? t("settings.widgets.reviewApprove") : t("settings.widgets.approve")}
                </Button>
            </Show>
            <Show when={pkg.state === "approved"}>
                <Button
                    disabled={busy() === pkg.id}
                    onClick={() =>
                        run(pkg.id, async () =>
                            (await RpcApi.WidgetsSetEnabledCommand(TabRpcClient, { id: pkg.id, enabled: false })).packages
                        )
                    }
                >
                    {t("settings.widgets.turnOff")}
                </Button>
            </Show>
            <Show when={pkg.state === "disabled"}>
                <Button
                    disabled={busy() === pkg.id}
                    onClick={() =>
                        run(pkg.id, async () =>
                            (await RpcApi.WidgetsSetEnabledCommand(TabRpcClient, { id: pkg.id, enabled: true })).packages
                        )
                    }
                >
                    {t("settings.widgets.turnOn")}
                </Button>
            </Show>
            <Button icon="folder-open" onClick={() => void reveal(pkg.folder)}>
                {t("settings.widgets.showFolder")}
            </Button>
            <Show when={!pkg.implied}>
                <Show
                    when={confirmRemove() === pkg.id}
                    fallback={
                        <Button tone="danger" onClick={() => setConfirmRemove(pkg.id)}>
                            {t("settings.widgets.uninstall")}
                        </Button>
                    }
                >
                    <span class="widget-row-confirm">{t("settings.widgets.deleteQuestion", { name: pkg.name })}</span>
                    <Button
                        tone="danger"
                        disabled={busy() === pkg.id}
                        onClick={() =>
                            run(pkg.id, async () => {
                                setConfirmRemove(null);
                                return (await RpcApi.WidgetsUninstallCommand(TabRpcClient, { id: pkg.id })).packages;
                            })
                        }
                    >
                        {t("settings.widgets.delete")}
                    </Button>
                    <Button onClick={() => setConfirmRemove(null)}>{t("settings.widgets.keep")}</Button>
                </Show>
            </Show>
        </div>
    );
}

function WidgetRow(part: { pkg: WidgetPackageInfo; s: WidgetsSectionState }): JSX.Element {
    const { pkg } = part;
    const { asking, setAsking } = part.s;
    return (
        <div class="widget-row" data-widget-id={pkg.id}>
            <div class="widget-row-head">
                <i class={`fa-solid fa-${pkg.icon}`} />
                <span class="widget-row-name">{pkg.name}</span>
                <span class="widget-row-version">{pkg.version}</span>
                <span class="widget-row-kind">{pkg.kind}</span>
                <span class={`widget-row-state state-${pkg.state}`}>{stateText(pkg.state)}</span>
            </div>
            <Show when={pkg.description}>
                <div class="widget-row-description">{pkg.description}</div>
            </Show>
            <Show when={pkg.error ?? widgetLoadErrors()[pkg.id]}>
                {(e) => <div class="settings-config-error">{e()}</div>}
            </Show>
            <div class={`widget-row-signature${pkg.signature?.state === "key_changed" ? " warning" : ""}`}>
                {signatureShort(pkg)}
            </div>
            <Show when={pkg.kind === "sandboxed" && pkg.permissions.length > 0}>
                <div class="widget-row-permissions">
                    {pkg.permissions.map((p) => describePermission(p).text).join(" · ")}
                </div>
            </Show>
            <Show when={asking() === pkg.id && (pkg.state === "needs_approval" || pkg.state === "changed")}>
                <ApprovalPrompt pkg={pkg} onDone={() => setAsking(null)} />
            </Show>
            <RowActions pkg={pkg} s={part.s} />
        </div>
    );
}

export function WidgetList(part: { s: WidgetsSectionState }): JSX.Element {
    const { list } = part.s;
    return (
        <Show
            when={list().length > 0}
            fallback={
                <div class="setting-devices-empty">
                    {widgetPackagesLoaded() ? t("settings.widgets.none") : t("settings.widgets.loading")}
                </div>
            }
        >
            <div class="widgets-list">
                <For each={list()}>{(pkg) => <WidgetRow pkg={pkg} s={part.s} />}</For>
            </div>
        </Show>
    );
}

export function Publishers(part: { s: WidgetsSectionState }): JSX.Element {
    const { publishers, busy, run } = part.s;
    return (
        <Show when={publishers().length > 0}>
            <div class="widgets-publishers">
                <div class="setting-devices-description">
                    {t("settings.widgets.publishersNote")}
                </div>
                <For each={publishers()}>
                    {(pin) => (
                        <div class="widget-publisher-row" data-publisher={pin.publisher}>
                            <span class="widget-row-name">{pin.publisher}</span>
                            <code>{pin.fingerprint}</code>
                            <Show when={typeof getApi().approvals?.forgetWidgetKey === "function"}>
                                <Button
                                    disabled={busy() === `key:${pin.publisher}`}
                                    onClick={() =>
                                        run(`key:${pin.publisher}`, async () => {
                                            await getApi().approvals.forgetWidgetKey!(pin.publisher);
                                            await refreshWidgetPackages();
                                        })
                                    }
                                >
                                    {t("settings.widgets.forgetKey")}
                                </Button>
                            </Show>
                        </div>
                    )}
                </For>
            </div>
        </Show>
    );
}
