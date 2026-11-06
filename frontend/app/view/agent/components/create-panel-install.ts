// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The create panel's Install step: the run for a CLI AgentMux installs, and the vendor's command for one it doesn't. */

import { createEffect, createMemo, createSignal, onCleanup } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { currentPlatform } from "@/app/store/toolchain-catalog";
import { agentInstallSession, onAgentInstallDone, watchAgentInstall } from "../install-sessions";
import type { VendorInstallInfo } from "@/types/rpc/VendorInstallInfo";
import type { PanelState } from "./create-panel-state";

export function useInstallStep(s: PanelState) {
    // For a CLI AgentMux doesn't install into its own folder: whether srv has the vendor's
    // installer to run for it (the six), and the command it would show. Null for any other.
    const [vendorInfo, setVendorInfo] = createSignal<VendorInstallInfo | null>(null);
    createEffect(() => {
        const id = s.providerId();
        setVendorInfo(null);
        if (!id || !s.userInstalled()) return;
        let stale = false;
        onCleanup(() => {
            stale = true;
        });
        RpcApi.ToolchainVendorInstallInfoCommand(TabRpcClient, { providerId: id })
            .then((info) => {
                if (!stale && info.available) setVendorInfo(info);
            })
            // No Run: the command, Copy and the vendor's link still work.
            .catch(() => {});
    });
    // The install this panel shows: AgentMux's own npm run, or the vendor's installer srv can run.
    const session = createMemo(() => {
        const id = s.providerId();
        return id && (!s.userInstalled() || vendorInfo()) ? agentInstallSession(id) : undefined;
    });
    // This panel shows the install, so its end needs no toast; an install
    // that ends here re-checks the CLI so the Create step follows.
    createEffect(() => {
        const id = s.providerId();
        if (!id || (s.userInstalled() && !vendorInfo())) return;
        const release = watchAgentInstall(id);
        const unsubscribe = onAgentInstallDone(id, (ok) => {
            if (ok) void s.props.recheckInstalled();
        });
        onCleanup(() => {
            release();
            unsubscribe();
        });
    });
    const [checkingAgain, setCheckingAgain] = createSignal(false);
    // The install finished here and the user chose Continue: go on to Create
    // even if the re-check hasn't caught up yet (the launch looks again).
    const [installDone, setInstallDone] = createSignal(false);
    const checkAgain = async () => {
        setCheckingAgain(true);
        try {
            await s.props.recheckInstalled();
        } finally {
            setCheckingAgain(false);
        }
    };
    const vendorCommand = () => {
        const p = s.provider();
        if (!p) return "";
        // srv's own text, which is what Run asks the person to approve.
        const fromSrv = vendorInfo()?.command;
        if (fromSrv) return fromSrv;
        return currentPlatform() === "windows" ? p.windowsInstallCommand : p.unixInstallCommand;
    };

    return { session, vendorInfo, checkingAgain, installDone, setInstallDone, checkAgain, vendorCommand };
}

export type InstallStep = ReturnType<typeof useInstallStep>;
