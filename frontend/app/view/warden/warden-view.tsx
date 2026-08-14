// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, onCleanup, onMount, type JSX } from "solid-js";

import { Tooltip } from "@/app/element/tooltip";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { WardenHostManager } from "@/app/view/warden-host/warden-host-manager";
import { WardenLanManager } from "@/app/view/warden-lan/warden-lan-manager";
import { WardenInternetStub } from "@/app/view/warden-internet/warden-internet-stub";
import { WardenAuditManager } from "@/app/view/warden-audit/warden-audit-manager";
import { WardenSupervisorManager } from "@/app/view/warden-supervisor/warden-supervisor-manager";
import { WARDEN_SECTION_LABELS, type WardenSection, type WardenViewModel } from "./warden-model";
import "./warden-view.scss";

const RAIL: { id: WardenSection; label: string; icon: string }[] = [
    { id: "host",       label: WARDEN_SECTION_LABELS.host,       icon: "server" },
    { id: "lan",        label: WARDEN_SECTION_LABELS.lan,        icon: "network-wired" },
    { id: "internet",   label: WARDEN_SECTION_LABELS.internet,   icon: "globe" },
    { id: "audit",      label: WARDEN_SECTION_LABELS.audit,      icon: "list-check" },
    { id: "supervisor", label: WARDEN_SECTION_LABELS.supervisor, icon: "user-shield" },
];

export function WardenView(props: ViewComponentProps<WardenViewModel>): JSX.Element {
    const model = props.model;
    // Meta-backed on the model (warden-model.ts's sectionAtom) rather than a
    // local createSignal — so WardenViewModel.viewName can react to it, and
    // the selected tab survives a block remount. Mirrors armory-view.tsx.
    const section = model.sectionAtom;
    const setSection = (id: WardenSection) =>
        void RpcApi.SetMetaCommand(TabRpcClient, {
            oref: `block:${model.blockId}`,
            meta: { "warden:section": id },
        });
    let viewRef: HTMLDivElement | undefined;

    // Ctrl+Wheel zoom — identical pipeline to Armory's (armory-view.tsx is
    // the direct precedent for this exact capture-phase-listener + CSS-zoom-
    // on-root shape).
    onMount(() => {
        if (!viewRef) return;
        const handleCtrlWheel = (ev: WheelEvent) => {
            if (!ev.ctrlKey) return;
            ev.preventDefault();
            ev.stopPropagation();
            const STEP = 0.1;
            const current = model.zoomAtom();
            const next = Math.max(0.5, Math.min(2.0, Math.round((current + (ev.deltaY > 0 ? -STEP : STEP)) * 100) / 100));
            void RpcApi.SetMetaCommand(TabRpcClient, {
                oref: `block:${model.blockId}`,
                meta: { "term:zoom": next === 1.0 ? null : next },
            });
        };
        viewRef.addEventListener("wheel", handleCtrlWheel, { passive: false, capture: true });
        onCleanup(() => viewRef?.removeEventListener("wheel", handleCtrlWheel, { capture: true }));
    });

    return (
        // warden-container carries container-type so that .warden-view (a
        // descendant) can be targeted by @container warden queries. A
        // container element cannot respond to its own container query.
        <div class="warden-container">
            <div class="warden-view" ref={viewRef} style={{ zoom: model.zoomAtom() }}>
                <nav class="bundle-manager-rail" aria-label="Warden section">
                    <For each={RAIL}>
                        {(item) => (
                            <Tooltip content={item.label} placement="right">
                                <button
                                    type="button"
                                    class="bundle-manager-rail-item"
                                    classList={{ "is-active": section() === item.id }}
                                    aria-pressed={section() === item.id}
                                    onClick={() => setSection(item.id)}
                                >
                                    <i class={`fa-sharp fa-solid fa-${item.icon}`} aria-hidden="true" />
                                    <span>{item.label}</span>
                                </button>
                            </Tooltip>
                        )}
                    </For>
                </nav>
                <div class="bundle-manager-section">
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
                </div>
                <nav class="bundle-manager-tab-bar" aria-label="Warden section">
                    <For each={RAIL}>
                        {(item) => (
                            <button
                                type="button"
                                classList={{ "is-active": section() === item.id }}
                                aria-pressed={section() === item.id}
                                onClick={() => setSection(item.id)}
                            >
                                <i class={`fa-sharp fa-solid fa-${item.icon}`} aria-hidden="true" />
                                <span>{item.label}</span>
                            </button>
                        )}
                    </For>
                </nav>
            </div>
        </div>
    );
}

WardenView.displayName = "WardenView";
