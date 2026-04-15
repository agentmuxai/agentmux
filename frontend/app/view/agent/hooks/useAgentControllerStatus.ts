// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentControllerStatus — owns all the agent-launch state and the
 * functions that drive it.
 *
 * Step 4 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * State managed:
 *   - authUrl       — the OAuth URL to display when login is needed
 *   - canRetry      — whether the retry button is shown after auth_failed
 *   - flowRunning   — true while the launch flow is actively executing
 *   - agentReady    — true once the controller is registered and ready
 *   - isLoading     — derived: flowRunning OR not agentReady
 *   - loginWaiting  — true during the OAuth poll phase
 *
 * Mutable internal state:
 *   - loginCancelled — flipped by cancelLogin() and by onCleanup
 *
 * Functions exposed:
 *   - startLaunchFlow() — runs the full launch flow once
 *   - cancelLogin()     — flips the cancellation flag and kills the host CLI
 *
 * Dependencies (passed as options):
 *   - blockId   — the block this status belongs to
 *   - provider  — accessor returning the current provider definition
 *   - log       — LogFn from useLaunchLogs (or any compatible)
 *
 * `setAuthUrl` is exposed alongside the accessor because the slash-command
 * `/login` handler in agent-view.tsx needs to manually set the OAuth URL
 * when the user types the command directly (separate from the launch
 * flow's auto-login phase).
 */

import { createMemo, createSignal, onCleanup, type Accessor } from "solid-js";
import { getApi } from "@/app/store/global";
import { runLaunchFlow } from "../flows/launch-flow";
import type { ProviderDefinition } from "../providers";

import type { LogFn } from "../types";
export type { LogFn };

export interface UseAgentControllerStatusOptions {
    blockId: string;
    provider: Accessor<ProviderDefinition | undefined>;
    log: LogFn;
    onLoginSuccess?: (email: string | null) => void;
}

export interface UseAgentControllerStatus {
    authUrl: Accessor<string | null>;
    setAuthUrl: (url: string | null) => void;
    canRetry: Accessor<boolean>;
    flowRunning: Accessor<boolean>;
    agentReady: Accessor<boolean>;
    isLoading: Accessor<boolean>;
    loginWaiting: Accessor<boolean>;
    startLaunchFlow: () => Promise<void>;
    cancelLogin: () => void;
}

/**
 * Build the auth env vars (CLAUDE_CONFIG_DIR etc.) from a provider
 * definition. Async because it ensures the auth dir exists on disk via
 * the host API. Returns undefined on any failure (non-fatal — falls back
 * to whatever the host's default state is).
 */
async function buildAuthEnv(
    prov: ProviderDefinition | undefined,
): Promise<Record<string, string> | undefined> {
    if (!prov?.authConfigDirEnvVar || !prov?.authDirName) return undefined;
    try {
        const authDir = await getApi().ensureAuthDir(prov.id);
        const env: Record<string, string> = { [prov.authConfigDirEnvVar]: authDir };
        if (prov.authExtraEnv) Object.assign(env, prov.authExtraEnv);
        return env;
    } catch {
        return undefined; // non-fatal — fall back to default auth dir
    }
}

export function useAgentControllerStatus(
    opts: UseAgentControllerStatusOptions,
): UseAgentControllerStatus {
    const [authUrl, setAuthUrl] = createSignal<string | null>(null);
    const [canRetry, setCanRetry] = createSignal(false);
    const [flowRunning, setFlowRunning] = createSignal(false);
    const [agentReady, setAgentReady] = createSignal(false);
    const [loginWaiting, setLoginWaiting] = createSignal(false);

    // Derived spinner state — caller wires this into the AgentFooter loading prop
    const isLoading = createMemo(() => flowRunning() || !agentReady());

    // Mutable cancellation flag. Flipped by cancelLogin() and by onCleanup.
    // Read inside startLaunchFlow's polling loop via the isCancelled callback.
    let loginCancelled = false;

    const startLaunchFlow = async () => {
        if (flowRunning()) return;
        loginCancelled = false;
        setFlowRunning(true);
        setCanRetry(false);
        const prov = opts.provider();
        try {
            const authEnv = await buildAuthEnv(prov);
            const result = await runLaunchFlow({
                blockId: opts.blockId,
                provider: prov,
                log: opts.log,
                setAuthUrl,
                isCancelled: () => loginCancelled,
                setLoginWaiting,
                authEnv,
                onLoginSuccess: opts.onLoginSuccess,
            });
            if (result === "success") {
                setAgentReady(true);
            } else if (result === "auth_failed" && !loginCancelled) {
                setCanRetry(true);
                setAgentReady(true); // clear spinner so retry button is usable
            }
        } catch (err: any) {
            opts.log("error", err?.message ?? String(err), "error");
            setAgentReady(true); // clear spinner on error
        } finally {
            setFlowRunning(false);
        }
    };

    const cancelLogin = () => {
        loginCancelled = true;
        getApi().cancelCliLogin().catch(() => {});
        opts.log("auth", "login cancelled", "warn");
    };

    // If the pane is closed while login is in progress, cancel and kill
    // the host CLI process. This onCleanup is registered against the
    // SolidJS owner context that called useAgentControllerStatus — the
    // agent presentation view's component scope.
    onCleanup(() => {
        if (loginWaiting()) {
            loginCancelled = true;
            getApi().cancelCliLogin().catch(() => {});
        }
    });

    return {
        authUrl,
        setAuthUrl,
        canRetry,
        flowRunning,
        agentReady,
        isLoading,
        loginWaiting,
        startLaunchFlow,
        cancelLogin,
    };
}
