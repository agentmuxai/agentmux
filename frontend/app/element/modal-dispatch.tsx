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
import { AgentNewIdentityModalPanel } from "@/app/view/agent/components/AgentNewIdentityModal";
import { AgentNewMemoryModalPanel } from "@/app/view/agent/components/AgentNewMemoryModal";
import { AgentCreateFromTemplateModalPanel } from "@/app/view/agent/components/AgentCreateFromTemplateModal";
import { BrowserAuthModalPanel } from "@/app/view/browser/components/BrowserAuthModal";
import "@/app/view/agent/components/AgentPrereqModal.scss";
import "@/app/view/agent/components/AgentNewBundleModal.scss";
import "@/app/view/browser/components/BrowserAuthModal.scss";

import type { ModalLayerApi, ModalLayerRequest } from "./modal-layer";

export function requestLabel(req: ModalLayerRequest): string {
    switch (req.kind) {
        case "launch-agent":
            return `Launch ${req.agent.name}`;
        case "new-identity":
            return "New Identity";
        case "new-memory":
            return "New Memory";
        case "agent-prereqs":
            return `Install required tools for ${req.agent.name}`;
        case "install-agent":
            return `Install ${req.agent.name}`;
        case "create-from-template":
            return `Create new agent from ${req.template.name}`;
        case "browser-auth":
            return req.isProxy ? "Proxy authentication required" : "Authentication required";
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
                        autoStartAuth={req.autoStartAuth}
                        onRequestNewIdentity={req.onRequestNewIdentity}
                        onRequestNewMemory={req.onRequestNewMemory}
                    />
                ),
            };
        case "new-identity":
            return {
                label: requestLabel(req),
                panel: (
                    <AgentNewIdentityModalPanel
                        initialName={req.initialName}
                        purpose={req.purpose}
                        // The layer owns the RPC + chaining so its
                        // `submitting()` flag (which gates safeClose)
                        // tracks the in-flight call. Mirrors the
                        // launch-agent dispatch above — reagent P1 on
                        // PR #911.
                        onSubmit={async ({ name, description }) => {
                            setSubmitting(true);
                            try {
                                // Wire convention from identity-pane-
                                // model.ts:bundleDraftToWire — empty id
                                // triggers server-side uuid; 0 timestamps
                                // trigger server-side now-stamping. Keeps
                                // id/timestamp handling in one place
                                // (codex P2 on PR #910 round 3).
                                const bundle = await RpcApi.UpsertIdentityBundleCommand(
                                    TabRpcClient,
                                    {
                                        id: "",
                                        name,
                                        description,
                                        is_blank: false,
                                        created_at: 0,
                                        updated_at: 0,
                                    },
                                );
                                setSubmitting(false);
                                // Caller's onCreated does modalLayer.replace
                                // back to Launch with the new id
                                // preselected — that's what unmounts this
                                // panel. We don't `api.close()` here.
                                req.onCreated(bundle.id, bundle.name);
                            } catch (e) {
                                setSubmitting(false);
                                throw e;
                            }
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
                        onSubmit={async ({ name, identityId, memoryId, agentType }) => {
                            setSubmitting(true);
                            try {
                                const resp = await RpcApi.AgentDefCreateFromTemplateCommand(
                                    TabRpcClient,
                                    {
                                        template_id: req.template.id,
                                        name,
                                        identity_id: identityId,
                                        memory_id: memoryId,
                                        // Persist the chosen runtime on the
                                        // new user-owned definition so later
                                        // reattach/auto-continue uses it too.
                                        agent_type: agentType,
                                    },
                                );
                                await req.onCreatedAndLaunch(
                                    resp.definition_id,
                                    resp.identity_id,
                                    resp.memory_id,
                                    name,
                                    agentType,
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
                        onSubmit={(username, password) => {
                            req.onSubmit(username, password);
                            api.close();
                        }}
                    />
                ),
            };
    }
}
