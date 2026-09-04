// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Modal dispatch helpers — concrete modal panel imports + label/render logic.
// To add a new modal kind: add a variant to ModalLayerRequest in modal-layer.ts,
// add a case here, and implement the panel component. ModalLayer.tsx never needs
// to change.

import type { JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { AgentLaunchModalPanel } from "@/app/view/agent/components/AgentLaunchModal";
import { AgentInstallModalPanel } from "@/app/view/agent/components/AgentInstallModal";
import { AgentPrereqModalPanel } from "@/app/view/agent/components/AgentPrereqModal";
import { AgentAddAccountModalPanel } from "@/app/view/agent/components/AgentNewIdentityModal";
import { AgentNewMemoryModalPanel } from "@/app/view/agent/components/AgentNewMemoryModal";
import { AgentCreateFromTemplateModalPanel } from "@/app/view/agent/components/AgentCreateFromTemplateModal";
import { BrowserAuthModalPanel } from "@/app/view/browser/components/BrowserAuthModal";
import { AgentIdentityModalPanel } from "@/app/view/agent/components/AgentIdentityModal";
import { AgentNativeMemoryModal } from "@/app/view/agent/components/AgentNativeMemoryModal";
import { AgentStashModal } from "@/app/view/agent/components/AgentStashModal";
import { BundleImportSelectModalPanel } from "@/app/view/memory/components/BundleImportSelectModal";
import { BundleImportPreviewModalPanel } from "@/app/view/memory/components/BundleImportPreviewModal";
import { BundleImportConfirmModalPanel } from "@/app/view/memory/components/BundleImportConfirmModal";
import "@/app/view/agent/components/AgentPrereqModal.scss";
import "@/app/view/agent/components/AgentNewBundleModal.scss";
import "@/app/view/agent/components/AgentIdentityModal.scss";
import "@/app/view/agent/components/AgentStashModal.scss";
import "@/app/view/browser/components/BrowserAuthModal.scss";
import "@/app/view/memory/components/BundleImportModal.scss";

import type { ModalLayerApi, ModalLayerRequest } from "./modal-layer";

export function requestLabel(req: ModalLayerRequest): string {
    switch (req.kind) {
        case "launch-agent":
            return `Launch ${req.agent.name}`;
        case "add-account":
            return "Add Account";
        case "new-memory":
            return "New Bundle";
        case "agent-prereqs":
            return `Install required tools for ${req.agent.name}`;
        case "install-agent":
            return `Install ${req.agent.name}`;
        case "create-from-template":
            return `Create new agent from ${req.template.name}`;
        case "browser-auth":
            return req.isProxy ? "Proxy authentication required" : "Authentication required";
        case "agent-identity":
            return `Identity — ${req.agent.name}`;
        case "agent-memory":
            return `Memory — ${req.agentName}`;
        case "agent-stash":
            return "Stash";
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
        case "launch-agent":
            return {
                label: requestLabel(req),
                panel: (
                    <AgentLaunchModalPanel
                        agent={req.agent}
                        onCancel={api.close}
                        onSubmit={async (overrides) => {
                            setSubmitting(true);
                            try {
                                await req.onSubmit(overrides);
                                setSubmitting(false);
                                api.close();
                            } catch (e) {
                                setSubmitting(false);
                                throw e;
                            }
                        }}
                        initialFormState={req.initialFormState}
                        onRequestAddAccount={req.onRequestAddAccount}
                        onRequestNewMemory={req.onRequestNewMemory}
                    />
                ),
            };
        case "add-account":
            return {
                label: requestLabel(req),
                panel: (
                    <AgentAddAccountModalPanel
                        provider={req.provider}
                        initialName={req.initialName}
                        // Unlike the old bundle flow, the RPC
                        // (`account.key.verify`) lives inside the panel
                        // itself — it needs to run the live-probe
                        // before knowing the resulting account id, and
                        // the panel already tracks its own submitting
                        // state for that. This callback just forwards
                        // the result to the caller's chain.
                        onSubmit={async ({ accountId }) => {
                            // Caller's onCreated does modalLayer.replace
                            // back to Launch with the new id
                            // preselected — that's what unmounts this
                            // panel. We don't `api.close()` here.
                            req.onCreated(accountId);
                        }}
                        // Caller's onCancel does modalLayer.replace back to
                        // Launch with the prior selection intact. Running
                        // api.close() afterward would nullify that replace
                        // (both run synchronously, last write wins) and
                        // exit the launch flow — reagent P1 on PR #910.
                        onCancel={req.onCancel}
                    />
                ),
            };
        case "new-memory":
            return {
                label: requestLabel(req),
                panel: (
                    <AgentNewMemoryModalPanel
                        initialName={req.initialName}
                        // Same lift-up pattern as new-identity above —
                        // layer owns the UpsertMemory RPC so its
                        // submitting() flag (gates safeClose) tracks
                        // the in-flight call.
                        onSubmit={async ({ name, description, contextFiles }) => {
                            setSubmitting(true);
                            try {
                                const memory = await RpcApi.UpsertMemoryCommand(
                                    TabRpcClient,
                                    {
                                        // Wire convention from
                                        // memory-model.ts:draftToWire —
                                        // empty id triggers server-side
                                        // uuid; 0 timestamps trigger
                                        // server-side now-stamping.
                                        id: "",
                                        name,
                                        description,
                                        provider: "",
                                        model: "",
                                        instructions: "",
                                        context_files: contextFiles,
                                        mcp_servers: "[]",
                                        skills: "[]",
                                        created_at: 0,
                                        updated_at: 0,
                                    },
                                );
                                setSubmitting(false);
                                req.onCreated(memory.id, memory.name);
                            } catch (e) {
                                setSubmitting(false);
                                throw e;
                            }
                        }}
                        onCancel={req.onCancel}
                    />
                ),
            };
        case "agent-prereqs":
            return {
                label: requestLabel(req),
                panel: (
                    <AgentPrereqModalPanel
                        agent={req.agent}
                        missing={req.missing}
                        installedPendingRestart={req.installedPendingRestart}
                        onToolInstalled={(tool) => req.onToolInstalled(tool)}
                        onRefresh={() => req.onRefresh()}
                        onProceed={() => req.onProceed()}
                        onCancel={() => {
                            req.onCancel();
                            api.close();
                        }}
                    />
                ),
            };
        case "install-agent":
            return {
                label: requestLabel(req),
                panel: (
                    <AgentInstallModalPanel
                        agent={req.agent}
                        onCancel={api.close}
                        onInstalled={(continueToLaunch: boolean) => {
                            // Hand off to the picker — it owns whether
                            // to call `modalLayer.replace(launchReq)`
                            // (continueToLaunch=true) or `modalLayer.close()`
                            // (continueToLaunch=false). Don't tear down
                            // the shell here — that would break the
                            // install→launch crossfade for the chain
                            // path. SPEC_MODAL_TRANSITIONS_2026_05_18.md.
                            req.onInstalled(continueToLaunch);
                        }}
                    />
                ),
            };
        case "create-from-template":
            return {
                label: requestLabel(req),
                panel: (
                    <AgentCreateFromTemplateModalPanel
                        template={req.template}
                        // The layer owns the create-then-launch chain
                        // (spec note on CreateFromTemplateRequest) so
                        // `submitting()` covers both RPC steps and ESC
                        // / backdrop dismiss stay blocked end-to-end.
                        onSubmit={async ({ name, accountId, memoryId, agentType, modelVendorBaseUrl, model }) => {
                            setSubmitting(true);
                            try {
                                const resp = await RpcApi.AgentDefCreateFromTemplateCommand(
                                    TabRpcClient,
                                    {
                                        template_id: req.template.id,
                                        name,
                                        identity_id: accountId,
                                        memory_id: memoryId,
                                        // Persist the chosen runtime on the
                                        // new user-owned definition so later
                                        // reattach/auto-continue uses it too.
                                        agent_type: agentType,
                                        // Always sent explicitly (never
                                        // omitted) — the form always has a
                                        // concrete value in scope (defaults
                                        // to the template's own), so there's
                                        // no "leave untouched" case to
                                        // distinguish here.
                                        model_vendor_base_url: modelVendorBaseUrl,
                                    },
                                );
                                await req.onCreatedAndLaunch(
                                    resp.definition_id,
                                    resp.identity_id,
                                    resp.memory_id,
                                    name,
                                    agentType,
                                    model,
                                );
                                setSubmitting(false);
                                api.close();
                            } catch (e) {
                                setSubmitting(false);
                                throw e;
                            }
                        }}
                        onCancel={api.close}
                    />
                ),
            };
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
        case "agent-stash":
            return {
                label: requestLabel(req),
                panel: (
                    <AgentStashModal
                        agentId={req.agentId}
                        agentName={req.agentName}
                        workingDirectory={req.workingDirectory}
                        initialTab={req.initialTab}
                        onClose={api.close}
                    />
                ),
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
