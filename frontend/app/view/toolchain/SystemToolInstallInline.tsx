// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SystemToolInstallInline — one-click install of a system tool (git,
 * node/npm, python) through the platform's own package manager, shown
 * inline below wherever a tool's "not found" row already renders (the
 * Toolchain modal's core-tools list, `AgentPrereqModal`'s missing-prereq
 * list). Not a nested modal — an expand-in-place panel, matching how
 * this app already renders streamed tool output inline elsewhere.
 *
 * State machine: `checking` → (`unavailable` — renders nothing, caller
 * keeps its existing link+copy-command fallback) | `idle` (shows the
 * resolved command + an explicit consent step) → `installing` (streamed
 * log, no cancel button — see below) → `done` | `failed`.
 *
 * Renders `null` whenever `toolchain.resolve_install_command` reports
 * unavailable (no package manager detected/usable) — the caller's own
 * existing fallback UI (install URL + copyable command) is what shows in
 * that case; this component never tries to replace or hide that.
 *
 * SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md §3.3-§3.4.
 */

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import "./SystemToolInstallInline.scss";

type Phase = "checking" | "unavailable" | "idle" | "installing" | "done" | "failed";

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
}

export const SystemToolInstallInline = (props: SystemToolInstallInlineProps): JSX.Element => {
    const [phase, setPhase] = createSignal<Phase>("checking");
    const [commandPreview, setCommandPreview] = createSignal("");
    const [needsElevation, setNeedsElevation] = createSignal(false);
    const [lines, setLines] = createSignal<Array<{ line: string; stream: "stdout" | "stderr" }>>([]);
    const [error, setError] = createSignal<string | null>(null);

    let unsub: (() => void) | null = null;
    // Deliberately NOT auto-cancelled on unmount, unlike
    // AgentInstallModalPanel's npm-into-an-isolated-dir install: once a
    // system package-manager transaction has actually started (dpkg/MSI/
    // brew mid-write), killing it because the user navigated away from
    // this panel is the same "leave it in a broken half-installed state"
    // risk this component's own UI already avoids by never offering a
    // cancel button past this point (SPEC §3.4). It runs to completion in
    // the background; the session cleans itself up server-side when the
    // child exits, listener or not.
    let disposed = false;

    onMount(async () => {
        try {
            const r = await RpcApi.ToolchainResolveInstallCommandCommand(TabRpcClient, { toolId: props.toolId });
            if (disposed) return;
            if (!r.available) {
                setPhase("unavailable");
                props.onUnavailable?.();
                return;
            }
            setCommandPreview(r.commandPreview);
            setNeedsElevation(r.needsElevation);
            setPhase("idle");
        } catch {
            // Treat a failed probe the same as "unavailable" — the
            // caller's link+copy-command fallback is always a safe
            // landing spot, never a dead end.
            if (!disposed) {
                setPhase("unavailable");
                props.onUnavailable?.();
            }
        }
    });

    onCleanup(() => {
        disposed = true;
        if (unsub) {
            unsub();
            unsub = null;
        }
    });

    const startInstall = async () => {
        // Tear down any prior run (Retry path) — without this, retrying
        // after a failure overwrites `unsub` with the new subscription's
        // teardown, leaking the previous one (it's then never called,
        // including on eventual component unmount). reagent P2, PR #2790.
        if (unsub) {
            unsub();
            unsub = null;
        }
        setPhase("installing");
        setError(null);
        setLines([]);
        try {
            const r = await RpcApi.ToolchainInstallSystemToolCommand(TabRpcClient, { toolId: props.toolId });
            if (disposed) return;
            unsub = waveEventSubscribe({
                eventType: "install_chunk",
                scope: `install:${r.sessionId}`,
                handler: (event: any) => {
                    const data = event?.data;
                    if (!data || typeof data !== "object") return;
                    if (typeof data.line === "string") {
                        setLines((prev) => [...prev, { line: data.line, stream: data.stream === "stderr" ? "stderr" : "stdout" }]);
                    } else if (data.op === "done") {
                        if (data.ok) {
                            setPhase("done");
                            props.onInstalled();
                        } else {
                            setError(data.error ?? "install failed");
                            setPhase("failed");
                        }
                    }
                },
            });
        } catch (e) {
            setError((e as Error)?.message ?? String(e));
            setPhase("failed");
        }
    };

    return (
        <Show when={phase() !== "checking" && phase() !== "unavailable"}>
            <div class="system-tool-install-inline">
                <Show when={phase() === "idle"}>
                    <div class="system-tool-install-consent">
                        <p class="system-tool-install-consent-text">
                            This will run:
                        </p>
                        <code class="system-tool-install-command">{commandPreview()}</code>
                        <Show when={needsElevation()}>
                            <p class="system-tool-install-elevation-note">
                                <i class="fa-solid fa-shield-halved" aria-hidden="true" />{" "}
                                This will ask for your password or show a system permission prompt.
                            </p>
                        </Show>
                        <Button onClick={() => void startInstall()} className="green solid">
                            Install
                        </Button>
                    </div>
                </Show>
                <Show when={phase() === "installing" || phase() === "done" || phase() === "failed"}>
                    <div class="system-tool-install-log" classList={{ "is-done": phase() === "done", "is-failed": phase() === "failed" }}>
                        <div class="system-tool-install-log-header">
                            <Show when={phase() === "installing"}>
                                <span class="system-tool-install-spinner" aria-hidden="true">⏳</span> Installing…
                            </Show>
                            <Show when={phase() === "done"}>
                                <span class="system-tool-install-ok" aria-hidden="true">✓</span> Installed
                            </Show>
                            <Show when={phase() === "failed"}>
                                <span class="system-tool-install-fail" aria-hidden="true">✗</span> Failed
                            </Show>
                        </div>
                        <pre class="system-tool-install-log-body">
                            {lines().map((l) => l.line).join("\n")}
                        </pre>
                        <Show when={error()}>
                            <div class="system-tool-install-error">{String(error())}</div>
                        </Show>
                        <Show when={phase() === "failed"}>
                            <Button onClick={() => void startInstall()}>Retry</Button>
                        </Show>
                    </div>
                </Show>
            </div>
        </Show>
    );
};

SystemToolInstallInline.displayName = "SystemToolInstallInline";
