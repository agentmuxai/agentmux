// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, Show, type JSX } from "solid-js";

import { TabbedPane, type TabItem } from "@/app/element/ui";
import { WardenHostManager } from "@/app/view/warden-host/warden-host-manager";
import { WardenLanManager } from "@/app/view/warden-lan/warden-lan-manager";
import { WardenInternetStub } from "@/app/view/warden-internet/warden-internet-stub";
import { WardenAuditManager } from "@/app/view/warden-audit/warden-audit-manager";
import { WardenHistoryManager } from "@/app/view/warden-history/warden-history-manager";
import { WardenKanbanManager } from "@/app/view/warden-kanban/warden-kanban-manager";
import { WardenAmbientManager } from "@/app/view/warden-ambient/warden-ambient-manager";
import { VaultView } from "@/app/view/vault/vault-view";
import { WARDEN_SECTIONS, type WardenSection, type WardenViewModel } from "./warden-model";
// The section panes use section-pane.scss's `.bundle-manager-section` / `-pane` rules (a pane that is not the
// selected section is `display: none` there). Connectors and Memory used to guarantee it was loaded, but
// their barrels no longer import section-pane.tsx, so without this import Warden shows every section at once.
import "@/app/view/section-pane/section-pane.scss";
import "./warden-view.scss";

const RAIL: TabItem<WardenSection>[] = [...WARDEN_SECTIONS.items];

export function WardenView(props: { model: WardenViewModel }): JSX.Element {
    const model = props.model;
    // Meta-backed on the model (warden-model.ts's sectionAtom) rather than a
    // local createSignal — so WardenViewModel.viewName can react to it, and
    // the selected tab survives a block remount. Mirrors section-pane.tsx.
    const section = model.sectionAtom;
    const setSection = (id: WardenSection) =>
        model.setMeta({ "warden:section": id });
    let viewRef: HTMLDivElement | undefined;
    // Sections shown so far. Each mounts the first time it is shown and then stays mounted, so
    // switching back is instant; a Warden opened from the Kanban or Vault button doesn't start the
    // Host, LAN and Audit polls, or read the Vault, until those are looked at.
    const [seen, setSeen] = createSignal<ReadonlySet<WardenSection>>(new Set([section()]));
    createEffect(() => {
        const s = section();
        if (!seen().has(s)) setSeen(new Set([...seen(), s]));
    });
    const shown = (id: WardenSection) => section() === id;
    const Lazy = (p: { id: WardenSection; children: JSX.Element }) => (
        <div class="bundle-manager-pane" classList={{ "is-hidden": !shown(p.id) }}>
            <Show when={seen().has(p.id)}>{p.children}</Show>
        </div>
    );

    // Ctrl+scroll and Ctrl/Cmd +/-/0 zoom this pane through the shared handlers
    // (app.tsx, keymodel.ts): the `paneZoom` capability is all it needs.

    return (
        // warden-container carries container-type so the managers inside can
        // answer @container warden queries. A
        // container element cannot respond to its own container query.
        <div class="warden-container">
            <div class="warden-view" ref={viewRef} style={{ zoom: model.zoomAtom() }}>
                {/* One tablist along the top: icons only when narrow, labels when
                    there's room (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5.4). */}
                <TabbedPane items={RAIL} value={section()} onChange={setSection} ariaLabel="Warden section" panelClass="bundle-manager-section">
                    {/*
                     * Each section mounts the first time it is shown, then stays mounted, so
                     * switching back is instant. Host, LAN, Audit and Ambient poll every 5s only
                     * while shown; Kanban and History load while shown. The Vault mounts on first
                     * show (listing it decrypts its entries, which can make the OS ask for the
                     * Vault key) and counts as shown, for its protections, only while selected
                     * (vault-in-warden.ts).
                     */}
                    <Lazy id="kanban">
                        <WardenKanbanManager
                            visible={shown("kanban")}
                            onNeedsCount={model.setKanbanNeeds}
                            openRequest={model.kanbanOpenAtom()}
                            onOpenHandled={() => model.setMeta({ "kanban:open": null })}
                            lanes={model.kanbanLanesAtom()}
                            onLanes={(on) => model.setMeta({ "kanban:lanes": on })}
                        />
                    </Lazy>
                    <Lazy id="vault">
                        <VaultView model={model.vault} />
                    </Lazy>
                    <Lazy id="host">
                        <WardenHostManager visible={shown("host")} />
                    </Lazy>
                    <Lazy id="lan">
                        <WardenLanManager visible={shown("lan")} />
                    </Lazy>
                    <Lazy id="internet">
                        <WardenInternetStub />
                    </Lazy>
                    <Lazy id="audit">
                        <WardenAuditManager visible={shown("audit")} />
                    </Lazy>
                    <Lazy id="history">
                        <WardenHistoryManager wardenBlockId={model.blockId} visible={shown("history")} />
                    </Lazy>
                    <Lazy id="ambient">
                        <WardenAmbientManager visible={shown("ambient")} onFailingCount={model.setAmbientFailing} />
                    </Lazy>
                </TabbedPane>
            </div>
        </div>
    );
}

WardenView.displayName = "WardenView";
