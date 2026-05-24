// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TabModalLayer — per-tab modal host.
 *
 * Stage 3 of SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21. This layer keeps its
 * imperative request API (`TabModalApi`: open / replace / close /
 * current) exactly as callers know it, but no longer hand-rolls the
 * overlay / backdrop / panel DOM. Instead it:
 *
 *  1. Renders a real mount node (`.tab-modal-mount`) inside `TabContent`'s
 *     `position:relative` root. `TabContent`'s tile layout (`props.children`)
 *     is a child of that mount node, and a `scope="tab"` `<Modal>` portals
 *     into the *same* node — becoming a sibling of the tile layout.
 *  2. Wraps everything in a `<TabModalScope.Provider>` whose value is an
 *     accessor for that mount node, so the descendant `<Modal scope="tab">`
 *     resolves its tab via `useContext(TabModalScope)`.
 *  3. Delegates all rendering — backdrop, panel chrome, ESC, focus trap,
 *     scope-relative `inert` + scroll lock, pane-overlay clip — to the
 *     unified `<Modal>`. The dispatched agent panel (`renderRequest().panel`)
 *     is the modal's children.
 *
 * The unified `<Modal>`'s scope-relative `inert` (spec §5) handles what
 * the old hand-rolled `inert` wrapper did: when the modal is open the
 * mount node's non-`.modal-root` children (the tile layout) are inerted,
 * trapping focus inside the panel while the tab bar + other tabs stay
 * live. Switching tabs still hides the modal via the existing
 * `display:none` on inactive tab content.
 *
 * Dismissal (spec §9): `closeOnBackdropClick={false}` folds in the old
 * no-backdrop-dismiss behaviour — a backdrop click nudges the panel's
 * `[data-modal-dismiss]` Cancel/Close control instead of closing. ESC is
 * the unified `<Modal>`'s. The submit-in-flight guard still lives here:
 * `safeClose` no-ops while `submitting()`, so ESC routed through
 * `onClose` is swallowed mid-RPC.
 *
 * See docs/specs/launch-modal-rearchitecture-2026-05-01.md (superseded)
 * and docs/specs/SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md §3/§5/§7/§9/§11.
 */

import { createMemo, createSignal, Show, type Component, type JSX } from "solid-js";

import { Modal, TabModalScope } from "@/element/modal";
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

import { TabModalContext, type TabModalApi, type TabModalRequest } from "./tab-modal";
import "./tab-modal.scss";

interface TabModalLayerProps {
    children: JSX.Element;
}

export const TabModalLayer: Component<TabModalLayerProps> = (props) => {
    const [current, setCurrent] = createSignal<TabModalRequest | null>(null);
    const [submitting, setSubmitting] = createSignal(false);

    // The mount node `<Modal scope="tab">` portals into. It also wraps
    // the tab's tile layout — so the unified Modal's scope-relative
    // `inert` (spec §5) inerts the tile layout (a non-`.modal-root`
    // sibling) while leaving the modal panel live. Held in a signal so
    // the `TabModalScope` accessor resolves lazily once the ref lands.
    const [mountEl, setMountEl] = createSignal<HTMLElement | null>(null);

    // Guard close: ESC (routed via the unified Modal's `onClose`) and a
    // would-be backdrop dismiss are blocked while a submit RPC is
    // in-flight so the user can't lose error feedback or trigger a
    // duplicate launch. `closeOnBackdropClick={false}` already prevents
    // backdrop dismissal outright; this additionally gates ESC.
    const safeClose = () => {
        if (!submitting()) setCurrent(null);
    };

    const api: TabModalApi = {
        open: (req) => { setSubmitting(false); setCurrent(req); },
        replace: (next) => {
            // Identical to `open` at the signal level — `replace` is a
            // continuation of the same modal session. The unified
            // <Modal> stays mounted across the swap (its `open` prop
            // stays true), and the keyed inner <Show> remounts only the
            // panel content, firing the content-fade keyframe.
            setSubmitting(false);
            setCurrent(next);
        },
        close: safeClose,
        current,
    };

    // Accessible label for the dialog. Derived from the request kind
    // alone — kept separate from `renderRequest` so reading it does NOT
    // build (and thus side-effect) a throwaway panel.
    const modalLabel = createMemo(() => {
        const req = current();
        return req ? requestLabel(req) : undefined;
    });

    return (
        <TabModalContext.Provider value={api}>
            <TabModalScope.Provider value={mountEl}>
                {/* Real mount node: wraps the tile layout AND hosts the
                    portalled <Modal>. `display:contents` keeps it
                    layout-transparent so TileLayout still sees
                    TabContent's flex container as its parent. */}
                <div class="tab-modal-mount" style="display:contents" ref={setMountEl}>
                    {props.children}
                    <Modal
                        open={current() != null}
                        scope="tab"
                        onClose={safeClose}
                        closeOnBackdropClick={false}
                        size="fit"
                        ariaLabel={modalLabel()}
                    >
                        {/* Keyed on the request identity so each
                            `replace` remounts the panel subtree — the
                            content-fade animation fires fresh for the
                            new content while the <Modal> shell (backdrop
                            + panel chrome) stays put. `renderRequest` is
                            called only inside this keyed scope so the
                            panel — and its on-create side effects
                            (reducer dispatch, store creation) — is built
                            exactly once per request, not on every
                            unrelated `current()` read. */}
                        <Show keyed when={current()}>
                            {(req) => (
                                <div class="tab-modal-content">
                                    {renderRequest(req, api, setSubmitting).panel}
                                </div>
                            )}
                        </Show>
                    </Modal>
                </div>
            </TabModalScope.Provider>
        </TabModalContext.Provider>
    );
};

// ── Render dispatch ──────────────────────────────────────────────────────────

/**
 * Accessible label for a request — pure, side-effect-free. Split out so
 * `modalLabel` can read it without building a panel. `renderRequest`
 * reuses it so the two stay in sync.
 */
function requestLabel(req: TabModalRequest): string {
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

function renderRequest(
    req: TabModalRequest,
    api: TabModalApi,
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
                                // Caller's onCreated does tabModal.replace
                                // back to Launch with the new id
                                // preselected — that's what unmounts this
                                // panel. We don't `api.close()` here.
                                req.onCreated(bundle.id, bundle.name);
                            } catch (e) {
                                setSubmitting(false);
                                throw e;
                            }
                        }}
                        // Caller's onCancel does tabModal.replace back to
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
                            // to call `tabModal.replace(launchReq)`
                            // (continueToLaunch=true) or `tabModal.close()`
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
                        onSubmit={async ({ name, identityId, memoryId }) => {
                            setSubmitting(true);
                            try {
                                const resp = await RpcApi.AgentDefCreateFromTemplateCommand(
                                    TabRpcClient,
                                    {
                                        template_id: req.template.id,
                                        name,
                                        identity_id: identityId,
                                        memory_id: memoryId,
                                    },
                                );
                                await req.onCreatedAndLaunch(
                                    resp.definition_id,
                                    resp.identity_id,
                                    resp.memory_id,
                                    name,
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
