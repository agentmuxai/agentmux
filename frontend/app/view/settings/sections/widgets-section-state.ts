// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What Settings → Widgets holds (widgets-section.tsx lays it out): the package list and the publishers' pinned keys,
// which RPC is out, which widget is being asked about, and the calls that install, rescan, reveal and run the rest.

import { createSignal } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import type { WidgetPackageInfo } from "@/app/store/rpc-api/widgets";
import { TabRpcClient } from "@/app/store/rpc-util";
import { setWidgetPackages, widgetPackages, widgetPublishers } from "@/app/store/widget-packages-store";
import { agentmuxHome } from "@/app/view/agent/agent-launch-env";
import { getApi } from "@/util/app-api";

export function errorText(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
}

export function createWidgetsSection() {
    const list = widgetPackages();
    const publishers = widgetPublishers();
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal<string | null>(null);
    const [asking, setAsking] = createSignal<string | null>(null);
    const [confirmRemove, setConfirmRemove] = createSignal<string | null>(null);
    const [replacePath, setReplacePath] = createSignal<string | null>(null);

    const reveal = async (path: string) => {
        try {
            await getApi().revealInFileExplorer(path);
            setError(null);
        } catch (e) {
            setError(`Couldn't open ${path}: ${errorText(e)}`);
        }
    };
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
    return { list, publishers, error, busy, asking, setAsking, confirmRemove, setConfirmRemove, replacePath, setReplacePath, reveal, run, installFrom, install, widgetsDir };
}

export type WidgetsSectionState = ReturnType<typeof createWidgetsSection>;
