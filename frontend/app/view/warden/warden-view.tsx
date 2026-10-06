// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { onCleanup, onMount, type JSX } from "solid-js";

import { TabbedPane, type TabItem } from "@/app/element/ui";
import { WardenHostManager } from "@/app/view/warden-host/warden-host-manager";
import { WardenLanManager } from "@/app/view/warden-lan/warden-lan-manager";
import { WardenInternetStub } from "@/app/view/warden-internet/warden-internet-stub";
import { WardenAuditManager } from "@/app/view/warden-audit/warden-audit-manager";
import { WardenSupervisorManager } from "@/app/view/warden-supervisor/warden-supervisor-manager";
import { WARDEN_SECTION_LABELS, type WardenSection, type WardenViewModel } from "./warden-model";
import "./warden-view.scss";

const RAIL: TabItem<WardenSection>[] = [
    { id: "host",       label: WARDEN_SECTION_LABELS.host,       icon: "server" },
    { id: "lan",        label: WARDEN_SECTION_LABELS.lan,        icon: "network-wired" },
    { id: "internet",   label: WARDEN_SECTION_LABELS.internet,   icon: "globe" },
    { id: "audit",      label: WARDEN_SECTION_LABELS.audit,      icon: "list-check" },
    { id: "supervisor", label: WARDEN_SECTION_LABELS.supervisor, icon: "user-shield" },
];

export function WardenView(props: { model: WardenViewModel }): JSX.Element {
    const model = props.model;
    // Meta-backed on the model (warden-model.ts's sectionAtom) rather than a
    // local createSignal — so WardenViewModel.viewName can react to it, and
    // the selected tab survives a block remount. Mirrors section-pane.tsx.
    const section = model.sectionAtom;
    const setSection = (id: WardenSection) =>
        model.setMeta({ "warden:section": id });
    let viewRef: HTMLDivElement | undefined;

    // Ctrl+Wheel zoom — identical pipeline to the section panes' (section-pane.tsx is
    // the direct precedent for this exact capture-phase-listener + CSS-zoom-
    // on-root shape).
    onMount(() => {
        if (!viewRef) return;
        const handleCtrlWheel = (ev: WheelEvent) => {
            // Ctrl+Shift+Scroll is AppAllPanesZoomHandler's all-panes gesture
            // (app.tsx) — let it bubble there instead of zooming just this
            // pane. See SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md.
            if (!ev.ctrlKey || ev.shiftKey) return;
            ev.preventDefault();
            ev.stopPropagation();
            const STEP = 0.1;
            const current = model.zoomAtom();
            const next = Math.max(0.5, Math.min(2.0, Math.round((current + (ev.deltaY > 0 ? -STEP : STEP)) * 100) / 100));
            model.setMeta({ "term:zoom": next === 1.0 ? null : next });
        };
        viewRef.addEventListener("wheel", handleCtrlWheel, { passive: false, capture: true });
        onCleanup(() => viewRef?.removeEventListener("wheel", handleCtrlWheel, { capture: true }));
    });

    return (
        // warden-container carries container-type so the managers inside can
        // answer @container warden queries. A
        // container element cannot respond to its own container query.
        <div class="warden-container">
            <div class="warden-view" ref={viewRef} style={{ zoom: model.zoomAtom() }}>
                {/* One tablist: a rail, an icon-only rail, or tabs along the top,
                    by width (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5.4). */}
                <TabbedPane items={RAIL} value={section()} onChange={setSection} ariaLabel="Warden section" panelClass="bundle-manager-section">
                    {/*
                     * All five sections stay mounted — toggling is instant
                     * and never re-fetches. Host/LAN/Audit each own their
                     * own 5s poll loop, unaffected by visibility.
                     */}
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "host" }}>
                        <WardenHostManager />
                    </div>
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "lan" }}>
                        <WardenLanManager />
                    </div>
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "internet" }}>
                        <WardenInternetStub />
                    </div>
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "audit" }}>
                        <WardenAuditManager />
                    </div>
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "supervisor" }}>
                        <WardenSupervisorManager />
                    </div>
                </TabbedPane>
            </div>
        </div>
    );
}

WardenView.displayName = "WardenView";
