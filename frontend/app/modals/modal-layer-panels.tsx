// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The modal kinds ModalLayer can show, and the panel each one renders. Registered here, not in
// element/modal-dispatch.tsx, so the element layer imports no view (scripts/RATCHETS.md).
// Loaded for its side effect by app-init.ts. Every kind of ModalLayerRequest (element/modal-layer.ts)
// has an entry below.

import { registerModalKind } from "@/app/element/modal-dispatch";
import { AgentBundlesTab } from "@/app/view/agent/components/AgentBundlesTab";
import { AgentContainerLimitsPanel } from "@/app/view/agent/components/AgentContainerLimitsPanel";
import { AgentEnvPanel } from "@/app/view/agent/components/AgentEnvPanel";
import { AgentSeatPanel } from "@/app/view/agent/components/AgentSeatPanel";
import { BrowserAuthModalPanel } from "@/app/view/browser/components/BrowserAuthModal";
import { BundleImportConfirmModalPanel } from "@/app/view/bundle/components/BundleImportConfirmModal";
import { BundleImportPreviewModalPanel } from "@/app/view/bundle/components/BundleImportPreviewModal";
import { BundleImportSelectModalPanel } from "@/app/view/bundle/components/BundleImportSelectModal";
import "@/app/view/browser/components/BrowserAuthModal.scss";
import "@/app/view/bundle/components/BundleImportModal.scss";

registerModalKind("browser-auth", {
    label: (req) => (req.isProxy ? "Proxy authentication required" : "Authentication required"),
    panel: (req, api) => (
        <BrowserAuthModalPanel
            origin={req.origin}
            realm={req.realm}
            isProxy={req.isProxy}
            onCancel={() => {
                req.onCancel();
                api.close();
            }}
            onSubmit={(username, password, save) => {
                req.onSubmit(username, password, save);
                api.close();
            }}
        />
    ),
});

registerModalKind("agent-seat", {
    label: (req) => `Seat — ${req.agentName}`,
    panel: (req, api) => <AgentSeatPanel agentId={req.agentId} agentName={req.agentName} onClose={api.close} />,
});

registerModalKind("agent-env", {
    label: (req) => `Environment — ${req.agentName}`,
    panel: (req, api) => <AgentEnvPanel agentId={req.agentId} agentName={req.agentName} onClose={api.close} />,
});

registerModalKind("agent-container-limits", {
    label: (req) => `Container limits — ${req.agentName}`,
    panel: (req, api) => <AgentContainerLimitsPanel agentId={req.agentId} agentName={req.agentName} onClose={api.close} />,
});

registerModalKind("agent-bundles", {
    label: (req) => `Bundles — ${req.agentName}`,
    panel: (req, api) => <AgentBundlesTab agentId={req.agentId} onClose={api.close} />,
});

registerModalKind("bundle-import-select", {
    label: () => "Import Bundle",
    panel: (req) => <BundleImportSelectModalPanel onPreviewed={req.onPreviewed} onCancel={req.onCancel} />,
});

registerModalKind("bundle-import-preview", {
    label: () => "Preview & Select",
    panel: (req) => <BundleImportPreviewModalPanel preview={req.preview} onNext={req.onNext} onCancel={req.onCancel} />,
});

registerModalKind("bundle-import-confirm", {
    label: () => "Confirm Import",
    panel: (req) => (
        <BundleImportConfirmModalPanel
            filePath={req.filePath}
            contentDigest={req.contentDigest}
            bundleDisplayName={req.bundleDisplayName}
            selection={req.selection}
            onImported={req.onImported}
            onCancel={req.onCancel}
        />
    ),
});
