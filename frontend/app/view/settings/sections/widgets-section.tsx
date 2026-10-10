// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Widgets: every installed widget package, its state and
// permissions, and installing, approving, enabling and removing them
// (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8.4).
//
// Approving is the user's decision in this UI, carried to srv by the host
// (`approvals.decideWidget`): an agent, which has srv's auth key, can install a
// package but can't approve one.

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { widgetLoadErrors } from "@/app/block/widget-loader";
import { getApi } from "@/app/store/app-api";
import { RpcApi } from "@/app/store/rpc-api";
import type { WidgetPackageInfo, WidgetState } from "@/app/store/rpc-api/widgets";
import { TabRpcClient } from "@/app/store/rpc-util";
import { refreshWidgetPackages, setWidgetPackages, widgetPackages, widgetPackagesLoaded, widgetPublishers } from "@/app/store/widget-packages-store";
import { agentmuxHome } from "@/app/view/agent/agent-launch-env";
import type { SettingsIndexEntry } from "../settings-model";
import { SectionHeader } from "../settings-controls";
import { installLabel, WidgetApprovalDetails } from "./widget-approval-details";
import { describePermission } from "./widget-permissions";

export const WIDGETS_SETTINGS = {
    installed: {
        id: "widgets.installed",
        label: "Widgets",
        description:
            "Widgets add your own kinds of pane. A widget runs only after you approve it here, and asks again whenever its files change.",
        section: "widgets",
        keywords: ["widget", "plugin", "extension", "install", "approve", "ext:", "sandbox", "custom pane", "widget.json"],
    },
} satisfies Record<string, SettingsIndexEntry>;

const STATE_TEXT: Record<WidgetState, string> = {
    approved: "On",
    needs_approval: "Waiting for your approval",
    changed: "Changed since you approved it",
    invalid: "Can't be loaded",
    disabled: "Off",
};

function errorText(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
}

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
        <div class="widget-approval" role="dialog" aria-label={`Install ${props.pkg.name}`}>
            <div class="widget-approval-title">
                <i class={`fa-solid fa-${props.pkg.icon}`} /> {updated() ? `${props.pkg.name} changed` : `Install ${props.pkg.name}?`}
            </div>
            <WidgetApprovalDetails pkg={props.pkg} />
            <Show when={error()}>
                <div class="settings-config-error" role="alert">
                    {error()}
                </div>
            </Show>
            <div class="widget-approval-actions">
                <Button tone="accent" disabled={busy()} onClick={() => void decide(true)}>
                    {installLabel(props.pkg, updated() ? "Approve" : "Install")}
                </Button>
                <Button disabled={busy()} onClick={() => void decide(false).then(props.onDone)}>
                    Cancel
                </Button>
            </div>
        </div>
    );
}

export function WidgetsSection(): JSX.Element {
    const list = widgetPackages();
    const publishers = widgetPublishers();
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal<string | null>(null);
    const [asking, setAsking] = createSignal<string | null>(null);
    const [confirmRemove, setConfirmRemove] = createSignal<string | null>(null);
    const [replacePath, setReplacePath] = createSignal<string | null>(null);

    const run = async (key: string, f: () => Promise<WidgetPackageInfo[] | void>) => {
        setBusy(key);
        try {
            const next = await f();
            if (next) setWidgetPackages(next);
            setError(null);
        } catch (e) {
            setError(errorText(e));
        } finally {
            setBusy(null);
        }
    };

    const installFrom = async (path: string, replace: boolean) =>
        run("install", async () => {
            const r = await RpcApi.WidgetsInstallCommand(TabRpcClient, { path, replace });
            setReplacePath(null);
            setAsking(r.id);
            return r.packages;
        }).catch(() => {});

    const install = async () => {
        const path = await getApi().showOpenFileDialog();
        if (!path) return;
        setBusy("install");
        try {
            const r = await RpcApi.WidgetsInstallCommand(TabRpcClient, { path, replace: false });
            setWidgetPackages(r.packages);
            setAsking(r.id);
            setError(null);
        } catch (e) {
            const msg = errorText(e);
            if (/already installed/.test(msg)) setReplacePath(path);
            else setError(msg);
        } finally {
            setBusy(null);
        }
    };

    const widgetsDir = (): string | null => {
        try {
            return `${agentmuxHome()}/widgets`;
        } catch {
            return null;
        }
    };

    return (
        <div class="settings-section-body widgets-section">
            <SectionHeader label={WIDGETS_SETTINGS.installed.label} />
            <div id={`setting-${WIDGETS_SETTINGS.installed.id}`} class="setting-row">
                <div class="setting-devices-description">{WIDGETS_SETTINGS.installed.description}</div>
                <div class="widgets-toolbar">
                    <Button icon="plus" disabled={busy() === "install"} onClick={() => void install()}>
                        Install…
                    </Button>
                    <Button
                        icon="rotate"
                        disabled={busy() === "rescan"}
                        onClick={() => void run("rescan", async () => (await RpcApi.WidgetsRescanCommand(TabRpcClient)).packages)}
                    >
                        Rescan
                    </Button>
                    <Show when={widgetsDir()}>
                        {(dir) => (
                            <Button icon="folder-open" onClick={() => getApi().openNativePath(dir())}>
                                Open widgets folder
                            </Button>
                        )}
                    </Show>
                </div>
                <div class="setting-devices-description">
                    Pick a package's widget.json, or a .zip. To write your own, start from a sample in docs/examples/widgets.
                </div>
                <Show when={replacePath()}>
                    {(path) => (
                        <div class="widget-approval">
                            That widget is already installed. Replace it with this one? It will ask for your approval again.
                            <div class="widget-approval-actions">
                                <Button tone="accent" onClick={() => void installFrom(path(), true)}>
                                    Replace
                                </Button>
                                <Button onClick={() => setReplacePath(null)}>Cancel</Button>
                            </div>
                        </div>
                    )}
                </Show>
                <Show when={error()}>
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                </Show>
                <Show
                    when={list().length > 0}
                    fallback={
                        <div class="setting-devices-empty">
                            {widgetPackagesLoaded() ? "No widgets installed." : "Loading…"}
                        </div>
                    }
                >
                    <div class="widgets-list">
                        <For each={list()}>
                            {(pkg) => (
                                <div class="widget-row" data-widget-id={pkg.id}>
                                    <div class="widget-row-head">
                                        <i class={`fa-solid fa-${pkg.icon}`} />
                                        <span class="widget-row-name">{pkg.name}</span>
                                        <span class="widget-row-version">{pkg.version}</span>
                                        <span class="widget-row-kind">{pkg.kind}</span>
                                        <span class={`widget-row-state state-${pkg.state}`}>{STATE_TEXT[pkg.state]}</span>
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
                                    <div class="widget-row-actions">
                                        <Show when={pkg.state === "needs_approval" || pkg.state === "changed"}>
                                            <Button tone="accent" onClick={() => setAsking(pkg.id)}>
                                                {pkg.state === "changed" ? "Review and approve" : "Approve"}
                                            </Button>
                                        </Show>
                                        <Show when={pkg.state === "approved"}>
                                            <Button
                                                disabled={busy() === pkg.id}
                                                onClick={() =>
                                                    void run(pkg.id, async () =>
                                                        (await RpcApi.WidgetsSetEnabledCommand(TabRpcClient, { id: pkg.id, enabled: false })).packages
                                                    )
                                                }
                                            >
                                                Turn off
                                            </Button>
                                        </Show>
                                        <Show when={pkg.state === "disabled"}>
                                            <Button
                                                disabled={busy() === pkg.id}
                                                onClick={() =>
                                                    void run(pkg.id, async () =>
                                                        (await RpcApi.WidgetsSetEnabledCommand(TabRpcClient, { id: pkg.id, enabled: true })).packages
                                                    )
                                                }
                                            >
                                                Turn on
                                            </Button>
                                        </Show>
                                        <Button icon="folder-open" onClick={() => getApi().openNativePath(pkg.folder)}>
                                            Show folder
                                        </Button>
                                        <Show when={!pkg.implied}>
                                            <Show
                                                when={confirmRemove() === pkg.id}
                                                fallback={
                                                    <Button tone="danger" onClick={() => setConfirmRemove(pkg.id)}>
                                                        Uninstall
                                                    </Button>
                                                }
                                            >
                                                <span class="widget-row-confirm">Delete {pkg.name} and its data?</span>
                                                <Button
                                                    tone="danger"
                                                    disabled={busy() === pkg.id}
                                                    onClick={() =>
                                                        void run(pkg.id, async () => {
                                                            setConfirmRemove(null);
                                                            return (await RpcApi.WidgetsUninstallCommand(TabRpcClient, { id: pkg.id })).packages;
                                                        })
                                                    }
                                                >
                                                    Delete
                                                </Button>
                                                <Button onClick={() => setConfirmRemove(null)}>Keep</Button>
                                            </Show>
                                        </Show>
                                    </div>
                                </div>
                            )}
                        </For>
                    </div>
                </Show>
                <Show when={publishers().length > 0}>
                    <div class="widgets-publishers">
                        <div class="setting-devices-description">
                            Publisher keys: the key each publisher's first signed widget was signed with. A later widget of theirs
                            signed with another key gets a warning. Forget a key only if its author told you they changed it.
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
                                                void run(`key:${pin.publisher}`, async () => {
                                                    await getApi().approvals.forgetWidgetKey!(pin.publisher);
                                                    await refreshWidgetPackages();
                                                })
                                            }
                                        >
                                            Forget key
                                        </Button>
                                    </Show>
                                </div>
                            )}
                        </For>
                    </div>
                </Show>
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
            return `Signed by ${s.publisher} · ${s.fingerprint}`;
        case "key_changed":
            return s.fingerprint ? `Signed by another key than ${s.publisher}'s (${s.fingerprint})` : `Not signed, unlike ${s.publisher}'s other widgets`;
        default:
            return "Not signed";
    }
}
