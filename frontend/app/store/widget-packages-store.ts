// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// srv's list of widget packages (`widgets.list`), kept current from the
// `widgetpackages` event (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md
// §8.1). The widget loader loads what's approved; Settings → Widgets shows all.

import { createSignal, type Accessor } from "solid-js";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import type { WidgetPackageInfo } from "@/app/store/rpc-api/widgets";
import { TabRpcClient } from "@/app/store/rpc-util";

const [packages, setPackages] = createSignal<WidgetPackageInfo[]>([]);
const [loaded, setLoaded] = createSignal(false);
let started: Promise<void> | null = null;

/** Replace the list (an RPC that returns the new list calls this). */
export function setWidgetPackages(list: WidgetPackageInfo[] | null | undefined): void {
    setPackages(list ?? []);
    setLoaded(true);
}

/** Fetch the list now. A failure keeps the last one. */
export async function refreshWidgetPackages(): Promise<void> {
    try {
        setWidgetPackages((await RpcApi.WidgetsListCommand(TabRpcClient, { timeout: 5000 }))?.packages);
    } catch (e) {
        console.log("widget packages: could not load the list", e);
        setLoaded(true);
    }
}

/** Start following srv's list; the promise settles after the first fetch. */
export function startWidgetPackages(): Promise<void> {
    if (started) return started;
    muxEventSubscribe({
        eventType: WpsEvent.WidgetPackages,
        handler: (event: { data?: { packages?: WidgetPackageInfo[] } }) => setWidgetPackages(event?.data?.packages),
    });
    started = refreshWidgetPackages();
    return started;
}

/** The list, kept current once started. */
export function widgetPackages(): Accessor<WidgetPackageInfo[]> {
    void startWidgetPackages();
    return packages;
}

/** Whether the first list has arrived. */
export function widgetPackagesLoaded(): boolean {
    return loaded();
}
