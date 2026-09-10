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

import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Button } from "@/element/button";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { CORE_TOOLS } from "@/app/view/agent/providers/toolchain-catalog";
import "./SystemToolInstallInline.scss";

type Phase = "checking" | "unavailable" | "idle" | "installing" | "done" | "failed";

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
    const [phase, setPhase] = createSignal<Phase>("checking");
    const [commandPreview, setCommandPreview] = createSignal("");
    const [needsElevation, setNeedsElevation] = createSignal(false);
    const [resolvedVersion, setResolvedVersion] = createSignal<string | null>(null);
    const [lines, setLines] = createSignal<Array<{ line: string; stream: "stdout" | "stderr" }>>([]);
    const [error, setError] = createSignal<string | null>(null);

    // Brand icon for the tool being installed (SPEC_SYSTEM_TOOL_INSTALL_
    // DETAILS_AUTOSCROLL_2026_09_10.md §6) — a plain frontend-only lookup,
    // same as `wingetId`/`brewFormula` elsewhere in this catalog; never
    // sent to or resolved by the backend.
    const brandIcon = () => CORE_TOOLS.find((t) => t.id === props.toolId)?.brandIcon;

    // Auto-stick-to-bottom for the streamed log, mirroring ToolOverlayLog.tsx's
    // proven pattern (SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md) rather than
    // inventing a new one — see SPEC_SYSTEM_TOOL_INSTALL_DETAILS_AUTOSCROLL_
    // 2026_09_10.md §3. `stickToBottom` is a plain `let`, not a signal: it's
    // read only inside the scroll handler and the log-lines effect below,
    // neither of which needs Solid to track it reactively.
    let stickToBottom = true;
    let logBodyRef: HTMLPreElement | undefined;
    const onLogScroll = () => {
        if (!logBodyRef) return;
        const dist = logBodyRef.scrollHeight - logBodyRef.scrollTop - logBodyRef.clientHeight;
        stickToBottom = dist < 40; // forgiving threshold — one mousewheel tick must not unstick
    };

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
        if (props.resolvedInfo) {
            setCommandPreview(props.resolvedInfo.commandPreview);
            setNeedsElevation(props.resolvedInfo.needsElevation);
            setResolvedVersion(props.resolvedInfo.resolvedVersion ?? null);
            setPhase("idle");
            return;
        }
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
            setResolvedVersion(r.resolvedVersion ?? null);
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

    // Re-sync scroll position when the native <details> panel opens.
    // A closed <details> doesn't lay out its children, so scrollHeight/
    // scrollTop writes against logBodyRef while collapsed are no-ops (or
    // measure a zero-size box) — the same class of problem ToolOverlayLog.tsx
    // solves with its `panelHidden` (content-visibility) guard, just
    // triggered by native <details> semantics instead of a CSS class here.
    // Without this, reopening mid-install (or after it finishes) would land
    // on whatever stale scrollTop was last set while visible, not the
    // newest line.
    //
    // A REF CALLBACK, not a plain `ref={detailsRef}` + top-level `onMount`:
    // this <details> element only exists inside the installing/done/failed
    // `<Show>` branch below, which isn't mounted yet when the component
    // itself first mounts (phase starts at "checking"/"idle"). A one-time
    // onMount reading `detailsRef` at that point would always see
    // `undefined` and silently never attach anything. A ref callback is
    // invoked by Solid whenever THIS element is actually created — i.e.
    // when the Show branch renders it — so it fires at the right time
    // regardless of which phase the component happened to mount in.
    const bindDetailsToggle = (el: HTMLDetailsElement) => {
        const onToggle = () => {
            if (el.open && stickToBottom && logBodyRef) {
                requestAnimationFrame(() => {
                    if (logBodyRef && logBodyRef.isConnected) {
                        logBodyRef.scrollTop = logBodyRef.scrollHeight;
                    }
                });
            }
        };
        el.addEventListener("toggle", onToggle);
        onCleanup(() => el.removeEventListener("toggle", onToggle));
    };

    // Auto-stick to bottom as new lines stream in, same shape as
    // ToolOverlayLog.tsx's own log-following effect.
    createEffect(() => {
        lines(); // register as a reactive dependency
        if (stickToBottom && logBodyRef) {
            // Wait one frame for the DOM to flush before measuring —
            // mirrors ToolOverlayLog.tsx's own effect.
            requestAnimationFrame(() => {
                if (logBodyRef && logBodyRef.isConnected) {
                    logBodyRef.scrollTop = logBodyRef.scrollHeight;
                }
            });
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
        // A retry after scrolling up to inspect a prior failure should
        // start pinned to the bottom again, not inherit the previous run's
        // scroll-away state.
        stickToBottom = true;
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
                            <Show when={brandIcon()}>
                                <i class={`system-tool-install-brand-icon fa-brands fa-${brandIcon()}`} aria-hidden="true" />
                            </Show>
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
                            {resolvedVersion() ? `Install v${resolvedVersion()} now` : "Install"}
                        </Button>
                    </div>
                </Show>
                <Show when={phase() === "installing" || phase() === "done" || phase() === "failed"}>
                    <div class="system-tool-install-log" classList={{ "is-done": phase() === "done", "is-failed": phase() === "failed" }}>
                        <div class="system-tool-install-log-header">
                            <Show when={brandIcon()}>
                                <i class={`system-tool-install-brand-icon fa-brands fa-${brandIcon()}`} aria-hidden="true" />
                            </Show>
                            <Show when={phase() === "installing"}>
                                <i class="fa-solid fa-spinner fa-spin" aria-hidden="true" /> Installing…
                            </Show>
                            <Show when={phase() === "done"}>
                                <span class="system-tool-install-ok" aria-hidden="true">✓</span> Installed
                            </Show>
                            <Show when={phase() === "failed"}>
                                <span class="system-tool-install-fail" aria-hidden="true">✗</span> Failed
                            </Show>
                        </div>
                        <Show when={phase() === "installing"}>
                            <div class="install-progress" aria-hidden="true">
                                <div class="install-progress-bar" />
                            </div>
                        </Show>
                        <details class="system-tool-install-details" ref={bindDetailsToggle}>
                            <summary>Details</summary>
                            <pre class="system-tool-install-log-body" ref={logBodyRef} onScroll={onLogScroll}>
                                {lines().map((l) => l.line).join("\n")}
                            </pre>
                        </details>
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
