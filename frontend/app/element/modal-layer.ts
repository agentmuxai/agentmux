// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ModalLayer types + context — scope-neutral modal-host dispatcher.
 *
 * `useModalLayer()` resolves to the nearest `<ModalLayer>` in the tree;
 * pane-scoped layers override tab-scoped ones via normal context resolution.
 */

import { createContext, useContext, type Accessor } from "solid-js";

// ── Request shape ────────────────────────────────────────────────────────────
//
// Discriminated union so the layer can dispatch on `kind`. New modal
// surfaces add a variant here and a render branch in ModalLayer.

export type ModalLayerRequest =
    | LaunchAgentRequest
    | InstallAgentRequest
    | AgentPrereqRequest
    | NewIdentityBundleRequest
    | NewMemoryBundleRequest
    | CreateFromTemplateRequest
    | BrowserAuthRequest
    | AgentIdentityRequest
    | AgentMemoryRequest;

export interface LaunchAgentRequest {
    kind: "launch-agent";
    /** Agent definition the user clicked. */
    agent: AgentDefinition;
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
     *  across the new-bundle round-trip. */
    initialFormState?: Partial<LaunchFormStateWire>;
    /** When true, the launch modal fires its OAuth `startConnect()`
     *  exactly once on mount. Set by the OAuth-Connect → New Identity
     *  → launch round-trip (spec SPEC_BUNDLE_MANAGEMENT_2026_05_22.md
     *  §2): after the user names the bundle and clicks Continue, the
     *  launch modal re-opens with the new identity preselected and
     *  this hint set, so OAuth resumes automatically against the
     *  freshly-named bundle instead of waiting for a second click. */
    autoStartAuth?: boolean;
    /** Optional callback fired when the user wants to create a new
     *  identity bundle. Caller is expected to call
     *  modalLayer.replace(newIdentityRequest) — the picker does this.
     *  The `current` snapshot carries the modal's live form state so
     *  the picker can preserve it across the new-bundle round-trip.
     *
     *  `purpose` distinguishes the two entry points:
     *   - `"create"` (the plain `+ New identity` button): create a
     *     named bundle and return to the launch form.
     *   - `"oauth-continue"` (the OAuth Connect `needs-bundle` click):
     *     create a named bundle, return to the launch form with the
     *     new id preselected AND `autoStartAuth` set so OAuth resumes
     *     automatically against the named bundle.
     *  Spec SPEC_BUNDLE_MANAGEMENT_2026_05_22.md §2. */
    onRequestNewIdentity?: (
        current: LaunchFormStateWire,
        purpose: "create" | "oauth-continue",
    ) => void;
    /** Same for "+ New memory". */
    onRequestNewMemory?: (current: LaunchFormStateWire) => void;
}

/** Snapshot of the editable Launch form. Kept here (not imported from
 *  AgentLaunchModal) so modal-layer.ts stays a leaf type module with
 *  no imports into the view layer. */
export interface LaunchFormStateWire {
    name: string;
    runtime: "host" | "container";
    image: string;
    identityId: string;
    memoryId: string;
    /** Continuation context — the `continueOfId` from the Continue
     *  dropdown. `null` = "— New agent —". Threaded through the
     *  `+ New bundle` round-trip so the re-opened launch modal restores
     *  Continue mode (otherwise an ambient-creds continuation drops out
     *  of Continue and the auth gate wrongly re-engages — see
     *  docs/analysis/LAUNCH_MODAL_CONTINUE_LOST_2026_05_22.md). */
    continueOfId: string | null;
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
    agent: AgentDefinition;
    originBlockId: string;
    /**
     * Called by the modal once the install completes successfully. The
     * boolean reflects which terminal button the user clicked:
     *  - `true`  — "Continue to Launch": flip install state AND open
     *    the launch modal as the natural next step.
     *  - `false` — "Close": flip install state but do not chain.
     *
     * Callers MUST flip cached install state in both branches — the Close
     * path skipping the flip strands users on a stale ribbon.
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
    agent: AgentDefinition;
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
 * call. Callers only supply the chain callbacks for after-success / on-cancel.
 *
 * Phase β of SPEC_LAUNCH_MODAL_PROFILE_SECTION_2026_05_18.md.
 */
export interface NewIdentityBundleRequest {
    kind: "new-identity";
    originBlockId: string;
    /** Initial value for the name field. Usually empty. */
    initialName?: string;
    /** Why the modal opened — controls the primary button label only
     *  ("Create" vs "Continue"). `"create"` (default) is the plain
     *  `+ New identity` button; `"oauth-continue"` is the OAuth-Connect
     *  (`needs-bundle`) interposition. Spec
     *  SPEC_BUNDLE_MANAGEMENT_2026_05_22.md §2. */
    purpose?: "create" | "oauth-continue";
    /** Called after the bundle is persisted on disk. Caller should
     *  `modalLayer.replace(launchRequest)` with the new id preselected;
     *  the layer does NOT close after this fires. */
    onCreated: (bundleId: string, bundleName: string) => void;
    /** Called when the user clicks Cancel. Caller should
     *  `modalLayer.replace(launchRequest)` with the prior selection
     *  intact, OR `modalLayer.close()` to exit. The layer does NOT
     *  close after this fires — running both replace + close
     *  synchronously nullified the replace. */
    onCancel: () => void;
}

/**
 * "+ New" affordance on the Launch modal's Memory row creates a new
 * Memory bundle with an optional pasted-text seed (saved as a single
 * `notes.md` context file).
 *
 * Same layer-owned-RPC contract as NewIdentityBundleRequest: the
 * UpsertMemory call lives in ModalLayer's dispatch so the layer's
 * submitting() flag tracks the in-flight RPC; caller routes after
 * success/cancel via modalLayer.replace or modalLayer.close.
 *
 * Phase γ of SPEC_LAUNCH_MODAL_PROFILE_SECTION_2026_05_18.md.
 */
export interface NewMemoryBundleRequest {
    kind: "new-memory";
    originBlockId: string;
    initialName?: string;
    /** Caller should modalLayer.replace(launchRequest) with the new id
     *  preselected. Layer does NOT close. */
    onCreated: (bundleId: string, bundleName: string) => void;
    /** Caller routes (replace vs close). Layer does NOT close. */
    onCancel: () => void;
}

/**
 * Two-tier picker — Phase 1 (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md).
 * Opens when the user clicks a card in the picker's Templates section.
 * On submit the layer chains:
 *  1. `agentdefcreatefromtemplate` to clone the template into a new
 *     user-owned definition,
 *  2. `launchAgentDefinition` with the picked bindings to spawn the
 *     new agent immediately.
 *
 * Why the layer owns the chain (not the picker): the modal's
 * `submitting()` gate must track BOTH the create RPC and the launch
 * await so ESC + backdrop dismiss are blocked across the whole flow.
 * Splitting them would leave a window where the create succeeded but
 * the launch hadn't fired, and ESC would lose the user's just-created
 * agent without launching it.
 */
export interface CreateFromTemplateRequest {
    kind: "create-from-template";
    /** Seeded template the user clicked. Its `is_seeded` field MUST be
     *  1; the backend rejects non-template ids defensively. */
    template: AgentDefinition;
    /** Block id of the pane that opened the modal. */
    originBlockId: string;
    /** Called by the layer after `agentdefcreatefromtemplate` returns.
     *  The picker uses this to fire `launchAgentDefinition` with the
     *  freshly-minted user-agent definition. Returns a promise so the
     *  layer can keep `submitting()` true across the launch await. */
    onCreatedAndLaunch: (
        newDefinitionId: string,
        identityId: string,
        memoryId: string,
        name: string,
        /** Runtime the user picked in the modal. The launch override
         *  uses this directly instead of re-reading the (template-
         *  derived) `agent_type`, so the choice actually takes effect. */
        agentType: "host" | "container",
    ) => Promise<void>;
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

/**
 * Agent pane identity modal — opened by the id-card icon in the agent
 * pane header. Replaces the cog → Identity tab flow with a pane-scoped
 * modal. Spec: SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md.
 */
export interface AgentIdentityRequest {
    kind: "agent-identity";
    /** Agent definition to show/edit accounts for. */
    agent: AgentDefinition;
    /** Block id of the pane — used to construct the IdentityViewModel. */
    blockId: string;
}

/**
 * Agent pane memory modal — opened by the brain icon in the agent
 * pane header. Shows the native memory folder for the agent.
 * Phase 1: placeholder UI. Phase 3 adds the full file browser + editor.
 * Spec: SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md.
 */
export interface AgentMemoryRequest {
    kind: "agent-memory";
    agentId: string;
    agentName: string;
    /** Agent's working directory — used to compute the memory folder path. */
    workingDirectory: string;
}

// ── Context API ──────────────────────────────────────────────────────────────

export interface ModalLayerApi {
    /** Open or replace the current modal request. */
    open: (req: ModalLayerRequest) => void;
    /**
     * Replace the current modal with `next` as a continuation of the
     * same flow. The backdrop + outer panel stay mounted across the
     * swap; only the panel content remounts with a content-fade
     * animation. Falls back to `open(next)` when no modal is open.
     *
     * See docs/specs/SPEC_MODAL_TRANSITIONS_2026_05_18.md.
     */
    replace: (next: ModalLayerRequest) => void;
    /** Close the current modal, if any. No-op when nothing is open. */
    close: () => void;
    /** The currently open request, or null. */
    current: Accessor<ModalLayerRequest | null>;
}

export const ModalLayerContext = createContext<ModalLayerApi | null>(null);

/**
 * Access the nearest modal-layer API in the tree. Pane-scoped layers
 * override tab-scoped ones via normal context resolution — call sites
 * don't need to know whether they're inside a tab layer or a pane
 * layer, the modal just opens at whichever scope encloses them.
 *
 * Throws when called outside any layer so misuses surface immediately
 * rather than silently no-op'ing.
 */
export function useModalLayer(): ModalLayerApi {
    const ctx = useContext(ModalLayerContext);
    if (!ctx) {
        throw new Error("useModalLayer called outside <ModalLayer>");
    }
    return ctx;
}
