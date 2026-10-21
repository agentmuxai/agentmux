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
import { VaultView } from "@/app/view/vault/vault-view";
import { WARDEN_SECTIONS, type WardenSection, type WardenViewModel } from "./warden-model";
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
    const [vaultSeen, setVaultSeen] = createSignal(false);
    createEffect(() => {
        if (section() === "vault") setVaultSeen(true);
    });

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
                     * All sections stay mounted — toggling is instant and
                     * never re-fetches. Host/LAN/Audit each own their own 5s
                     * poll loop, unaffected by visibility; History loads when
                     * first shown and on Refresh.
                     */}
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "kanban" }}>
                        <WardenKanbanManager
                            visible={section() === "kanban"}
                            onNeedsCount={model.setKanbanNeeds}
                            openRequest={model.kanbanOpenAtom()}
                            onOpenHandled={() => model.setMeta({ "kanban:open": null })}
                            lanes={model.kanbanLanesAtom()}
                            onLanes={(on) => model.setMeta({ "kanban:lanes": on })}
                        />
                    </div>
                    {/* Mounted the first time it is shown: listing the Vault decrypts its entries, which
                        can make the OS ask for the Vault key, so a Warden opened on another section
                        never does. Shown, for its protections, only while selected (vault-in-warden.ts). */}
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "vault" }}>
                        <Show when={vaultSeen()}>
                            <VaultView model={model.vault} />
                        </Show>
                    </div>
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
                        <WardenAuditManager visible={section() === "audit"} />
                    </div>
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "history" }}>
                        <WardenHistoryManager wardenBlockId={model.blockId} visible={section() === "history"} />
                    </div>
                </TabbedPane>
            </div>
        </div>
    );
}

WardenView.displayName = "WardenView";
