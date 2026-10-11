// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings → Widgets → Browse the catalog
// (docs/specs/SPEC_WIDGET_SHARING_2026_10_10.md §4.5): the AgentMux widget
// catalog's sandboxed widgets, as srv fetched and checked them. Installing
// one copies it in; like any widget, it runs only once the user approves it
// in the prompt that opens next.

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import type { WidgetCatalogItem, WidgetPackageInfo } from "@/app/store/rpc-api/widgets";
import { TabRpcClient } from "@/app/store/rpc-util";
import { describePermission } from "./widget-permissions";

const [items, setItems] = createSignal<WidgetCatalogItem[]>([]);

/** Is `pkg` exactly what the catalog lists: its files, signed by the
 *  publisher key the catalog lists? For the prompt's catalog line. */
export function inCatalog(pkg: Pick<WidgetPackageInfo, "id" | "hash" | "signature">): boolean {
    return items().some((i) => i.entry.id === pkg.id && i.entry.hash === pkg.hash && i.fingerprint === pkg.signature?.fingerprint);
}

/** The line the prompt adds for a package that is in the catalog. */
export const CATALOG_LINE = "From the AgentMux catalog: its files and its publisher's key are the ones the catalog lists.";

export function WidgetCatalog(props: { onInstalled: (id: string, packages: WidgetPackageInfo[]) => void }): JSX.Element {
    const [open, setOpen] = createSignal(false);
    const [loading, setLoading] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal<string | null>(null);

    const load = async () => {
        setLoading(true);
        setError(null);
        try {
            setItems((await RpcApi.WidgetsCatalogCommand(TabRpcClient, { timeout: 60000 })).items);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setLoading(false);
        }
    };

    const install = async (id: string) => {
        setBusy(id);
        setError(null);
        try {
            const r = await RpcApi.WidgetsCatalogInstallCommand(TabRpcClient, { id }, { timeout: 120000 });
            props.onInstalled(r.id, r.packages);
            void load();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(null);
        }
    };

    const action = (i: WidgetCatalogItem): string | null => {
        if (i.current) return null;
        return i.installed_version ? `Update to ${i.entry.version}` : "Install";
    };

    return (
        <div class="widget-catalog">
            <Button
                icon="store"
                onClick={() => {
                    const next = !open();
                    setOpen(next);
                    if (next) void load();
                }}
            >
                {open() ? "Hide the catalog" : "Browse the catalog"}
            </Button>
            <Show when={open()}>
                <div class="setting-devices-description">
                    Sandboxed widgets from the AgentMux catalog. Each one asks for your approval before it runs.
                </div>
                <Show when={error()}>
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                </Show>
                <Show when={!loading()} fallback={<div class="setting-devices-empty">Loading the catalog…</div>}>
                    <Show when={items().length > 0} fallback={<div class="setting-devices-empty">{error() ? "" : "The catalog is empty."}</div>}>
                        <div class="widgets-list">
                            <For each={items()}>
                                {(i) => (
                                    <div class="widget-row" data-catalog-id={i.entry.id}>
                                        <div class="widget-row-head">
                                            <i class={`fa-solid fa-${/^[a-z0-9-]{1,60}$/.test(i.entry.icon ?? "") ? i.entry.icon : "puzzle-piece"}`} />
                                            <span class="widget-row-name">{i.entry.name}</span>
                                            <span class="widget-row-version">{i.entry.version}</span>
                                            <Show when={i.current}>
                                                <span class="widget-row-state state-approved">
                                                    {i.installed_state === "approved" ? "Installed" : "Installed, waiting for your approval"}
                                                </span>
                                            </Show>
                                        </div>
                                        <Show when={i.entry.description}>
                                            <div class="widget-row-description">{i.entry.description}</div>
                                        </Show>
                                        <div class="widget-row-signature">
                                            {i.entry.author ? `By ${i.entry.author} · ` : ""}signed by {i.fingerprint}
                                        </div>
                                        <div class="widget-row-permissions">
                                            {i.entry.permissions.length > 0
                                                ? i.entry.permissions.map((p) => describePermission(p).text).join(" · ")
                                                : "Asks for no permissions."}
                                        </div>
                                        <Show when={action(i)}>
                                            {(label) => (
                                                <div class="widget-row-actions">
                                                    <Button tone="accent" disabled={busy() === i.entry.id} onClick={() => void install(i.entry.id)}>
                                                        {busy() === i.entry.id ? "Installing…" : label()}
                                                    </Button>
                                                </div>
                                            )}
                                        </Show>
                                    </div>
                                )}
                            </For>
                        </div>
                    </Show>
                </Show>
            </Show>
        </div>
    );
}

/** Tests only. */
export function __setCatalogItems(list: WidgetCatalogItem[]): void {
    setItems(list);
}
