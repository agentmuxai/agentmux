// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Formerly IdentityManager — the full-CRUD Identity-bundle management
// UI (list / create / edit / delete / per-provider binding). Deleted
// as dead code (issue #1624 PR-C follow-up): its two intended mount
// points never had a live caller —
//
//   1. The `view: "identity"` agent-settings pane demoted to a
//      read-only `<BundleSummaryPanel/>` back in
//      specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md PR 5.
//   2. The "hamburger Identity & Memory manager" (`BundleManagerModal`)
//      referenced in that pane's comment was never actually built —
//      the Armory pane shipped instead, and its "Identities" tab
//      mounts the read-only `<AgentIdentitiesPanel/>`
//      (`agent-identities-panel.tsx`), not this file.
//
// `statusBadge()` is the one export `<AgentIdentitiesPanel/>` still
// reuses, so it stays.

/**
 * Map an oauth-class binding's account status to the small status-badge
 * descriptor rendered in the bindings table. Per spec §4.4. The
 * fallback (`label = status string verbatim, dot = "unknown"`) keeps
 * api-key rows whose `status` is a freeform legacy string visible
 * without forcing them through this dispatch.
 */
export function statusBadge(status: string | undefined): {
    label: string;
    dot: "valid" | "expired" | "needs_reauth" | "unknown";
    reconnect: boolean;
} {
    switch (status) {
        case "valid":
            return { label: "Valid", dot: "valid", reconnect: false };
        case "expired":
            return { label: "Expired", dot: "expired", reconnect: false };
        case "needs_reauth":
            return { label: "Reconnect needed", dot: "needs_reauth", reconnect: true };
        case "unknown":
        case undefined:
        case "":
            return { label: "—", dot: "unknown", reconnect: false };
        default:
            // api-key freeform strings (e.g. "ok", "invalid", "checking")
            // render verbatim with the neutral dot so legacy rows stay
            // readable.
            return { label: status, dot: "unknown", reconnect: false };
    }
}
