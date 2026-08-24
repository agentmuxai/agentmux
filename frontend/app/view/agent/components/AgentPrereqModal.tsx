// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPrereqModalPanel — pre-launch modal listing missing system
 * tools the provider's CLI needs at runtime. Opens when any of the
 * provider's `systemPrereqs` aren't on PATH.
 *
 * SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md.
 */

import { createSignal, For, Show, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { getApi } from "@/app/store/global";
import { SystemToolInstallInline } from "@/app/view/toolchain/SystemToolInstallInline";

interface MissingPrereq {
    tool: string;
    label: string;
    installUrl: string;
    installLinkText: string;
}

// Tool ids the backend's system-install catalog covers
// (system_install_handlers.rs) — anything else keeps the existing
// link-only row unchanged. See
// docs/specs/SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md.
const SYSTEM_INSTALLABLE_IDS = new Set(["git", "node", "npm", "python"]);

interface AgentPrereqModalPanelProps {
    agent: AgentDefinition;
    missing: MissingPrereq[];
    onRefresh: () => void;
    onProceed: () => void;
    onCancel: () => void;
}

export const AgentPrereqModalPanel = (props: AgentPrereqModalPanelProps): JSX.Element => {
    const open = (url: string): void => {
        try {
            getApi().openExternal(url);
        } catch {
            // Fall through — if external-open fails, the link is at
            // least visible in the modal as text the user can copy.
        }
    };

    const [expandedInstalls, setExpandedInstalls] = createSignal<ReadonlySet<string>>(new Set());
    const toggleInstallPanel = (tool: string) => {
        setExpandedInstalls((prev) => {
            const next = new Set(prev);
            if (next.has(tool)) next.delete(tool); else next.add(tool);
            return next;
        });
    };

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">
                    Install required tools to use {props.agent.name}
                </h2>
                <p class="modal-panel-description">
                    {props.agent.name} depends on tools that aren't on your
                    PATH. Install them, then click <em>Refresh</em>.
                </p>
            </header>
            <div class="modal-panel-body agent-prereq-modal-body">
                <ul class="agent-prereq-modal-list">
                    <For each={props.missing}>
                        {(req) => (
                            <li class="agent-prereq-modal-row">
                                <span class="agent-prereq-modal-icon" aria-hidden="true">⚠</span>
                                <div class="agent-prereq-modal-info">
                                    <div class="agent-prereq-modal-tool">
                                        {req.label}
                                        <span class="agent-prereq-modal-tool-status"> — not found</span>
                                    </div>
                                    <a
                                        class="agent-prereq-modal-link"
                                        href={req.installUrl}
                                        onClick={(e) => {
                                            e.preventDefault();
                                            open(req.installUrl);
                                        }}
                                    >
                                        {req.installLinkText}
                                        <span class="agent-prereq-modal-link-arrow" aria-hidden="true"> ↗</span>
                                    </a>
                                    <Show when={SYSTEM_INSTALLABLE_IDS.has(req.tool)}>
                                        <button
                                            type="button"
                                            class="agent-prereq-modal-link"
                                            onClick={() => toggleInstallPanel(req.tool)}
                                        >
                                            or install it now ↓
                                        </button>
                                        <Show when={expandedInstalls().has(req.tool)}>
                                            <SystemToolInstallInline
                                                toolId={req.tool}
                                                onInstalled={() => {
                                                    toggleInstallPanel(req.tool);
                                                    props.onRefresh();
                                                }}
                                            />
                                        </Show>
                                    </Show>
                                </div>
                            </li>
                        )}
                    </For>
                </ul>
                <p class="agent-prereq-modal-hint">
                    Already installed under a non-standard path? Click <em>Launch
                    anyway</em> — AgentMux can't see it from <code>PATH</code> but
                    your CLI might pick it up.
                </p>
            </div>
            <footer class="modal-panel-footer">
                <Button onClick={() => props.onCancel()} data-modal-dismiss>Cancel</Button>
                <Button onClick={() => props.onRefresh()}>Refresh</Button>
                <Button onClick={() => props.onProceed()} className="green solid">
                    Launch anyway
                </Button>
            </footer>
        </>
    );
};

AgentPrereqModalPanel.displayName = "AgentPrereqModalPanel";
