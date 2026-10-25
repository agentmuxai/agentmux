// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One npm install session per provider CLI, outliving the panel that started
 * it (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md §2.3). Closing the new-agent
 * panel under a harness card doesn't stop its install: the card shows
 * "Installing…", reopening the panel shows the same run, and when it ends with
 * no panel open a toast says so. Cancel is explicit.
 *
 * Sessions live under their own reactive root for the life of the page, one
 * per provider, so a `createInstallSession` (which must have an owner) isn't
 * tied to a component.
 */

import { createRoot, createSignal } from "solid-js";

import { createInstallSession, type InstallSession } from "@/element/install/install-session";
import { NpmStepTracker } from "@/element/install/npm-steps";
import { pushNotification } from "@/app/store/flash-notifications";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

import { getCliCatalogEntry } from "./defaults/cli-catalog";
import { invalidateInstallCheck } from "./install-check-cache";
import { getProvider } from "./providers";

interface Entry {
    session: InstallSession;
    /** Open panels showing this install; a toast only when none is. */
    watchers: () => number;
    setWatchers: (fn: (n: number) => number) => void;
    listeners: Set<(ok: boolean) => void>;
}

const entries = new Map<string, Entry>();
/** Bumped when a session is created, so a reader that looked before it existed re-runs. */
const [created, bumpCreated] = createSignal(0);

function displayName(providerId: string): string {
    return getCliCatalogEntry(providerId)?.displayName ?? getProvider(providerId)?.displayName ?? providerId;
}

function entryFor(providerId: string): Entry {
    const existing = entries.get(providerId);
    if (existing) return existing;
    const entry = createRoot(() => {
        const [watchers, setWatchers] = createSignal(0);
        const listeners = new Set<(ok: boolean) => void>();
        const session = createInstallSession({
            tracker: () => new NpmStepTracker(displayName(providerId)),
            begin: async () => {
                const prov = getProvider(providerId);
                if (!prov?.npmPackage) throw new Error(`${displayName(providerId)} isn't installed by AgentMux`);
                // srv installs its own registry's package for this id.
                return RpcApi.InstallStartCommand(TabRpcClient, { providerId: prov.id });
            },
            cancel: (sessionId) => RpcApi.InstallCancelCommand(TabRpcClient, { sessionId }),
            // The root is never disposed; a run stops only on Cancel.
            cancelOnDispose: false,
            onDone: (ok) => {
                // Every picker and the Toolchain must stop serving the
                // pre-install answer.
                invalidateInstallCheck(providerId);
                for (const cb of [...listeners]) cb(ok);
                if (watchers() > 0) return;
                const name = displayName(providerId);
                pushNotification({
                    icon: ok ? "fa-check" : "fa-triangle-exclamation",
                    title: ok ? `${name} is installed` : `${name} didn't install`,
                    message: ok
                        ? `Click its card under New Agent to create the agent.`
                        : (session.failure()?.message ?? "Open its card in a new agent pane to see why, and retry."),
                    timestamp: new Date().toISOString(),
                    type: ok ? "info" : "error",
                    expiration: Date.now() + (ok ? 10_000 : 30_000),
                });
            },
        });
        return { session, watchers, setWatchers, listeners };
    });
    entries.set(providerId, entry);
    bumpCreated((n) => n + 1);
    return entry;
}

/** The install session for `providerId`'s CLI, created on first use. */
export function agentInstallSession(providerId: string): InstallSession {
    return entryFor(providerId).session;
}

/** Whether `providerId`'s CLI is being installed right now. Reactive. */
export function isAgentInstalling(providerId: string): boolean {
    created();
    return entries.get(providerId)?.session.state() === "running";
}

/**
 * Marks a panel as showing `providerId`'s install, so its end needs no toast.
 * Returns the release, for the panel's cleanup.
 */
export function watchAgentInstall(providerId: string): () => void {
    const entry = entryFor(providerId);
    entry.setWatchers((n) => n + 1);
    let released = false;
    return () => {
        if (released) return;
        released = true;
        entry.setWatchers((n) => Math.max(0, n - 1));
    };
}

/** Runs `cb` each time an install of `providerId` ends. Returns the unsubscribe. */
export function onAgentInstallDone(providerId: string, cb: (ok: boolean) => void): () => void {
    const entry = entryFor(providerId);
    entry.listeners.add(cb);
    return () => entry.listeners.delete(cb);
}

/** Test-only: forget every session. */
export function __resetAgentInstallSessions(): void {
    entries.clear();
}
