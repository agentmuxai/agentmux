// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TabModalLayer types + context — per-tab modal scoping.
 *
 * Each `<TabContent>` wraps its tile layout in a `<TabModalLayer>` which
 * provides this context. Components inside a tab call `useTabModal()` to
 * open and close tab-scoped modals (e.g., AgentLaunchModal). The layer
 * handles rendering the overlay; consumers only declare the request.
 *
 * See docs/specs/launch-modal-rearchitecture-2026-05-01.md.
 */

import { createContext, useContext, type Accessor } from "solid-js";

// ── Request shape ────────────────────────────────────────────────────────────
//
// Discriminated union so the layer can dispatch on `kind`. New tab-modal
// surfaces add a variant here and a render branch in TabModalLayer.

export type TabModalRequest = LaunchAgentRequest | InstallAgentRequest;

export interface LaunchAgentRequest {
    kind: "launch-agent";
    /** Forge definition the user clicked. */
    agent: ForgeAgent;
    /** Block id of the pane that opened the modal. */
    originBlockId: string;
    /**
     * Called by the layer when the user submits valid form data. Returns
     * a promise so the modal can show "Launching…" until it resolves; on
     * resolve, the layer closes the modal automatically. If the promise
     * rejects, the modal stays open and surfaces the error.
     */
    onSubmit: (overrides: LaunchAgentSubmit) => Promise<void> | void;
}

export interface LaunchAgentSubmit {
    instanceName: string;
    agentType: "host" | "container";
    environment: "local" | "docker";
    containerImage?: string;
}

/**
 * Install-agent modal — opens when the user picks an agent whose CLI
 * isn't installed in the per-version cache. Streams the npm-install
 * output live inside the modal. On success the layer auto-transitions
 * to the launch-agent modal (via `onInstalled`). Per
 * SPEC_AGENT_INSTALL_STAGE_2026_05_17.md §6.
 */
export interface InstallAgentRequest {
    kind: "install-agent";
    agent: ForgeAgent;
    originBlockId: string;
    /**
     * Called by the modal once the install completes successfully. The
     * boolean reflects which terminal button the user clicked:
     *  - `true`  — "Continue to Launch": flip install state AND open
     *    the launch modal as the natural next step.
     *  - `false` — "Close": flip install state but do not chain.
     *
     * Callers MUST flip cached install state in both branches; codex
     * caught a regression on PR #895 where the Close path skipped the
     * flip and stranded users on a stale ribbon.
     */
    onInstalled: (continueToLaunch: boolean) => void;
}

// ── Context API ──────────────────────────────────────────────────────────────

export interface TabModalApi {
    /** Open or replace the current tab modal request. */
    open: (req: TabModalRequest) => void;
    /**
     * Replace the current modal with `next` as a continuation of the
     * same flow. The backdrop + outer panel stay mounted across the
     * swap; only the panel content remounts with a content-fade
     * animation. Falls back to `open(next)` when no modal is open.
     *
     * See docs/specs/SPEC_MODAL_TRANSITIONS_2026_05_18.md.
     */
    replace: (next: TabModalRequest) => void;
    /** Close the current modal, if any. No-op when nothing is open. */
    close: () => void;
    /** The currently open request, or null. */
    current: Accessor<TabModalRequest | null>;
}

export const TabModalContext = createContext<TabModalApi | null>(null);

/**
 * Access the tab-scoped modal API. Throws when called outside a
 * `<TabModalLayer>` so misuses surface immediately rather than silently
 * no-op'ing.
 */
export function useTabModal(): TabModalApi {
    const ctx = useContext(TabModalContext);
    if (!ctx) {
        throw new Error("useTabModal called outside <TabModalLayer>");
    }
    return ctx;
}
