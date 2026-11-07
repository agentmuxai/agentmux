// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The create panel's Install step: the run for a CLI AgentMux installs, and the vendor's command for one it doesn't. */

import { createEffect, createMemo, createSignal, onCleanup } from "solid-js";

import { currentPlatform } from "@/app/store/toolchain-catalog";
import { agentInstallSession, onAgentInstallDone, watchAgentInstall } from "../install-sessions";
import type { PanelState } from "./create-panel-state";

export function useInstallStep(s: PanelState) {
    const session = createMemo(() => {
        const id = s.providerId();
        return id && !s.userInstalled() ? agentInstallSession(id) : undefined;
    });
    // This panel shows the install, so its end needs no toast; an install
    // that ends here re-checks the CLI so the Create step follows.
    createEffect(() => {
        const id = s.providerId();
        if (!id || s.userInstalled()) return;
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
        return currentPlatform() === "windows" ? p.windowsInstallCommand : p.unixInstallCommand;
    };

    return { session, checkingAgain, installDone, setInstallDone, checkAgain, vendorCommand };
}

export type InstallStep = ReturnType<typeof useInstallStep>;
