// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ModalLayer — scope-parameterized modal host.
 *
 * Single dispatcher used by both tab-scoped and pane-scoped modal
 * hosts. Same request union, same imperative API (`open` / `replace`
 * / `close` / `current`), same render-dispatch table — the only knob
 * is the `scope` prop, which selects:
 *  - which `<Modal>` scope is used (`scope="tab"` vs `scope="pane"`),
 *  - which scope-mount context Provider is set (TabModalScope vs
 *    PaneModalScope) so the inner `<Modal>` resolves its mount node
 *    via the same scope axis.
 *
 * Mount strategy (unchanged from the original TabModalLayer):
 *  1. A real DOM mount node (`.modal-layer-mount`) wraps `props.children`
 *     and hosts the portalled `<Modal>` as a sibling.
 *  2. The Scope.Provider exposes that mount node as an accessor so the
 *     descendant `<Modal>` can resolve its mount via `useContext`.
 *  3. The unified `<Modal>` owns backdrop, panel chrome, ESC, focus
 *     trap, scope-relative `inert` + scroll lock, and the pane-overlay
 *     clip — this layer only handles dispatch.
 *
 * Use:
 *  - `<ModalLayer scope="tab">{props.children}</ModalLayer>` wraps a
 *    tab's tile layout in `frontend/app/tab/tabcontent.tsx`.
 *  - `<ModalLayer scope="pane">{props.children}</ModalLayer>` wraps a
 *    pane's content in `frontend/app/view/agent/agent-view.tsx` (and
 *    any other pane that wants pane-scoped modals).
 *
 * Inner components call `useModalLayer()` (from `./modal-layer`) and
 * never care which scope they're inside — pane wins over tab via
 * normal context resolution when both layers wrap the call site.
 *
 * History: this file is the descendant of `TabModalLayer` (per
 * `docs/specs/launch-modal-rearchitecture-2026-05-01.md`,
 * `SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md` §3/§5/§7/§9/§11). Lifted
 * out of `tab/` and parameterized over scope for the launch-modal
 * pane-scope work (`SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md`).
 *
 * Dismissal: `closeOnBackdropClick={false}` keeps the no-backdrop-
 * dismiss behaviour — a backdrop click nudges the panel's
 * `[data-modal-dismiss]` Cancel/Close control instead of closing.
 * ESC routes through `safeClose`, which no-ops while a submit RPC
 * is in-flight so the user can't lose error feedback or trigger a
 * duplicate launch.
 */

import { createMemo, createSignal, onCleanup, Show, type Component, type JSX } from "solid-js";

import { Modal, PaneModalScope, TabModalScope } from "@/element/modal";
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

import { ModalLayerContext, type ModalLayerApi, type ModalLayerRequest } from "./modal-layer";
import "./modal-layer.scss";

/** Pane-width threshold (CSS px) below which the compact-modal chrome
 *  fires. Picked so multi-pane layouts (browser pane at 240px in a
 *  three-pane window verified live during the auth-modal investigation)
 *  trigger the variant, while a single-pane window (e.g. a comfortable
 *  600+px agent pane) keeps the standard layout. Above this width
 *  every existing modal panel renders cleanly without horizontal
 *  scroll. Spec: SPEC_MODAL_COMPACT_VARIANT_2026_05_25.md §1. */
const COMPACT_THRESHOLD_PX = 400;

interface ModalLayerProps {
    /** Which scope's lock + mount this layer provides. `"tab"` covers
     *  the surrounding tab's content (legacy behaviour); `"pane"`
     *  covers a single pane only. The `<Modal>` rendered inside uses
     *  this same scope. */
    scope: "tab" | "pane";
    children: JSX.Element;
}

export const ModalLayer: Component<ModalLayerProps> = (props) => {
    const [current, setCurrent] = createSignal<ModalLayerRequest | null>(null);
    const [submitting, setSubmitting] = createSignal(false);

    // The mount node `<Modal>` portals into. It also wraps the layer's
    // children — so the unified Modal's scope-relative `inert` (spec §5
    // of SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21) inerts the children (a
    // non-`.modal-root` sibling) while leaving the modal panel live.
    // Held in a signal so the Scope.Provider accessor resolves lazily
    // once the ref lands.
    const [mountEl, setMountEl] = createSignal<HTMLElement | null>(null);

    // Compact-modal trigger — a ResizeObserver watches the mount node
    // and toggles a class when the lock region is narrower than
    // COMPACT_THRESHOLD_PX. CSS in `modal.scss` keys off
    // `.modal-layer-mount--compact` to shrink panel chrome (smaller
    // padding, smaller title, footer stacks vertically) so dialogs
    // remain usable in narrow panes (verified at 240px in a multi-
    // pane window). Spec: SPEC_MODAL_COMPACT_VARIANT_2026_05_25.md.
    const [isCompact, setIsCompact] = createSignal(false);
    let resizeObserver: ResizeObserver | null = null;

    const attachMountRef = (el: HTMLElement | null) => {
        setMountEl(el);
        // Disconnect any prior observer (handles re-mount in HMR /
        // hot-reload) before attaching a new one.
        if (resizeObserver) {
            resizeObserver.disconnect();
            resizeObserver = null;
        }
        if (!el) return;
        // `display:contents` on the mount node has no layout box of
        // its own, but `ResizeObserver` reports the content rect of
        // the observed element's children. In practice the mount
        // wraps the pane root, so `contentRect.width` equals the
        // pane width. (codex-anticipated edge case: observe lands
        // ~one frame post-mount; isCompact() stays false until then.
        // First-frame standard-variant flash is acceptable — the
        // compact variant is an additive layout adjustment, not a
        // correctness gate.)
        resizeObserver = new ResizeObserver((entries) => {
            for (const entry of entries) {
                const w = entry.contentRect.width;
                const compact = w > 0 && w < COMPACT_THRESHOLD_PX;
                if (compact !== isCompact()) setIsCompact(compact);
            }
        });
        resizeObserver.observe(el);
    };
    onCleanup(() => {
        if (resizeObserver) {
            resizeObserver.disconnect();
            resizeObserver = null;
        }
    });

    // Guard close: ESC (routed via the unified Modal's `onClose`) and a
    // would-be backdrop dismiss are blocked while a submit RPC is
    // in-flight so the user can't lose error feedback or trigger a
    // duplicate launch. `closeOnBackdropClick={false}` already prevents
    // backdrop dismissal outright; this additionally gates ESC.
    const safeClose = () => {
        if (!submitting()) setCurrent(null);
    };

    const api: ModalLayerApi = {
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

    // Scope.Provider selection: the inner `<Modal>` reads the same
    // scope-mount context that this layer publishes. `<ModalLayer
    // scope="pane">` publishes `PaneModalScope`; `scope="tab"`
    // publishes `TabModalScope`. The `<Modal scope={props.scope}>`
    // below then looks up via the matching context — they always agree
    // because they're parameterized over the same prop.
    const ScopeProvider = props.scope === "pane" ? PaneModalScope.Provider : TabModalScope.Provider;

    return (
        <ModalLayerContext.Provider value={api}>
            <ScopeProvider value={mountEl}>
                {/* Real mount node: wraps the layer's children AND hosts
                    the portalled <Modal>. `display:contents` keeps it
                    layout-transparent so callers' flex / grid containers
                    see their original parent. */}
                <div
                    class={`modal-layer-mount${isCompact() ? " modal-layer-mount--compact" : ""}`}
                    style="display:contents"
                    ref={attachMountRef}
                >
                    {props.children}
                    <Modal
                        open={current() != null}
                        scope={props.scope}
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
                                <div class="modal-layer-content">
                                    {renderRequest(req, api, setSubmitting).panel}
                                </div>
                            )}
                        </Show>
                    </Modal>
                </div>
            </ScopeProvider>
        </ModalLayerContext.Provider>
    );
};

// ── Render dispatch ──────────────────────────────────────────────────────────

/**
 * Accessible label for a request — pure, side-effect-free. Split out so
 * `modalLabel` can read it without building a panel. `renderRequest`
 * reuses it so the two stay in sync.
 */
function requestLabel(req: ModalLayerRequest): string {
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
