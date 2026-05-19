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

export type TabModalRequest =
    | LaunchAgentRequest
    | InstallAgentRequest
    | AgentPrereqRequest
    | NewIdentityBundleRequest
    | NewMemoryBundleRequest
    | BrowserAuthRequest;

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
    /** Initial form state for the launch modal — used by the
     *  "+ New" → create → replace-back flow to restore the user's
     *  in-progress edits (name, runtime, image, identity, memory)
     *  across the new-bundle round-trip. Codex P2 on PR #910
     *  rounds 6 + 7. */
    initialFormState?: Partial<LaunchFormStateWire>;
    /** Optional callback fired when the user clicks the "+ New
     *  identity" button. Caller is expected to call
     *  tabModal.replace(newIdentityRequest) — the picker does this.
     *  The `current` snapshot carries the modal's live form state so
     *  the picker can preserve it across the new-bundle round-trip. */
    onRequestNewIdentity?: (current: LaunchFormStateWire) => void;
    /** Same for "+ New memory". */
    onRequestNewMemory?: (current: LaunchFormStateWire) => void;
}

/** Snapshot of the editable Launch form. Kept here (not imported from
 *  AgentLaunchModal) so tab-modal.ts stays a leaf type module with no
 *  imports into the view layer. */
export interface LaunchFormStateWire {
    name: string;
    runtime: "host" | "container";
    image: string;
    identityId: string;
    memoryId: string;
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

/**
 * Pre-launch system prerequisite check. Opened when one or more of
 * the provider's `systemPrereqs` are missing from PATH (e.g. Claude
 * Code requires `git` to start a session — anthropics/claude-code#29898).
 * See docs/specs/SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md.
 */
export interface AgentPrereqRequest {
    kind: "agent-prereqs";
    agent: ForgeAgent;
    originBlockId: string;
    /** Missing prereqs to display. Each row is rendered with its
     *  platform-appropriate install link. */
    missing: Array<{
        tool: string;
        label: string;
        installUrl: string;
        installLinkText: string;
    }>;
    /** User clicked "Refresh" — caller re-probes and either closes
     *  this modal (no missing) or replaces it with an updated list. */
    onRefresh: () => void;
    /** "Launch anyway" — proceed to install/launch despite the missing
     *  tool. Useful when the tool is at a non-standard PATH. */
    onProceed: () => void;
    /** "Cancel" — close the modal, do not launch. */
    onCancel: () => void;
}

/**
 * "+ New" affordance on the Launch modal's Identity row creates an
 * empty Identity bundle. Connector setup (Claude/Codex/GitHub/AWS)
 * happens in the Identity pane afterward.
 *
 * The actual UpsertIdentityBundle RPC is owned by the layer so its
 * `submitting()` flag (which gates safeClose) tracks the in-flight
 * call — see reagent P1 on PR #911. Callers only supply the chain
 * callbacks for after-success / on-cancel.
 *
 * Phase β of SPEC_LAUNCH_MODAL_PROFILE_SECTION_2026_05_18.md.
 */
export interface NewIdentityBundleRequest {
    kind: "new-identity";
    originBlockId: string;
    /** Initial value for the name field. Usually empty. */
    initialName?: string;
    /** Called after the bundle is persisted on disk. Caller should
     *  `tabModal.replace(launchRequest)` with the new id preselected;
     *  the layer does NOT close after this fires. */
    onCreated: (bundleId: string, bundleName: string) => void;
    /** Called when the user clicks Cancel. Caller should
     *  `tabModal.replace(launchRequest)` with the prior selection
     *  intact, OR `tabModal.close()` to exit. The layer does NOT
     *  close after this fires — running both replace + close
     *  synchronously nullified the replace, reagent P1 on PR #910. */
    onCancel: () => void;
}

/**
 * "+ New" affordance on the Launch modal's Memory row creates a new
 * Memory bundle with an optional pasted-text seed (saved as a single
 * `notes.md` context file).
 *
 * Same layer-owned-RPC contract as NewIdentityBundleRequest: the
 * UpsertMemory call lives in TabModalLayer's dispatch so the layer's
 * submitting() flag tracks the in-flight RPC; caller routes after
 * success/cancel via tabModal.replace or tabModal.close.
 *
 * Phase γ of SPEC_LAUNCH_MODAL_PROFILE_SECTION_2026_05_18.md.
 */
export interface NewMemoryBundleRequest {
    kind: "new-memory";
    originBlockId: string;
    initialName?: string;
    /** Caller should tabModal.replace(launchRequest) with the new id
     *  preselected. Layer does NOT close. */
    onCreated: (bundleId: string, bundleName: string) => void;
    /** Caller routes (replace vs close). Layer does NOT close. */
    onCancel: () => void;
}

/**
 * Browser-pane HTTP Basic / Digest auth challenge. Opened by the
 * BrowserViewModel when CEF fires `browser-pane-auth-required`.
 * Phase α of SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md.
 */
export interface BrowserAuthRequest {
    kind: "browser-auth";
    /** Pane that the request belongs to (used as the modal origin). */
    blockId: string;
    /** Opaque id correlating this prompt with the parked CEF callback
     *  on the host side. The submit/cancel IPC echoes it back. */
    requestId: string;
    /** Origin of the challenge (e.g. `https://pulse.asaf.cc`). Shown
     *  to the user so they know who's asking. */
    origin: string;
    /** Realm header value (e.g. `Restricted area`). Shown as the
     *  prompt subtitle. Empty when the server didn't supply one. */
    realm: string;
    /** True for HTTP 407 proxy auth; false for 401 server auth.
     *  v1 surfaces both with the same UI; future spec may diverge. */
    isProxy: boolean;
    /** User submitted credentials. The layer routes them to the host
     *  via `browser_pane_auth_submit`. */
    onSubmit: (username: string, password: string) => void;
    /** User cancelled. The layer routes to `browser_pane_auth_cancel`,
     *  which calls `AuthCallback.cancel()` so CEF aborts the request
     *  and renders the 401 response body. */
    onCancel: () => void;
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
