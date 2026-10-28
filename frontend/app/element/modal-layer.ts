// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ModalLayer types + context — scope-neutral modal-host dispatcher.
 *
 * `useModalLayer()` resolves to the nearest `<ModalLayer>` in the tree;
 * pane-scoped layers override tab-scoped ones via normal context resolution.
 */

import { createContext, useContext, type Accessor } from "solid-js";
import type { BundleImportCommitResponse, BundleImportPreviewResponse } from "@/app/store/rpc-api";

// ── Request shape ────────────────────────────────────────────────────────────
//
// Discriminated union so the layer can dispatch on `kind`. New modal
// surfaces add a variant here and a render branch in ModalLayer.

export type ModalLayerRequest =
    | BrowserAuthRequest
    | AgentSeatRequest
    | AgentEnvRequest
    | AgentBundlesRequest
    | BundleImportSelectRequest
    | BundleImportPreviewRequest
    | BundleImportConfirmRequest;

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
    /** Origin of the challenge (e.g. `https://protected.example.com`). Shown
     *  to the user so they know who's asking. */
    origin: string;
    /** Realm header value (e.g. `Restricted area`). Shown as the
     *  prompt subtitle. Empty when the server didn't supply one. */
    realm: string;
    /** True for HTTP 407 proxy auth; false for 401 server auth.
     *  v1 surfaces both with the same UI; future spec may diverge. */
    isProxy: boolean;
    /** User submitted credentials. The layer routes them to the host
     *  via `browser_pane_auth_submit`, and — when `save` is true (the
     *  panel's opt-in "save this credential" checkbox) — also fires
     *  `browser_pane_auth_save` so the host stores it in the OS keychain
     *  for future auto-fill. */
    onSubmit: (username: string, password: string, save: boolean) => void;
    /** User cancelled. The layer routes to `browser_pane_auth_cancel`,
     *  which calls `AuthCallback.cancel()` so CEF aborts the request
     *  and renders the 401 response body. */
    onCancel: () => void;
}

/**
 * An agent's seat (role, lead, area), opened by "Seat…" in My Agents' row
 * menu and the agent pane's right-click menu (AgentSeatPanel).
 * SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3.
 */
export interface AgentSeatRequest {
    kind: "agent-seat";
    agentId: string;
    agentName: string;
}

/**
 * An agent's extra environment variables, opened by "Environment…" in My
 * Agents' row menu (AgentEnvPanel).
 */
export interface AgentEnvRequest {
    kind: "agent-env";
    agentId: string;
    agentName: string;
}

/**
 * An existing agent's ordered bundle list, opened by "Bundles…" in My Agents'
 * row menu and the agent pane's right-click menu (AgentBundlesTab).
 */
export interface AgentBundlesRequest {
    kind: "agent-bundles";
    agentId: string;
    agentName: string;
}

/**
 * Armory Bundle Format (ABF) import — Phase 3
 * (docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md §4). Three chained
 * modal-layer requests, following the same `ModalLayerApi.replace()`
 * sequential pattern as the install→launch chain above — this codebase has
 * no wizard-panel component, and threading step state through this
 * request-kind union keeps each step's props typed and testable
 * independently.
 */

/** Step 1 — pick a `.abf` file and preview it. Has almost no UI of its own:
 *  it triggers the native file dialog on mount, shows a spinner while
 *  `bundle.import.preview` runs, and surfaces a parse/validation error
 *  inline (with a retry) rather than advancing. */
interface BundleImportSelectRequest {
    kind: "bundle-import-select";
    /** Preview succeeded — caller does
     *  `modalLayer.replace(previewRequest)`. */
    onPreviewed: (filePath: string, preview: BundleImportPreviewResponse) => void;
    onCancel: () => void;
}

/** One skill row's selection state — every parsed skill gets an entry,
 *  including non-colliding ones (checked by default). For a colliding
 *  row, `renameValue` starts empty (§4.1 point 5) and is the ONLY value
 *  ever submitted as `import_as`; leaving it empty at commit time is
 *  treated as skip regardless of the checkbox (§4.1 point 4). */
export interface BundleImportSkillSelectionState {
    sourceDir: string;
    checked: boolean;
    renameValue: string;
}

/** Built by Step 2, consumed by Step 3 — the user's full selection,
 *  independent of the (possibly truncated) preview display values. */
export interface BundleImportSelectionState {
    bundleName: string;
    includeInstructions: boolean;
    includeContextFileIds: number[];
    skills: BundleImportSkillSelectionState[];
    /** The bundle's widgets to install, by id; each asks for approval. */
    includeWidgets?: string[];
}

/** Step 2 — preview & select. Renders the checklist described in §4 Step
 *  2: editable bundle name, instructions checkbox + preview, context
 *  files, skills (with collision UI per §4.1), MCP servers, a read-only
 *  requirements summary, and a dismissible warnings banner. */
interface BundleImportPreviewRequest {
    kind: "bundle-import-preview";
    filePath: string;
    preview: BundleImportPreviewResponse;
    /** "Next" — caller does
     *  `modalLayer.replace(confirmRequest)`. */
    onNext: (selection: BundleImportSelectionState) => void;
    onCancel: () => void;
}

/** Step 3 — confirm & import. Calls `bundle.import.commit`; a digest
 *  mismatch (the file changed since preview) surfaces as a distinct
 *  "re-select and preview again" error rather than a generic failure. */
interface BundleImportConfirmRequest {
    kind: "bundle-import-confirm";
    filePath: string;
    contentDigest: string;
    bundleDisplayName: string;
    selection: BundleImportSelectionState;
    /** Commit succeeded — caller does `modalLayer.close()` (+ any
     *  post-import navigation). */
    onImported: (result: BundleImportCommitResponse) => void;
    onCancel: () => void;
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
