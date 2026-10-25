// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Modal dispatch helpers — concrete modal panel imports + label/render logic.
// To add a new modal kind: add a variant to ModalLayerRequest in modal-layer.ts,
// add a case here, and implement the panel component. ModalLayer.tsx never needs
// to change.

import type { JSX } from "solid-js";
import { BrowserAuthModalPanel } from "@/app/view/browser/components/BrowserAuthModal";
import { AgentBundlesTab } from "@/app/view/agent/components/AgentBundlesTab";
import { AgentEnvPanel } from "@/app/view/agent/components/AgentEnvPanel";
import { AgentIdentityModalPanel } from "@/app/view/agent/components/AgentIdentityModal";
import { AgentNativeMemoryModal } from "@/app/view/agent/components/AgentNativeMemoryModal";
import { AgentSeatPanel } from "@/app/view/agent/components/AgentSeatPanel";
import { BundleImportSelectModalPanel } from "@/app/view/bundle/components/BundleImportSelectModal";
import { BundleImportPreviewModalPanel } from "@/app/view/bundle/components/BundleImportPreviewModal";
import { BundleImportConfirmModalPanel } from "@/app/view/bundle/components/BundleImportConfirmModal";
import "@/app/view/agent/components/AgentIdentityModal.scss";
import "@/app/view/agent/components/AgentStashModal.scss";
import "@/app/view/browser/components/BrowserAuthModal.scss";
import "@/app/view/bundle/components/BundleImportModal.scss";

import type { ModalLayerApi, ModalLayerRequest } from "./modal-layer";

export function requestLabel(req: ModalLayerRequest): string {
    switch (req.kind) {
        case "browser-auth":
            return req.isProxy ? "Proxy authentication required" : "Authentication required";
        case "agent-identity":
            return `Identity — ${req.agent.name}`;
        case "agent-memory":
            return `Memory — ${req.agentName}`;
        case "agent-seat":
            return `Seat — ${req.agentName}`;
        case "agent-env":
            return `Environment — ${req.agentName}`;
        case "agent-bundles":
            return `Bundles — ${req.agentName}`;
        case "bundle-import-select":
            return "Import Bundle";
        case "bundle-import-preview":
            return "Preview & Select";
        case "bundle-import-confirm":
            return "Confirm Import";
    }
}

export function renderRequest(
    req: ModalLayerRequest,
    api: ModalLayerApi,
    setSubmitting: (v: boolean) => void,
): { label: string; panel: JSX.Element } {
    switch (req.kind) {
        case "browser-auth":
            return {
                label: requestLabel(req),
                panel: (
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
            };
        case "agent-identity":
            return {
                label: requestLabel(req),
                panel: (
                    <AgentIdentityModalPanel
                        agent={req.agent}
                        blockId={req.blockId}
                        onClose={api.close}
                    />
                ),
            };
        case "agent-memory":
            return {
                label: requestLabel(req),
                panel: (
                    <AgentNativeMemoryModal
                        agentId={req.agentId}
                        agentName={req.agentName}
                        workingDirectory={req.workingDirectory}
                        onClose={api.close}
                    />
                ),
            };
        case "agent-seat":
            return {
                label: requestLabel(req),
                panel: <AgentSeatPanel agentId={req.agentId} agentName={req.agentName} onClose={api.close} />,
            };
        case "agent-env":
            return {
                label: requestLabel(req),
                panel: <AgentEnvPanel agentId={req.agentId} agentName={req.agentName} onClose={api.close} />,
            };
        case "agent-bundles":
            return {
                label: requestLabel(req),
                panel: <AgentBundlesTab agentId={req.agentId} onClose={api.close} />,
            };
        case "bundle-import-select":
            return {
                label: requestLabel(req),
                panel: (
                    <BundleImportSelectModalPanel
                        onPreviewed={req.onPreviewed}
                        onCancel={req.onCancel}
                    />
                ),
            };
        case "bundle-import-preview":
            return {
                label: requestLabel(req),
                panel: (
                    <BundleImportPreviewModalPanel
                        preview={req.preview}
                        onNext={req.onNext}
                        onCancel={req.onCancel}
                    />
                ),
            };
        case "bundle-import-confirm":
            return {
                label: requestLabel(req),
                panel: (
                    <BundleImportConfirmModalPanel
                        filePath={req.filePath}
                        contentDigest={req.contentDigest}
                        bundleDisplayName={req.bundleDisplayName}
                        selection={req.selection}
                        onImported={req.onImported}
                        onCancel={req.onCancel}
                    />
                ),
            };
    }
}
