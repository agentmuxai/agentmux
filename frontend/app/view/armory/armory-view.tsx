// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { For, onCleanup, onMount, type JSX } from "solid-js";

import { Tooltip } from "@/app/element/tooltip";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { MemoryManager } from "@/app/view/memory/memory-manager";
import { AccountsManager } from "@/app/view/accounts/accounts-manager";
import { GlobalBrainManager } from "@/app/view/brain/global-brain-manager";
import { McpManager } from "@/app/view/mcp/mcp-manager";
import { SkillManager } from "@/app/view/skill/skill-manager";
import { NativeMemoryManager } from "@/app/view/native-memory/native-memory-manager";
import { ARMORY_SECTION_LABELS, type ArmorySection, type ArmoryViewModel } from "./armory-model";
import "./armory-view.scss";

// Global Memory + Personal Memory sit together right below Accounts,
// deliberately adjacent — the user-facing framing is one "Memory" concept
// split in two by scope (workspace-wide vs. per-agent), not two unrelated
// features that happen to share a rail. See
// docs/specs/SPEC_ARMORY_MEMORY_GLOBAL_PERSONAL_RENAME_2026_08_22.md.
const RAIL: { id: ArmorySection; label: string; tooltip?: string; icon: string }[] = [
    { id: "accounts", label: ARMORY_SECTION_LABELS.accounts, icon: "key" },
    { id: "memory",   label: ARMORY_SECTION_LABELS.memory,   tooltip: "Workspace-wide — composes into every agent's CLAUDE.md", icon: "brain" },
    { id: "native_memory", label: ARMORY_SECTION_LABELS.native_memory, tooltip: "Per-agent — version history, diff and revert", icon: "clock-rotate-left" },
    { id: "skills",   label: ARMORY_SECTION_LABELS.skills,   icon: "wand-magic-sparkles" },
    { id: "mcp",      label: ARMORY_SECTION_LABELS.mcp,      icon: "plug" },
    { id: "bundles",  label: ARMORY_SECTION_LABELS.bundles,  tooltip: "Armory Bundle Format (ABF)", icon: "layer-group" },
];

export function ArmoryView(props: ViewComponentProps<ArmoryViewModel>): JSX.Element {
    const model = props.model;
    // Meta-backed on the model (armory-model.ts's sectionAtom) rather than a
    // local createSignal — so ArmoryViewModel.viewName can react to it, and
    // the selected tab survives a block remount.
    const section = model.sectionAtom;
    const setSection = (id: ArmorySection) =>
        void RpcApi.SetMetaCommand(TabRpcClient, {
            oref: `block:${model.blockId}`,
            meta: { "armory:section": id },
        });
    let viewRef: HTMLDivElement | undefined;

    // Ctrl+Wheel zoom — same term:zoom-on-block-meta pipeline as editor/term/
    // agent/swarm (editor-view.tsx is the direct precedent for this exact
    // capture-phase-listener + CSS-zoom-on-root shape). Capture phase so we
    // intercept before any scrollable content inside Armory (rail/section
    // panes use plain overflow:auto, no wheel handling of their own to
    // conflict with); preventDefault suppresses CEF's native Ctrl+Scroll
    // page zoom the same way it already does for every other pane type.
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
        // armory-container carries container-type so that .armory-view
        // (a descendant) can be targeted by @container armory queries.
        // A container element cannot respond to its own container query.
        <div class="armory-container">
            <div class="armory-view" ref={viewRef} style={{ zoom: model.zoomAtom() }}>
                <nav class="bundle-manager-rail" aria-label="Armory section">
                    <For each={RAIL}>
                        {(item) => (
                            <Tooltip content={item.tooltip ?? item.label} placement="right">
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
                     * All six managers stay mounted — toggling is instant and
                     * never re-fetches. All stay consistent via WPS *:changed events.
                     */}
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "accounts" }}>
                        <AccountsManager />
                    </div>
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "memory" }}>
                        <GlobalBrainManager />
                    </div>
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "skills" }}>
                        <SkillManager />
                    </div>
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "mcp" }}>
                        <McpManager />
                    </div>
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "bundles" }}>
                        <MemoryManager />
                    </div>
                    <div class="bundle-manager-pane" classList={{ "is-hidden": section() !== "native_memory" }}>
                        <NativeMemoryManager />
                    </div>
                </div>
                <nav class="bundle-manager-tab-bar" aria-label="Armory section">
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
