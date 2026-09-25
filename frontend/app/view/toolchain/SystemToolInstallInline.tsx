// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SystemToolInstallInline — one-click install of a system tool (git,
 * node/npm, python) through the platform's own package manager, shown
 * inline below wherever a tool's "not found" row already renders (the
 * Toolchain modal's core-tools list, `AgentPrereqModal`'s missing-prereq
 * list). Not a nested modal — an expand-in-place panel.
 *
 * State machine: `checking` → (`unavailable` — renders nothing, caller
 * keeps its existing link+copy-command fallback) | `ready` (shows the
 * resolved command + an explicit consent step) → the shared install
 * session (`<InstallProgress>`: plain steps, Details collapsed, no cancel
 * button — see below).
 *
 * Renders `null` whenever `toolchain.resolve_install_command` reports
 * unavailable (no package manager detected/usable) — the caller's own
 * existing fallback UI (install URL + copyable command) is what shows in
 * that case; this component never tries to replace or hide that.
 *
 * SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md §3.3-§3.4;
 * SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §5.2 (Toolchain pane rows).
 */

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { InstallConfirm } from "@/element/install/InstallConfirm";
import { InstallProgress } from "@/element/install/InstallProgress";
import { createInstallSession } from "@/element/install/install-session";
import { SystemStepTracker } from "@/element/install/system-steps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { CORE_TOOLS } from "@/app/view/agent/providers/toolchain-catalog";
import "./SystemToolInstallInline.scss";

type Resolution = "checking" | "unavailable" | "ready";

interface ResolvedInstallInfo {
    commandPreview: string;
    needsElevation: boolean;
    /** Version this exact command would install, queried live from the
     *  package manager's own catalog — `null`/absent when the query
     *  failed or isn't implemented for this platform. Never a hardcoded
     *  guess (see #2942). */
    resolvedVersion?: string | null;
}

interface SystemToolInstallInlineProps {
    toolId: string;
    /** Fires once, on a successful install — the caller re-probes its
     *  own row/prereq state (this component doesn't know how). */
    onInstalled: () => void;
    /** Fires once, when resolution completes as unavailable (no package
     *  manager detected/usable on this machine) — callers use this to
     *  collapse/hide whatever toggle exposed this panel so a user who
     *  clicked "install now" doesn't end up staring at a permanently
     *  blank expanded area with no visible fallback. reagent P2,
     *  PR #2790. */
    onUnavailable?: () => void;
    /** Pre-resolved install info from the caller (e.g. a parent that
     *  already resolved every missing tool's command/version up front to
     *  label its own toggle button) — when provided, this component skips
     *  its own `toolchain.resolve_install_command` call and uses this
     *  directly instead. Callers that don't pre-resolve (e.g. the
     *  Toolchain modal's core-tools list) omit this and keep the
     *  existing self-resolve-on-mount behavior. */
    resolvedInfo?: ResolvedInstallInfo;
}

export const SystemToolInstallInline = (props: SystemToolInstallInlineProps): JSX.Element => {
    const [resolution, setResolution] = createSignal<Resolution>("checking");
    const [info, setInfo] = createSignal<ResolvedInstallInfo>({ commandPreview: "", needsElevation: false });

    const tool = () => CORE_TOOLS.find((t) => t.id === props.toolId);
    // Brand icon for the tool being installed (SPEC_SYSTEM_TOOL_INSTALL_
    // DETAILS_AUTOSCROLL_2026_09_10.md §6) — a plain frontend-only lookup,
    // never sent to or resolved by the backend.
    const brandIcon = () => tool()?.brandIcon;

    // No `cancel`: once a system package-manager transaction has started
    // (dpkg/MSI/brew mid-write), killing it — by a button or because the
    // user navigated away from this panel — risks a broken half-installed
    // state (SPEC §3.4). It runs to completion in the background; the
    // session cleans itself up server-side when the child exits.
    const session = createInstallSession({
        tracker: () => new SystemStepTracker({ name: tool()?.label ?? props.toolId, needsElevation: info().needsElevation }),
        begin: () => RpcApi.ToolchainInstallSystemToolCommand(TabRpcClient, { toolId: props.toolId }),
        cancelOnDispose: false,
        onDone: (ok) => {
            if (ok) props.onInstalled();
        },
    });

    let disposed = false;
    onCleanup(() => {
        disposed = true;
    });

    onMount(async () => {
        if (props.resolvedInfo) {
            setInfo(props.resolvedInfo);
            setResolution("ready");
            return;
        }
        try {
            const r = await RpcApi.ToolchainResolveInstallCommandCommand(TabRpcClient, { toolId: props.toolId });
            if (disposed) return;
            if (!r.available) {
                setResolution("unavailable");
                props.onUnavailable?.();
                return;
            }
            setInfo({ commandPreview: r.commandPreview, needsElevation: r.needsElevation, resolvedVersion: r.resolvedVersion });
            setResolution("ready");
        } catch {
            // Treat a failed probe the same as "unavailable" — the
            // caller's link+copy-command fallback is always a safe
            // landing spot, never a dead end.
            if (!disposed) {
                setResolution("unavailable");
                props.onUnavailable?.();
            }
        }
    });

    const status = () => {
        switch (session.state()) {
            case "running":
                return "Installing…";
            case "done":
                return "Installed";
            case "failed":
                return "Failed";
            default:
                return "";
        }
    };

    return (
        <Show when={resolution() === "ready"}>
            <div class="system-tool-install-inline">
                <Show
                    when={session.state() !== "idle"}
                    fallback={
                        <InstallConfirm
                            commandPreview={info().commandPreview}
                            needsElevation={info().needsElevation}
                            brandIcon={brandIcon()}
                        >
                            <Button onClick={() => void session.start()} className="green solid">
                                {info().resolvedVersion ? `Install v${info().resolvedVersion} now` : "Install"}
                            </Button>
                        </InstallConfirm>
                    }
                >
                    <div class="system-tool-install-status">
                        <Show when={brandIcon()}>
                            <i class={`system-tool-install-brand-icon fa-brands fa-${brandIcon()}`} aria-hidden="true" />
                        </Show>
                        {status()}
                    </div>
                    <InstallProgress session={session} kind="system-tool" />
                    <Show when={session.state() === "failed"}>
                        <div>
                            <Button onClick={() => void session.start()}>Retry</Button>
                        </div>
                    </Show>
                </Show>
            </div>
        </Show>
    );
};

SystemToolInstallInline.displayName = "SystemToolInstallInline";
