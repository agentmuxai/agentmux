// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ModalLayer — scope-parameterized modal host component.
 *
 * Wraps children in a `.modal-layer-mount` container that hosts the portalled
 * `<Modal>`. The `scope` prop selects which `<Modal>` scope and Scope.Provider
 * are used; pane-scope layers override tab-scope ones via context resolution.
 * ESC is blocked while a submit RPC is in-flight (`safeClose` guard).
 */

import { createMemo, createSignal, Show, type Component, type JSX } from "solid-js";

import { Modal, PaneModalScope, TabModalScope } from "@/element/modal";
import { initialFocusTarget } from "./modal-focus-trap";
import { ModalLayerContext, type ModalLayerApi, type ModalLayerRequest } from "./modal-layer";
import { renderRequest, requestLabel } from "./modal-dispatch";
import "./modal-layer.scss";

// Compact-variant threshold lives in CSS as a `@container modal-mount
// (max-width: 400px)` query — see `modal.scss`. The mount div below
// declares the container with `container-type: inline-size`, so the
// browser drives compact behavior continuously as the mount rect
// changes. No JS observer, no class toggle, no near-threshold
// flicker. Phase 2 of MODAL_COMPACT_VARIANT_ARCHITECTURE_2026_05_26.

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

    // Guard close: ESC (routed via the unified Modal's `onClose`) and a
    // would-be backdrop dismiss are blocked while a submit RPC is
    // in-flight so the user can't lose error feedback or submit twice.
    // `closeOnBackdropClick={false}` already prevents backdrop dismissal
    // outright; this additionally gates ESC. No current kind sets
    // `submitting` (the launch / add-account / new-bundle modals that did
    // are gone); kept for the next form-like kind.
    const safeClose = () => {
        if (!submitting()) setCurrent(null);
    };

    // Set by `replace` so the next content mount also moves focus —
    // `<Modal>` only resolves initial focus when it opens, and a replace
    // keeps it open.
    let replacing = false;

    // Every content mount starts at the top of the panel. `.modal-panel`
    // is the scroll container and survives a `replace`, so without this
    // the new content inherits the old content's scroll position — e.g.
    // a long modal scrolled to its bottom handed the next one an
    // already-scrolled panel
    // (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §1.1 item 2, §4.3 rule 4).
    const onContentMount = (el: HTMLDivElement) => {
        const moveFocus = replacing;
        replacing = false;
        requestAnimationFrame(() => {
            const panel = el.closest<HTMLElement>(".modal-panel");
            if (!panel) return;
            panel.scrollTop = 0;
            if (moveFocus) initialFocusTarget(panel)?.focus();
        });
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
            replacing = current() != null;
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
                    the portalled <Modal>. `container-type: inline-size`
                    (declared in modal.scss) lets CSS `@container modal-
                    mount (max-width: 400px)` queries drive the compact
                    variant continuously — no JS class toggle, no
                    near-threshold flicker. The element generates a
                    real box (drops the previous `display: contents`)
                    because container queries require a principal box;
                    the box is sized to fill its parent (`width: 100%;
                    height: 100%; position: relative`) so it stays
                    visually transparent to the surrounding pane
                    layout. Phase 2 of MODAL_COMPACT_VARIANT_
                    ARCHITECTURE_2026_05_26. */}
                <div
                    class="modal-layer-mount"
                    ref={setMountEl}
                >
                    {props.children}
                    <Modal
                        open={current() != null}
                        scope={props.scope}
                        onClose={safeClose}
                        closeOnBackdropClick={false}
                        size="fit"
                        // Per-kind hook for panel-level layout, e.g. the
                        // install dialog's fixed header and footer.
                        panelClass={current() ? `modal-panel--${current()!.kind}` : undefined}
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
                                <div class="modal-layer-content" ref={onContentMount}>
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

